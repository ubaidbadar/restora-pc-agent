use crate::printer_state::WinPrinter;

/// Enumerate Winspool level-2 queue metadata. Caller must isolate this synchronous API;
/// on remote connections EnumPrinters may block in network/driver RPC.
#[cfg(windows)]
pub fn list() -> Result<Vec<WinPrinter>, String> {
    use windows::Win32::Graphics::Printing::{
        EnumPrintersW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_2W,
    };
    use windows::core::PWSTR;

    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0u32, 0u32);
    // The size query normally fails with ERROR_INSUFFICIENT_BUFFER.
    let initial = unsafe { EnumPrintersW(flags, PWSTR::null(), 2, None, &mut needed, &mut count) };
    if needed == 0 {
        initial.map_err(|e| e.to_string())?;
        return Ok(Vec::new());
    }

    // The returned Winspool data has pointer-containing PRINTER_INFO_2W values.
    // Use a usize-aligned allocation, NOT a potentially underaligned Vec<u8>.
    let word = std::mem::size_of::<usize>();
    let mut size = needed as usize;
    for retry in 0..=1 {
        let mut storage = vec![0usize; size.div_ceil(word)];
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(storage.as_mut_ptr().cast::<u8>(), storage.len() * word)
        };
        let allocation = bytes.len();
        let result = unsafe {
            EnumPrintersW(
                flags,
                PWSTR::null(),
                2,
                Some(bytes),
                &mut needed,
                &mut count,
            )
        };
        match result {
            Ok(()) => {
                let count = count as usize;
                let entry_size = std::mem::size_of::<PRINTER_INFO_2W>();
                if count > allocation / entry_size {
                    return Err("Winspool returned an invalid printer count".into());
                }
                let infos = unsafe {
                    std::slice::from_raw_parts(storage.as_ptr().cast::<PRINTER_INFO_2W>(), count)
                };
                return infos
                    .iter()
                    .map(|item| {
                        let name = unsafe { item.pPrinterName.to_string() }
                            .map_err(|e| format!("invalid Windows printer name: {e}"))?;
                        let port = if item.pPortName.is_null() {
                            String::new()
                        } else {
                            unsafe { item.pPortName.to_string() }.unwrap_or_default()
                        };
                        Ok(WinPrinter {
                            name,
                            port,
                            status: item.Status,
                            attributes: item.Attributes,
                            jobs: item.cJobs,
                        })
                    })
                    .collect();
            }
            Err(_error) if retry == 0 && needed as usize > allocation => {
                size = needed as usize;
                continue;
            }
            Err(error) => return Err(format!("EnumPrintersW level 2: {error}")),
        }
    }
    Err("Windows printer enumeration did not complete".to_string())
}

/// The legacy pairing and reporting protocol only sends name strings.
pub fn names(printers: &[WinPrinter]) -> Vec<String> {
    printers.iter().map(|p| p.name.clone()).collect()
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
pub fn list() -> Result<Vec<WinPrinter>, String> {
    Err("printers can only be listed on Windows".to_string())
}

#[cfg(not(windows))]
pub fn print_raw(_printer: &str, _bytes: &[u8]) -> Result<(), String> {
    Err("printing is only available on Windows".to_string())
}
