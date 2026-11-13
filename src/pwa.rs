//! Launch Restora in its installed PWA window when available.
//! Packaged PWAs use their verified AppUserModelID; browser PWAs use the web+restora protocol.
use crate::config;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use winreg::{RegKey, enums::HKEY_CLASSES_ROOT};

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

/// Chrome/Edge Start-menu app IDs are opaque Windows identifiers, not package-family AUMIDs.
fn valid_start_app_id(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 180
        && raw.is_ascii()
        && raw
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'_' | b'-' | b'!'))
}

fn launch_app_id(app_id: &str) -> bool {
    valid_start_app_id(app_id)
        && Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{app_id}"))
            .spawn()
            .is_ok()
}

/// Launch the signed Restora PWA package when it is provisioned for this Windows user.
fn open_packaged_pwa() -> bool {
    let Ok(contents) = std::fs::read_to_string(config::folder().join("pwa-aumid.txt")) else {
        return false;
    };
    let aumid = contents.trim();
    if !valid_aumid(aumid) {
        return false;
    }
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

    installed && launch_app_id(aumid)
}

/// Chrome and Edge register browser-installed PWAs in the Windows Start menu.
fn browser_pwa_app_id() -> Option<String> {
    const QUERY: &str = r#"
$apps = @(Get-StartApps | Where-Object {
    $_.Name -eq 'Restora' -or $_.Name -match '^Restora (.+)$'
})
$exact = @($apps | Where-Object { $_.Name -eq 'Restora' })
if ($exact.Count -eq 1) {
    [Console]::Out.Write($exact[0].AppID)
    exit 0
}
if ($apps.Count -eq 1) {
    [Console]::Out.Write($apps[0].AppID)
    exit 0
}
exit 1
"#;

    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            QUERY,
        ])
        .creation_flags(NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let app_id = String::from_utf8(output.stdout).ok()?;
    let app_id = app_id.trim();
    valid_start_app_id(app_id).then(|| app_id.to_string())
}

fn protocol_uri(path: &str) -> Option<&'static str> {
    match path {
        "/orders" => Some("web+restora://orders"),
        "/settings/printers" => Some("web+restora://settings"),
        _ => None,
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
    // Prefer the protocol even for a packaged PWA: it carries the requested route into the existing window.
    if let Some(uri) = protocol_uri(path) {
        if protocol_handler_registered() && shell_open(uri) {
            return;
        }
    }

    // Legacy installed apps do not understand the protocol yet. Their Start-menu entry is a safe
    // fallback for the ordinary Orders launch; settings uses the HTTPS deep link below.
    if path == "/orders" {
        if open_packaged_pwa() {
            return;
        }
        if browser_pwa_app_id().is_some_and(|app_id| launch_app_id(&app_id)) {
            return;
        }
    }

    let url = format!("{}{}", app_url.trim_end_matches('/'), path);
    if !shell_open(&url) {
        tracing::warn!("could not open Restora at {url}");
    }
}

#[cfg(test)]
mod tests {
    use super::{protocol_uri, valid_aumid, valid_start_app_id};

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
    fn accepts_browser_pwa_start_menu_ids_without_accepting_shell_input() {
        assert!(valid_start_app_id("Chrome._crx_abcdefghijklmnop"));
        assert!(valid_start_app_id("MSEdge.app.abcdefghijklmnop"));
        assert!(!valid_start_app_id(""));
        assert!(!valid_start_app_id("Restora PWA"));
        assert!(!valid_start_app_id("Restora;Start-Process powershell"));
        assert!(!valid_start_app_id("shell:AppsFolder\\Restora"));
    }

    #[test]
    fn protocol_routes_are_allowlisted() {
        assert_eq!(protocol_uri("/orders"), Some("web+restora://orders"));
        assert_eq!(protocol_uri("/settings/printers"), Some("web+restora://settings"));
        assert_eq!(protocol_uri("https://evil.example"), None);
    }
}
