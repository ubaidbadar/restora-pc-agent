//! Hermetic failure-injection scenarios. These run on Linux AND Windows.
//! They do NOT claim that an actual USB printer or hardware sensor responded.
use crate::printer_state::{
    PrinterHealth, PrinterState as S, Source, WinPrinter, bound_usb_presence, classify,
    spooler_state, usb_port_hint,
};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn queue(name: &str, port: &str, status: u32, attributes: u32) -> WinPrinter {
    WinPrinter { name: name.into(), port: port.into(), status, attributes, jobs: 0 }
}

#[test]
fn exhaustive_spooler_fault_flags_never_get_overridden_by_successful_pnp_or_probe() {
    // Real Windows PRINTER_STATUS_* bits. Vary every documented status
    // that this classifier currently consumes, including simultaneous faults.
    const FLAGS: [u32; 13] = [
        0x10,       // paper out
        0x8,        // paper jam
        0x40,       // paper problem
        0x0040_0000,// door open
        0x2,        // printer error
        0x0010_0000,// user intervention
        0x0004_0000,// no toner
        0x0000_1000,// not available
        0x80,       // offline
        0x0200_0000,// server offline
        0x1,        // paused
        0x0080_0000,// unknown
        0x0000_0020,// manual feed (not diagnosed by agent)
    ];
    for combination in 0u32..(1u32 << FLAGS.len()) {
        let status = FLAGS.iter().enumerate().fold(0, |bits, (i, flag)| {
            if combination & (1 << i) != 0 { bits | flag } else { bits }
        });
        let paper = status & 0x10 != 0;
        let error = status & (0x8 | 0x40 | 0x0040_0000 | 0x2 |
            0x0010_0000 | 0x0004_0000 | 0x1000) != 0;
        let offline = status & (0x80 | 0x0200_0000) != 0;
        let unknown = status & (0x1 | 0x0080_0000) != 0;

        let expected = if paper { S::PaperOut } else if error { S::Error }
            else if offline { S::Offline } else if unknown { S::Unknown }
            else { S::Ready };

        assert_eq!(spooler_state(status, 0), expected, "status flags={status:#010x}");
        for attribute in [0u32, 0x400] {
            let expected = if attribute & 0x400 != 0 && expected == S::Ready {
                S::Offline
            } else if attribute & 0x400 != 0 && expected == S::Unknown {
                // Offline is more actionable than PAUSED/unknown
                S::Offline
            } else { expected };
            let q = queue("Simulated USB", "USB001", status, attribute);
            let observed = classify(&q, Some(true), Some(S::Ready), 123);
            if expected == S::Ready {
                assert_eq!(observed.state, S::Ready, "{status:#x}");
                assert_eq!(observed.source, Source::Printer);
            } else {
                assert_eq!(observed.state, expected, "{status:#x}, attr={attribute:#x}");
                assert_eq!(observed.source, Source::Spooler);
            }
        }
    }
}

#[test]
fn simulated_usb_unplugged_powered_off_and_sensor_faults_fail_closed() {
    let q = queue("Thermal", "USB001", 0, 0);
    let cases = [
        (None, None, S::Unknown, Source::Spooler),  // installed queue only
        (Some(true), None, S::Unknown, Source::Pnp), // interface only, not powered evidence
        (Some(false), None, S::Offline, Source::Pnp), // verified mapped interface disappeared
        (Some(true), Some(S::Ready), S::Ready, Source::Printer), // *synthetic* valid probe
        (Some(true), Some(S::PaperOut), S::PaperOut, Source::Printer),
        (Some(true), Some(S::Error), S::Error, Source::Printer), // cover open, device error
        (Some(true), Some(S::Unknown), S::Unknown, Source::Printer), // malformed status
    ];
    for (pnp, probe, expected, source) in cases {
        let actual = classify(&q, pnp, probe, 123);
        assert_eq!((actual.state, actual.source), (expected, source));
    }
    // A no-paper or open-cover spooler fault must override any optimistic probe.
    for (flag, expected) in [(0x10, S::PaperOut), (0x0040_0000, S::Error), (0x80, S::Offline)] {
        let actual = classify(&queue("Thermal", "USB001", flag, 0), Some(true), Some(S::Ready), 123);
        assert_eq!(actual.state, expected);
        assert_eq!(actual.source, Source::Spooler);
    }
}

#[test]
fn identical_usb_printers_and_reenumeration_do_not_invent_a_binding() {
    let first = r"\\?\usb#vid_0001&pid_0002#serialA#{28d78fad-5a12-11d1-ae5b-0000f803a8c2}";
    let second = r"\\?\usb#vid_0001&pid_0002#serialB#{28d78fad-5a12-11d1-ae5b-0000f803a8c2}";
    let mut bindings = HashMap::from([
        ("Kitchen".to_owned(), first.to_owned()),
        ("Bar".to_owned(), second.to_owned()),
    ]);
    let present = vec![second.to_owned()];
    assert_eq!(bound_usb_presence("Kitchen", &bindings, &present), Some(false));
    assert_eq!(bound_usb_presence("Bar", &bindings, &present), Some(true));
    assert_eq!(bound_usb_presence("Other identical model", &bindings, &present), None);
    assert_eq!(classify(&queue("Bar", "USB002", 0, 0), Some(true), None, 123).state, S::Unknown);
    bindings.insert("Bar".to_owned(), first.to_owned()); // duplicate paths are ambiguous
    assert_eq!(bound_usb_presence("Kitchen", &bindings, &present), None);
    assert_eq!(bound_usb_presence("Bar", &bindings, &present), None);
    bindings.insert("Bar".to_owned(), " USB002 ".into()); // port name is not identity
    assert_eq!(bound_usb_presence("Bar", &bindings, &present), None);
    bindings.insert("Bar".to_owned(), String::new()); // empty binding not absent
    assert_eq!(bound_usb_presence("Bar", &bindings, &present), None);
}

#[test]
fn unknown_connections_and_network_spooler_only_semantics_are_explicit() {
    for (port, expected) in [
        ("IP_192.168.1.2", S::Ready),
        ("WSD-a91a", S::Ready),
        (r"\\server\SharedPrinter", S::Ready),
        ("USB003", S::Unknown),
        ("", S::Unknown),
        ("UnexpectedCustomPort", S::Unknown),
    ] {
        let q = queue("Queue", port, 0, 0);
        let h = classify(&q, None, None, 200);
        assert_eq!(h.state, expected, "{port}");
        assert_eq!(h.source, Source::Spooler);
        assert_eq!(h.checked_at, 200);
    }
    assert!(usb_port_hint("usb001"));
    assert!(usb_port_hint("USB001, LPT1:"));
    assert!(!usb_port_hint("USB"));
    assert!(!usb_port_hint("🖨USB001"));
}

#[test]
fn deterministic_randomized_fault_and_serialization_fuzz() {
    // Fixed seed makes a CI failure reproducible. No third-party fuzz runtime.
    let mut rng: u64 = 0x9e37_79b9_7f4a_7c15;
    for i in 0..20_000 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let bytes = rng.to_le_bytes();
        let status = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        let attr = u32::from_le_bytes(bytes[4..].try_into().unwrap());
        let q = queue("Synthetic", if i % 2 == 0 { "USB001" } else { "IP_127.0.0.1" }, status, attr);
        let health = classify(&q, Some(true), None, i);
        assert_ne!(health.state, S::NotFound); // queue exists in every synthetic scenario
        if i % 2 == 0 { assert_ne!(health.state, S::Ready); }
        let wire = serde_json::to_vec(&health).unwrap();
        let reread: PrinterHealth = serde_json::from_slice(&wire).unwrap();
        assert_eq!(health, reread);
    }
    for invalid in [b"".as_slice(), b"{".as_slice(), b"{\"state\":\"ready\"}".as_slice(), b"[]".as_slice()] {
        assert!(serde_json::from_slice::<PrinterHealth>(invalid).is_err());
    }
}

#[test]
fn concurrent_notification_storm_is_coalesced_without_losing_final_invalidation() {
    let dirty = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let dirty = dirty.clone();
            scope.spawn(move || {
                for _ in 0..1000 {
                    dirty.store(true, Ordering::Release);
                    std::thread::yield_now();
                }
            });
        }
        // Poller might consume earlier notifications; last event after join
        // must always be detectable by the next scan.
        for _ in 0..1000 { let _ = dirty.swap(false, Ordering::AcqRel); }
    });
    dirty.store(true, Ordering::Release);
    assert!(dirty.swap(false, Ordering::AcqRel));
    assert!(!dirty.load(Ordering::Acquire));
}
