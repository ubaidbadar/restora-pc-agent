/// The user's Windows default printer. Discovery alone does not imply a
/// printer should receive restaurant slips, so the backend only auto-routes to
/// this explicit OS choice.
#[cfg(windows)]
pub fn default_name() -> Option<String> {
    use windows::Win32::Graphics::Printing::GetDefaultPrinterW;
    use windows::core::PWSTR;

    let mut size = 0u32;
    let _ = unsafe { GetDefaultPrinterW(None, &mut size) };
    if size == 0 || size > 4096 {
        return None;
    }
    let mut buffer = vec![0u16; size as usize];
    if !unsafe { GetDefaultPrinterW(Some(PWSTR(buffer.as_mut_ptr())), &mut size) }.as_bool() {
        return None;
    }
    let end = buffer
        .iter()
        .position(|ch| *ch == 0)
        .unwrap_or(buffer.len());
    String::from_utf16(&buffer[..end])
        .ok()
        .filter(|name| !name.is_empty())
}

#[cfg(not(windows))]
pub fn default_name() -> Option<String> {
    None
}

/// Names of the printers installed in Windows, as the spooler knows them.
#[cfg(windows)]
pub fn list() -> Result<Vec<String>, String> {
    use windows::Win32::Graphics::Printing::{
        EnumPrintersW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_4W,
    };
    use windows::core::PWSTR;

    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0u32, 0u32);
    // The first call only says how many bytes the list takes; it "fails" with insufficient buffer.
    let _ = unsafe { EnumPrintersW(flags, PWSTR::null(), 4, None, &mut needed, &mut count) };
    if needed == 0 {
        return Ok(Vec::new());
    }
    let mut buffer = vec![0u8; needed as usize];
    unsafe {
        EnumPrintersW(
            flags,
            PWSTR::null(),
            4,
            Some(&mut buffer),
            &mut needed,
            &mut count,
        )
    }
    .map_err(|e| e.to_string())?;
    let infos = unsafe {
        std::slice::from_raw_parts(buffer.as_ptr() as *const PRINTER_INFO_4W, count as usize)
    };
    Ok(infos
        .iter()
        .filter_map(|info| unsafe { info.pPrinterName.to_string().ok() })
        .collect())
}

/// Windows can list an installed printer that is visibly offline. Known
/// spooler errors are not successful print submissions; leave the slip queued.
/// Status 0 is *unknown*, not proof of readiness or physical paper delivery.
fn known_printer_problem(status: u32) -> Option<&'static str> {
    const PROBLEMS: &[(u32, &str)] = &[
        (0x0000_0080, "Windows reports that the printer is offline"),
        (0x0000_0001, "Windows reports that the printer is paused"),
        (
            0x0000_0010,
            "Windows reports that the printer is out of paper",
        ),
        (0x0000_0008, "Windows reports a paper jam"),
        (0x0000_0040, "Windows reports a paper problem"),
        (0x0040_0000, "Windows reports an open printer cover"),
        (
            0x0010_0000,
            "Windows reports that the printer needs attention",
        ),
        (
            0x0000_1000,
            "Windows reports that the printer is unavailable",
        ),
        (0x0000_0002, "Windows reports a printer error"),
    ];
    PROBLEMS
        .iter()
        .find(|(flag, _)| status & flag != 0)
        .map(|(_, message)| *message)
}

/// A fast Winspool status hint, not a guarantee the device is connected.
/// Microsoft notes that network printer state can be cached and stale.
#[cfg(windows)]
fn spooler_status(handle: windows::Win32::Graphics::Printing::PRINTER_HANDLE) -> Option<u32> {
    use windows::Win32::Graphics::Printing::GetPrinterW;

    let mut needed = 0u32;
    let _ = unsafe { GetPrinterW(handle, 6, None, &mut needed) };
    if !(4..=4096).contains(&needed) {
        return None;
    }
    let mut buffer = vec![0u8; needed as usize];
    unsafe { GetPrinterW(handle, 6, Some(&mut buffer), &mut needed) }.ok()?;
    let bytes: [u8; 4] = buffer.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// Sends `bytes` to the printer as a RAW job: the printer gets them exactly as they are (ESC/POS), no driver rendering.
#[cfg(windows)]
pub fn print_raw(printer: &str, bytes: &[u8]) -> Result<(), String> {
    use windows::Win32::Graphics::Printing::{
        AbortPrinter, ClosePrinter, DOC_INFO_1W, EndDocPrinter, EndPagePrinter, OpenPrinterW,
        PRINTER_HANDLE, StartDocPrinterW, StartPagePrinter, WritePrinter,
    };
    use windows::core::{PCWSTR, PWSTR};

    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let name = wide(printer);
    let mut handle = PRINTER_HANDLE::default();
    unsafe { OpenPrinterW(PCWSTR(name.as_ptr()), &mut handle, None) }
        .map_err(|e| format!("cannot open printer: {e}"))?;
    // A disconnected USB/network printer may remain installed and enumerated.
    // Do not mark such an order "printed" just because Windows accepts it
    // into a queue that cannot currently deliver it.
    if let Some(problem) = spooler_status(handle).and_then(known_printer_problem) {
        let _ = unsafe { ClosePrinter(handle) };
        return Err(problem.to_string());
    }

    let mut title = wide("Restora slip");
    let mut datatype = wide("RAW");
    let doc = DOC_INFO_1W {
        pDocName: PWSTR(title.as_mut_ptr()),
        pOutputFile: PWSTR::null(),
        pDatatype: PWSTR(datatype.as_mut_ptr()),
    };

    let result = (|| unsafe {
        if StartDocPrinterW(handle, 1, &doc) == 0 {
            return Err("cannot start the print job".to_string());
        }
        let written = if StartPagePrinter(handle).as_bool() {
            let mut written = 0u32;
            let ok = WritePrinter(
                handle,
                bytes.as_ptr() as *const _,
                bytes.len() as u32,
                &mut written,
            )
            .as_bool();
            let ended = EndPagePrinter(handle).as_bool();
            ok && ended && written as usize == bytes.len()
        } else {
            false
        };
        if !written {
            let _ = AbortPrinter(handle);
            return Err("the printer did not take the whole slip".to_string());
        }
        if !EndDocPrinter(handle).as_bool() {
            let _ = AbortPrinter(handle);
            return Err("Windows could not finish the print job".to_string());
        }
        Ok(())
    })();
    let _ = unsafe { ClosePrinter(handle) };
    result
}

#[cfg(not(windows))]
pub fn list() -> Result<Vec<String>, String> {
    Err("printers can only be listed on Windows".to_string())
}

#[cfg(not(windows))]
pub fn print_raw(_printer: &str, _bytes: &[u8]) -> Result<(), String> {
    Err("printing is only available on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::known_printer_problem;

    #[test]
    fn installed_but_offline_printer_does_not_count_as_printed() {
        assert_eq!(
            known_printer_problem(0x0000_0080),
            Some("Windows reports that the printer is offline")
        );
        assert!(known_printer_problem(0x0000_0010).is_some());
        assert!(known_printer_problem(0x0040_0000).is_some());
        assert!(known_printer_problem(0x0000_0001).is_some());
    }

    #[test]
    fn a_spooler_without_a_known_problem_is_not_mistaken_for_a_failure() {
        assert_eq!(known_printer_problem(0), None);
        assert_eq!(known_printer_problem(0x0000_0400), None);
    }
}
