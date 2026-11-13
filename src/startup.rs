#[cfg(not(feature = "store"))]
pub fn ensure_autostart() {
    use crate::config;
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_NAME: &str = "RestoraPrintAgent";

    if config::folder().join("autostart.off").exists() {
        return;
    }
    let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY) else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let command = format!("\"{}\" --background", exe.display());
    if key.get_value::<String, _>(RUN_NAME).ok().as_deref() != Some(command.as_str()) {
        let _ = key.set_value(RUN_NAME, &command);
    }
}

#[cfg(feature = "store")]
pub fn ensure_autostart() {}
