/// Names of the printers installed in Windows, as the spooler knows them.
/// Level 4 avoids remote OpenPrinter/RPC waits that level 2 causes on shared queues.
#[cfg(windows)]
pub fn list() -> Result<Vec<String>, String> {
    use windows::Win32::Graphics::Printing::{
        EnumPrintersW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_4W,
    };
    use windows::core::PWSTR;

    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0u32, 0u32);
    let first = unsafe { EnumPrintersW(flags, PWSTR::null(), 4, None, &mut needed, &mut count) };
    if needed == 0 {
        return first.map(|_| Vec::new()).map_err(|err| err.to_string());
    }
    // A Winspool buffer contains native pointer-bearing structs. Vec<u8>
    // does not guarantee their alignment; allocate pointer-aligned storage.
    for _ in 0..3 {
        if needed as usize > 16 * 1024 * 1024 {
            return Err("Windows printer inventory exceeds the safety limit".to_string());
        }
        let words = (needed as usize).div_ceil(std::mem::size_of::<usize>());
        let mut storage = vec![0usize; words];
        let available = storage.len() * std::mem::size_of::<usize>();
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(storage.as_mut_ptr().cast::<u8>(), available)
        };
        let capacity = available as u32;
        match unsafe {
            EnumPrintersW(
                flags,
                PWSTR::null(),
                4,
                Some(bytes),
                &mut needed,
                &mut count,
            )
        } {
            Ok(()) => {
                if (count as usize) > available / std::mem::size_of::<PRINTER_INFO_4W>() {
                    return Err("Windows returned a malformed printer count".to_string());
                }
                let infos = unsafe {
                    std::slice::from_raw_parts(
                        storage.as_ptr().cast::<PRINTER_INFO_4W>(),
                        count as usize,
                    )
                };
                let mut names: Vec<_> = infos
                    .iter()
                    .filter_map(|info| unsafe { info.pPrinterName.to_string().ok() })
                    .collect();
                names.sort();
                names.dedup();
                return Ok(names);
            }
            Err(err) if needed > capacity => {
                // Printer inventory grew between the sizing and reading calls.
                tracing::debug!("Windows printer inventory changed while reading: {err}");
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("Windows printer inventory changed repeatedly during discovery".to_string())
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
