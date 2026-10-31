//! What the setup page and the tray show: one honest summary built from facts the agent really has.
//! Four separate layers are kept apart: this program (always running if this is answered), the connection to Restora,
//! the printers Windows lists, and the last result of each slip. Windows cannot say whether a printer has paper or is
//! powered, so "found in Windows" is never shown as "ready".
use crate::{agent::Agent, printers};
use serde::Serialize;

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    /// Nothing to report yet (not paired).
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
    /// The name staff gave it in Restora, else the Windows name.
    pub name: String,
    pub system_name: String,
    /// What it prints: "kitchen", "receipt".
    pub prints: Vec<String>,
    /// In the Windows list now: true, false, or null when this PC cannot say.
    pub found: Option<bool>,
    /// Whether the last slip sent to it printed: true, false, or null when nothing was sent since this program started.
    pub last_ok: Option<bool>,
    /// Listed in Windows but not set up in Restora.
    pub unused: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub paired: bool,
    pub online: bool,
    pub version: &'static str,
    pub headline: Headline,
    pub printers: Vec<PrinterRow>,
    pub recent: Vec<crate::agent::Recent>,
}

/// The one line shown first, worst news first. "Good" only when every layer checked out.
pub fn headline(paired: bool, online: bool, set_up: usize, missing: usize, failing: usize, found: usize) -> Headline {
    let (tone, title, detail) = if !paired {
        (Tone::Neutral, "Not connected to Restora yet".to_string(), "Type the 6-digit code from Restora to connect this PC.".to_string())
    } else if !online {
        (Tone::Warn, "No internet connection".to_string(), "New slips wait in Restora and print here as soon as the internet is back.".to_string())
    } else if set_up == 0 {
        (Tone::Warn, "No printer is set up yet".to_string(), "In Restora, open Settings, then Printers, and choose what each printer prints.".to_string())
    } else if missing > 0 {
        let what = if missing == 1 { "A printer was not found on this PC".to_string() } else { format!("{missing} printers were not found on this PC") };
        (Tone::Bad, what, "Check that it is switched on and plugged in. Slips for it wait and print when it is back.".to_string())
    } else if failing > 0 {
        let what = if failing == 1 { "The last slip did not print".to_string() } else { format!("{failing} printers did not print their last slip") };
        (Tone::Bad, what, "Check paper and the cover. Restora tries again for 15 minutes, then marks the slip as failed.".to_string())
    } else {
        let n = if found == 1 { "1 printer is".to_string() } else { format!("{found} printers are") };
        (Tone::Good, "Connected and listening".to_string(), format!("{n} found on this PC. Windows cannot tell paper or cover status, so check those when a slip is missing."))
    };
    Headline { tone, title, detail }
}

/// Reads everything once. Cheap enough to call every few seconds; blocking, so async callers use `spawn_blocking`.
pub fn snapshot(agent: &Agent) -> Snapshot {
    let paired = agent.paired();
    let online = agent.online.load(std::sync::atomic::Ordering::Relaxed);
    let windows = printers::list().ok();
    let recent = agent.recent();
    let configured = agent.printers();

    let mut rows: Vec<PrinterRow> = Vec::new();
    for p in configured.as_array().into_iter().flatten() {
        let Some(system_name) = p["systemName"].as_str() else { continue };
        let label = p["label"].as_str().filter(|l| !l.trim().is_empty()).unwrap_or(system_name);
        let prints = p["prints"].as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(String::from)).collect()).unwrap_or_default();
        rows.push(PrinterRow {
            name: label.to_string(),
            system_name: system_name.to_string(),
            prints,
            found: windows.as_ref().map(|list| list.iter().any(|n| n == system_name)),
            last_ok: recent.iter().find(|r| r.printer == system_name).map(|r| r.state == "printed"),
            unused: false,
        });
    }
    let set_up = rows.len();
    if let Some(list) = &windows {
        for name in list {
            if !rows.iter().any(|r| &r.system_name == name) {
                rows.push(PrinterRow { name: name.clone(), system_name: name.clone(), prints: Vec::new(), found: Some(true), last_ok: None, unused: true });
            }
        }
    }
    let used = &rows[..set_up];
    let missing = used.iter().filter(|r| r.found == Some(false)).count();
    let failing = used.iter().filter(|r| r.last_ok == Some(false)).count();
    let found = used.iter().filter(|r| r.found == Some(true)).count();
    Snapshot { paired, online, version: env!("CARGO_PKG_VERSION"), headline: headline(paired, online, set_up, missing, failing, found), printers: rows, recent }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_good_without_internet_or_printers() {
        assert_eq!(headline(false, false, 0, 0, 0, 0).tone, Tone::Neutral);
        assert_eq!(headline(true, false, 2, 0, 0, 2).tone, Tone::Warn);
        assert_eq!(headline(true, true, 0, 0, 0, 0).tone, Tone::Warn);
    }

    #[test]
    fn a_missing_or_failing_printer_is_bad_not_green() {
        assert_eq!(headline(true, true, 2, 1, 0, 1).tone, Tone::Bad);
        assert_eq!(headline(true, true, 2, 0, 1, 2).tone, Tone::Bad);
    }

    #[test]
    fn good_only_when_every_layer_checks_out() {
        let h = headline(true, true, 2, 0, 0, 2);
        assert_eq!(h.tone, Tone::Good);
        assert!(h.detail.contains("2 printers are found"));
    }
}
