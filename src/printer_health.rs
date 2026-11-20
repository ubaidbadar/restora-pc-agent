//! Best-effort Windows spooler diagnostics. These are queue/driver reports, not physical paper confirmation.
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrinterHealth {
    pub system_name: String,
    pub state: &'static str,
    pub checked_at: u64,
    pub queued_jobs: u32,
    pub status_flags: u32,
}

/// Windows drivers may report no flags even for a switched-off thermal printer.
/// Never infer "ready" from zero status or from a printer simply being installed.
fn state(status: u32, attributes: u32) -> &'static str {
    // Winspool PRINTER_STATUS_* and PRINTER_ATTRIBUTE_WORK_OFFLINE values.
    if status & (0x0000_0080 | 0x0000_1000) != 0 || attributes & 0x0000_0400 != 0 {
        "offline"
    } else if status & 0x0000_0001 != 0 {
        "paused"
    } else if status & 0x0000_0010 != 0 {
        "paperOut"
    } else if status & 0x0000_0008 != 0 {
        "paperJam"
    } else if status & 0x0040_0000 != 0 {
        "doorOpen"
    } else if status & (0x0000_0002 | 0x0000_0040 | 0x0004_0000 | 0x0010_0000) != 0 {
        "error"
    } else if status & (0x0000_0200 | 0x0000_0400 | 0x0000_4000) != 0 {
        "busy"
    } else {
        "unknown"
    }
}

pub fn unavailable(state: &str) -> bool {
    matches!(
        state,
        "offline" | "paused" | "paperOut" | "paperJam" | "doorOpen" | "error"
    )
}

/// Enumerating level 2 includes status and job count; level 4 only gives names.
/// This can block in a damaged driver, so the caller must run it off the async runtime,
/// never start a second scan while the first is in flight, and expire stale readings.
#[cfg(windows)]
pub fn scan() -> Result<Vec<PrinterHealth>, String> {
    use windows::Win32::Graphics::Printing::{
        EnumPrintersW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_2W,
    };
    use windows::core::PWSTR;

    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0u32, 0u32);
    let first = unsafe { EnumPrintersW(flags, PWSTR::null(), 2, None, &mut needed, &mut count) };
    if needed == 0 {
        return first.map(|_| Vec::new()).map_err(|e| e.to_string());
    }

    // Winspool writes structs containing pointers: ensure pointer-width alignment.
    let words = (needed as usize).div_ceil(std::mem::size_of::<usize>());
    let mut storage = vec![0usize; words];
    let buffer = unsafe {
        std::slice::from_raw_parts_mut(
            storage.as_mut_ptr().cast::<u8>(),
            storage.len() * std::mem::size_of::<usize>(),
        )
    };
    unsafe {
        EnumPrintersW(
            flags,
            PWSTR::null(),
            2,
            Some(buffer),
            &mut needed,
            &mut count,
        )
    }
    .map_err(|e| e.to_string())?;

    let checked_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let infos = unsafe {
        std::slice::from_raw_parts(storage.as_ptr().cast::<PRINTER_INFO_2W>(), count as usize)
    };
    Ok(infos
        .iter()
        .filter_map(|info| {
            let system_name = unsafe { info.pPrinterName.to_string() }.ok()?;
            Some(PrinterHealth {
                system_name,
                state: state(info.Status, info.Attributes),
                checked_at,
                queued_jobs: info.cJobs,
                status_flags: info.Status,
            })
        })
        .collect())
}

#[cfg(not(windows))]
pub fn scan() -> Result<Vec<PrinterHealth>, String> {
    Err("printer health is only available on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::{state, unavailable};

    #[test]
    fn windows_reports_offline_even_if_the_printer_is_still_installed() {
        assert_eq!(state(0x80, 0), "offline");
        assert_eq!(state(0, 0x400), "offline");
        assert_eq!(state(0x1000, 0), "offline");
    }

    #[test]
    fn flags_prefer_actionable_errors_over_busy() {
        assert_eq!(state(0x10 | 0x400, 0), "paperOut");
        assert_eq!(state(0x08, 0), "paperJam");
        assert_eq!(state(0x0040_0000, 0), "doorOpen");
        assert_eq!(state(0x01, 0), "paused");
        assert_eq!(state(0x02, 0), "error");
    }

    #[test]
    fn only_explicit_windows_error_states_block_spooler_submission() {
        assert!(unavailable("offline"));
        assert!(unavailable("paperOut"));
        assert!(!unavailable("unknown"));
        assert!(!unavailable("busy"));
    }

    #[test]
    fn zero_status_is_not_proof_of_hardware_readiness() {
        assert_eq!(state(0, 0), "unknown");
        assert_eq!(state(0x400, 0), "busy");
    }
}
