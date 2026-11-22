//! Local status is only a snapshot of cached evidence, never a spooler call.
use crate::{
    agent::Agent,
    config,
    printer_state::{PrinterHealth, PrinterState, Source},
};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Neutral,
    Good,
    Warn,
    Bad,
}

#[derive(Serialize, Debug)]
pub struct Headline {
    pub tone: Tone,
    pub title: String,
    pub detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrinterRow {
    pub name: String,
    pub system_name: String,
    pub prints: Vec<String>,
    /// Backward compatible: does Windows still list the queue, not physical readiness?
    pub found: Option<bool>,
    /// Last *spooler submission*, not a physical paper-output confirmation.
    pub last_ok: Option<bool>,
    pub unused: bool,
    pub state: PrinterState,
    pub source: Option<Source>,
    pub checked_at: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub paired: bool,
    pub reconnect_required: bool,
    pub branch_id: Option<String>,
    pub online: bool,
    pub version: &'static str,
    pub headline: Headline,
    pub printers: Vec<PrinterRow>,
    pub recent: Vec<crate::agent::Recent>,
    /// This value must never be false unless Windows' effective WPP API says so.
    pub wpp: serde_json::Value,
    /// Inventory of USBPRINT interfaces, not a queue-to-hardware association.
    pub usbprint_interfaces_present: Option<usize>,
}

pub fn headline(
    paired: bool,
    online: bool,
    states: &[PrinterState],
    last_failed: bool,
    checked: bool,
    wpp: Option<bool>,
) -> Headline {
    let (tone, title, detail) = if wpp == Some(true) {
        (
            Tone::Bad,
            "Windows Protected Print Mode is enabled",
            "Legacy RAW ESC/POS printing is not supported in protected mode. Turn it off in Windows printer settings and reconnect your printer.",
        )
    } else if !paired {
        (
            Tone::Neutral,
            "Not connected to Restora yet",
            "Connect this PC in Restora Settings > Printers.",
        )
    } else if !online {
        (
            Tone::Warn,
            "No internet connection",
            "Remote slips wait in Restora. Local printing may still be available.",
        )
    } else if states.is_empty() {
        (
            Tone::Warn,
            "No printer is set up yet",
            "Open Restora Settings > Printers and assign your print queues.",
        )
    } else if !checked {
        (
            Tone::Neutral,
            "Checking Windows printers",
            "Printer discovery has not finished; printing remains available.",
        )
    } else if states
        .iter()
        .any(|s| matches!(s, PrinterState::NotFound | PrinterState::Offline))
    {
        (
            Tone::Bad,
            "A printer is missing or offline",
            "Check power, cables and the Windows print queue.",
        )
    } else if states
        .iter()
        .any(|s| matches!(s, PrinterState::PaperOut | PrinterState::Error))
    {
        (
            Tone::Bad,
            "A printer needs attention",
            "Check paper, cover and Windows printer errors.",
        )
    } else if last_failed {
        (
            Tone::Bad,
            "The last printer submission failed",
            "Check the printer. Restora's existing retry schedule remains active.",
        )
    } else if states.contains(&PrinterState::Unknown) {
        (
            Tone::Warn,
            "Printer readiness is unverified",
            "Windows can list a USB printer even when it is off. No supported hardware status was confirmed.",
        )
    } else {
        (
            Tone::Good,
            "Connected and listening",
            "The available print queues report ready. This does not guarantee physical paper output.",
        )
    };
    Headline {
        tone,
        title: title.into(),
        detail: detail.into(),
    }
}

fn observation(
    name: &str,
    health: &Option<Vec<PrinterHealth>>,
    windows_known: bool,
    now: u64,
) -> (PrinterState, Option<Source>, Option<u64>) {
    if let Some(found) = health
        .as_ref()
        .and_then(|list| list.iter().find(|h| h.system_name == name))
    {
        // No stale green (or stale errors). Keep the timestamp so users can see the age.
        let state = if now.saturating_sub(found.checked_at) > 90 {
            PrinterState::Unknown
        } else {
            found.state
        };
        (state, Some(found.source), Some(found.checked_at))
    } else if windows_known {
        (PrinterState::NotFound, Some(Source::Spooler), None)
    } else {
        (PrinterState::Unknown, None, None)
    }
}

pub fn snapshot(agent: &Agent) -> Snapshot {
    let paired = agent.paired();
    let branch_id = agent
        .config
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|c| c.branch_id.clone());
    let online = agent.online.load(std::sync::atomic::Ordering::Relaxed);
    let windows = agent.windows_printers();
    let health = agent.printer_health();
    let checked = windows.is_some();
    let recent = agent.recent();
    let configured = agent.printers();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut rows = Vec::<PrinterRow>::new();
    for printer in configured.as_array().into_iter().flatten() {
        let Some(system_name) = printer["systemName"].as_str() else {
            continue;
        };
        let label = printer["label"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(system_name);
        let prints = printer["prints"]
            .as_array()
            .map(|kinds| {
                kinds
                    .iter()
                    .filter_map(|kind| kind.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let (state, source, checked_at) = observation(system_name, &health, checked, now);
        rows.push(PrinterRow {
            name: label.to_string(),
            system_name: system_name.to_string(),
            prints,
            found: windows
                .as_ref()
                .map(|list| list.iter().any(|name| name == system_name)),
            last_ok: agent.last_result(system_name).map(|r| r.ok),
            unused: false,
            state,
            source,
            checked_at,
        });
    }
    let configured_count = rows.len();
    if let Some(list) = &windows {
        for name in list {
            if rows.iter().any(|row| &row.system_name == name) {
                continue;
            }
            let (state, source, checked_at) = observation(name, &health, checked, now);
            rows.push(PrinterRow {
                name: name.clone(),
                system_name: name.clone(),
                prints: Vec::new(),
                found: Some(true),
                last_ok: agent.last_result(name).map(|r| r.ok),
                unused: true,
                state,
                source,
                checked_at,
            });
        }
    }
    let selected = &rows[..configured_count];
    let states = selected.iter().map(|r| r.state).collect::<Vec<_>>();
    let last_failed = selected.iter().any(|r| r.last_ok == Some(false));
    // The effective WPP state is currently unknown on windows 0.61.3.
    // A policy registry value is NOT an effective-mode query.
    let wpp = serde_json::Value::String("unknown".into());
    Snapshot {
        paired,
        reconnect_required: config::reconnect_required(),
        branch_id,
        online,
        version: env!("CARGO_PKG_VERSION"),
        headline: headline(paired, online, &states, last_failed, checked, None),
        printers: rows,
        recent,
        wpp,
        usbprint_interfaces_present: agent.pnp_count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usb_unknown_never_gets_a_green_headline() {
        assert_eq!(
            headline(true, true, &[PrinterState::Unknown], false, true, None).tone,
            Tone::Warn
        );
        assert_eq!(
            headline(true, true, &[PrinterState::Ready], false, true, None).tone,
            Tone::Good
        );
    }
    #[test]
    fn missing_faults_and_wpp_are_bad() {
        for state in [
            PrinterState::NotFound,
            PrinterState::Offline,
            PrinterState::PaperOut,
            PrinterState::Error,
        ] {
            assert_eq!(
                headline(true, true, &[state], false, true, None).tone,
                Tone::Bad
            );
        }
        assert_eq!(
            headline(true, true, &[PrinterState::Ready], false, true, Some(true)).tone,
            Tone::Bad
        );
        assert_eq!(
            headline(true, true, &[PrinterState::Ready], true, true, None).tone,
            Tone::Bad
        );
    }
    #[test]
    fn no_discovery_is_not_a_false_missing_printer() {
        assert_eq!(
            observation("Kitchen", &None, false, 100).0,
            PrinterState::Unknown
        );
        assert_eq!(
            observation("Kitchen", &Some(vec![]), true, 100).0,
            PrinterState::NotFound
        );
    }
    #[test]
    fn stale_readiness_downgrades_to_unknown() {
        let h = PrinterHealth {
            system_name: "Kitchen".into(),
            port: "USB001".into(),
            state: PrinterState::Ready,
            source: Source::Printer,
            checked_at: 10,
        };
        assert_eq!(
            observation("Kitchen", &Some(vec![h]), true, 101).0,
            PrinterState::Unknown
        );
    }
}
