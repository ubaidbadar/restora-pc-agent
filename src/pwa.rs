//! Launch Restora in its installed PWA window when available.
//! Packaged PWAs use their verified AppUserModelID; Chrome/Edge PWAs use the standard web+restora
//! protocol declared in the web app manifest. No undocumented browser command-line switches are used.
use crate::config;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use winreg::{RegKey, enums::HKEY_CLASSES_ROOT;

const NO_WINDOW: u32 = 0x0800_0000;

fn valid_aumid(raw: &str) -> bool {
    if raw.len() > 180 || !raw.is_ascii() {
        return false;
    }
    let Some((family, app)) = raw.split_once('!') else {
        return false;
    };
    !family.is_empty()
        && !app.is_empty()
        && family.contains('_')
        && !app.contains('!')
        && family
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'_' | b'-'))
        && app
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'_' | b'-'))
}

fn protocol_uri(path: &str) -> &'static str {
    if path == "/settings/printers" {
        "web+restora://settings"
    } else {
        "web+restora://orders"
    }
}

fn protocol_handler_registered() -> bool {
    RegKey::predef(HKEY_CLASSES_ROOT)
        .open_subkey("web+restora")
        .and_then(|key| key.get_raw_value("URL Protocol"))
        .is_ok()
}

fn shell_open(target: &str) -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};

    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(target),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    result.0 as isize > 32
}

pub fn open(app_url: &str, path: &str) {
    if let Ok(contents) = std::fs::read_to_string(config::folder().join("pwa-aumid.txt")) {
        let aumid = contents.trim();
        if valid_aumid(aumid) {
            // Check the current user's registered package, not Start Menu entries,
            // which may lag behind a successful MSIX installation.
            let (family, _) = aumid.split_once('!').expect("validated app identity");
            let (name, _) = family.rsplit_once('_').expect("validated package family");
            let query = format!(
                "if (Get-AppxPackage -Name '{name}' | Where-Object {{ $_.PackageFamilyName -eq '{family}' }} | Select-Object -First 1) {{ exit 0 }}; exit 1"
            );
            let installed = Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &query,
                ])
                .creation_flags(NO_WINDOW)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|result| result.success());
            if installed
                && Command::new("explorer.exe")
                    .arg(format!("shell:AppsFolder\\{aumid}"))
                    .spawn()
                    .is_ok()
            {
                return;
            }
        }
    }

    // A browser-installed PWA registers this supported custom protocol in Windows.
    // Windows/browser consent is required on first use; if no handler exists, do not show
    // an Open With dialog: fall back directly to the normal HTTPS page and its install offer.
    if protocol_handler_registered() && shell_open(protocol_uri(path)) {
        return;
    }

    let url = format!("{}{}", app_url.trim_end_matches('/'), path);
    if !shell_open(&url) {
        tracing::warn!("could not open Restora at {url}");
    }
}

#[cfg(test)]
mod tests {
    use super::{protocol_uri, valid_aumid};

    #[test]
    fn accepts_only_packaged_app_ids() {
        assert!(valid_aumid("BadarDigital.Restora_8wekyb3d8bbwe!App"));
        assert!(!valid_aumid(""));
        assert!(!valid_aumid("Restora"));
        assert!(!valid_aumid("Restora!App"));
        assert!(!valid_aumid("Restora_x!App!Other"));
        assert!(!valid_aumid("Restora_x!';exit 0;#"));
        assert!(!valid_aumid("Restora_x!App/Other"));
    }

    #[test]
    fn launches_only_allowlisted_pwa_destinations() {
        assert_eq!(protocol_uri("/orders"), "web+restora://orders");
        assert_eq!(protocol_uri("/settings/printers"), "web+restora://settings");
        assert_eq!(protocol_uri("https://evil.example"), "web+restora://orders");
    }
}
