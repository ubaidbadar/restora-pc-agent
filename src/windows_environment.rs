//! Query the **effective** Windows Protected Print Mode state.
//! This official WinRT API is available on Windows 11 24H2 (build 26100+).
//! Unsupported Windows builds and activation errors must return Unknown (None).
#[cfg(windows)]
pub fn effective_wpp() -> Option<bool> {
    use windows::Graphics::Printing::ProtectedPrint::WindowsProtectedPrintInfo;
    match WindowsProtectedPrintInfo::IsProtectedPrintEnabled() {
        Ok(enabled) => Some(enabled),
        Err(error) => {
            tracing::debug!("Windows Protected Print Mode cannot be queried: {error}");
            None
        }
    }
}

#[cfg(not(windows))]
pub fn effective_wpp() -> Option<bool> {
    None
}
