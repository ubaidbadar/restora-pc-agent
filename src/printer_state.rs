//! Pure, cross-platform classification. A Windows queue is not a physical-health check.
use serde::{Deserialize, Serialize};

/// Status sent to the local status page and, additively, to the backend.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrinterState {
    Ready,
    Offline,
    PaperOut,
    Error,
    NotFound,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Spooler,
    Pnp,
    Printer,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WinPrinter {
    pub name: String,
    pub port: String,
    pub status: u32,
    pub attributes: u32,
    pub jobs: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrinterHealth {
    pub system_name: String,
    pub port: String,
    pub state: PrinterState,
    pub source: Source,
    pub checked_at: u64,
}

/// All flags are raw u32 in windows 0.61's PRINTER_INFO_2W.
/// See PRINTER_INFO_2 (winspool.h); WORK_OFFLINE is an ATTRIBUTE, not STATUS.
pub fn spooler_state(status: u32, attributes: u32) -> PrinterState {
    const OFFLINE: u32 = 0x00000080;
    const SERVER_OFFLINE: u32 = 0x02000000;
    const WORK_OFFLINE: u32 = 0x00000400;
    const PAPER_OUT: u32 = 0x00000010;
    const PAPER_JAM: u32 = 0x00000008;
    const PAPER_PROBLEM: u32 = 0x00000040;
    const DOOR_OPEN: u32 = 0x00400000;
    const ERROR: u32 = 0x00000002;
    const NOT_AVAILABLE: u32 = 0x00001000;
    const USER_INTERVENTION: u32 = 0x00100000;
    const NO_TONER: u32 = 0x00040000;
    const PAUSED: u32 = 0x00000001;
    const SERVER_UNKNOWN: u32 = 0x00800000;
    if status & PAPER_OUT != 0 {
        PrinterState::PaperOut
    } else if status
        & (PAPER_JAM
            | PAPER_PROBLEM
            | DOOR_OPEN
            | ERROR
            | USER_INTERVENTION
            | NO_TONER
            | NOT_AVAILABLE)
        != 0
    {
        PrinterState::Error
    } else if attributes & WORK_OFFLINE != 0 || status & (OFFLINE | SERVER_OFFLINE) != 0 {
        PrinterState::Offline
    } else if status & (PAUSED | SERVER_UNKNOWN) != 0 {
        PrinterState::Unknown
    } else {
        PrinterState::Ready
    }
}

/// This is only a transport *hint*. Ports are not stable physical IDs.
/// A USB name must NEVER be joined to a PnP interface by enumeration order or VID:PID.
pub fn usb_port_hint(port: &str) -> bool {
    port.split(',').map(str::trim).any(|p| {
        p.get(..3)
            .is_some_and(|head| head.eq_ignore_ascii_case("USB"))
            && p.get(3..)
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    })
}

pub fn non_usb_port_known(port: &str) -> bool {
    !port.is_empty()
        && !usb_port_hint(port)
        && port.split(',').all(|p| {
            let p = p.trim();
            ["IP_", "WSD-", "COM", "LPT", "FILE:", "PORTPROMPT:", "NUL:"]
                .iter()
                .any(|prefix| p.to_ascii_uppercase().starts_with(prefix))
                || p.starts_with("\\\\")
        })
}

/// Resolve only an explicitly configured, one-to-one USBPRINT identity.
/// Multiple queues bound to the same device (or empty paths) are ambiguous;
/// that must NEVER be interpreted as a definitive physical absence.
pub fn bound_usb_presence(
    queue: &str,
    bindings: &std::collections::HashMap<String, String>,
    present_interfaces: &[String],
) -> Option<bool> {
    let path = bindings.get(queue)?.trim();
    if path.is_empty()
        || !path.starts_with(r"\\?\")
        || !path.to_ascii_lowercase().contains("usb#")
        || bindings
            .iter()
            .any(|(other, candidate)| other != queue && candidate.trim().eq_ignore_ascii_case(path))
    {
        return None;
    }
    Some(
        present_interfaces
            .iter()
            .any(|interface| interface.eq_ignore_ascii_case(path)),
    )
}

/// Fail closed for USB and for unclassified transport. PnP inventory alone is not enough.
/// A future verified, one-to-one queue binding may provide positive/negative PnP evidence;
/// *present* is still Unknown without real-time printer protocol evidence.
pub fn classify(
    printer: &WinPrinter,
    bound_usb_present: Option<bool>,
    probe: Option<PrinterState>,
    checked_at: u64,
) -> PrinterHealth {
    let spooler = spooler_state(printer.status, printer.attributes);
    let (state, source) = if spooler != PrinterState::Ready {
        (spooler, Source::Spooler)
    } else if usb_port_hint(&printer.port) {
        if let Some(false) = bound_usb_present {
            (PrinterState::Offline, Source::Pnp)
        } else if let Some(actual) = probe {
            (actual, Source::Printer)
        } else {
            (
                PrinterState::Unknown,
                if bound_usb_present.is_some() {
                    Source::Pnp
                } else {
                    Source::Spooler
                },
            )
        }
    } else if non_usb_port_known(&printer.port) {
        (PrinterState::Ready, Source::Spooler)
    } else {
        (PrinterState::Unknown, Source::Spooler)
    };
    PrinterHealth {
        system_name: printer.name.clone(),
        port: printer.port.clone(),
        state,
        source,
        checked_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printer(port: &str, status: u32, attributes: u32) -> WinPrinter {
        WinPrinter {
            name: "Kitchen".into(),
            port: port.into(),
            status,
            attributes,
            jobs: 0,
        }
    }

    #[test]
    fn printer_flags_are_precise_and_precedence_is_deterministic() {
        assert_eq!(spooler_state(0, 0), PrinterState::Ready);
        assert_eq!(spooler_state(0, 0x400), PrinterState::Offline);
        assert_eq!(spooler_state(0x80, 0), PrinterState::Offline);
        assert_eq!(spooler_state(0x0200_0000, 0), PrinterState::Offline);
        assert_eq!(spooler_state(0x10, 0), PrinterState::PaperOut);
        for flag in [0x8, 0x40, 0x0040_0000, 0x2, 0x0010_0000, 0x0004_0000] {
            assert_eq!(spooler_state(flag, 0), PrinterState::Error);
        }
        assert_eq!(spooler_state(0x1000, 0), PrinterState::Error);
        assert_eq!(spooler_state(1, 0), PrinterState::Unknown);
        assert_eq!(spooler_state(0x10 | 0x80, 0), PrinterState::PaperOut);
    }

    #[test]
    fn usb_queue_and_pnp_presence_do_not_imply_ready() {
        let p = printer("USB001", 0, 0);
        assert_eq!(classify(&p, None, None, 10).state, PrinterState::Unknown);
        assert_eq!(
            classify(&p, Some(true), None, 10).state,
            PrinterState::Unknown
        );
        assert_eq!(
            classify(&p, Some(false), None, 10).state,
            PrinterState::Offline
        );
        assert_eq!(
            classify(&p, Some(true), Some(PrinterState::Ready), 10).state,
            PrinterState::Ready
        );
        assert_eq!(
            classify(&p, Some(true), Some(PrinterState::PaperOut), 10).state,
            PrinterState::PaperOut
        );
        assert_eq!(classify(&p, None, None, 10).source, Source::Spooler);
        assert_eq!(classify(&p, Some(true), None, 10).source, Source::Pnp);
    }

    #[test]
    fn non_usb_spooler_and_unrecognized_ports_are_distinct() {
        assert_eq!(
            classify(&printer("IP_192.168.1.20", 0, 0), None, None, 10).state,
            PrinterState::Ready
        );
        assert_eq!(
            classify(&printer("USB001", 0x10, 0), None, None, 10).state,
            PrinterState::PaperOut
        );
        assert_eq!(
            classify(&printer("MYSTERY", 0, 0), None, None, 10).state,
            PrinterState::Unknown
        );
        assert!(usb_port_hint("USB002, LPT1:"));
        assert!(!usb_port_hint("USBPRINTER"));
        assert!(!usb_port_hint("🖨️USB001"));
    }
}
