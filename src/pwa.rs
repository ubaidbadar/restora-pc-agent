//! Launch Restora in its installed PWA window when available.
//! Packaged PWAs use their verified AppUserModelID; browser PWAs use the web+restora protocol.
use crate::{config, launch};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use winreg::{RegKey, enums::HKEY_CLASSES_ROOT};

const NO_WINDOW: u32 = 0x0800_0000;

fn valid_aumid(raw: &str) -> bool {
    // The provisioned PWA must have its own package identity, never the agent's.
    if raw.starts_with("BadarDigital.RestoraPrint_") || raw.len() > 180 || !raw.is_ascii() {
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

/// Small fixed-output queries only; never let a broken PowerShell profile/service block launch.
fn powershell(query: &str) -> Option<std::process::Output> {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            query,
        ])
        .creation_flags(NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match launch::query_output(&mut command, std::time::Duration::from_secs(3)) {
        Ok(output) if output.status.success() => Some(output),
        Ok(output) => {
            tracing::warn!("PWA lookup returned {}", output.status);
            None
        }
        Err(error) => {
            tracing::warn!("PWA lookup failed: {error}");
            None
        }
    }
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
    let installed = powershell(&query).is_some();

    installed && launch_app_id(aumid)
}

/// Chrome and Edge register browser-installed PWAs in the Windows Start menu.
const BROWSER_QUERY: &str = r#"
$apps = @(Get-StartApps | Where-Object {
    ($_.Name -eq 'Restora' -or $_.Name -match '^Restora (.+)$') -and
    ($_.AppID -match '^(Chrome\._crx_|MSEdge\.)')
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

fn browser_pwa_app_id() -> Option<String> {
    let output = powershell(BROWSER_QUERY)?;
    let app_id = String::from_utf8(output.stdout).ok()?;
    let app_id = app_id.trim();
    launch::browser_app_id(app_id).then(|| app_id.to_string())
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
    // Shell extensions can require STA COM. Use a fresh thread so calls from
    // Tokio workers and Store activation do not inherit an incompatible apartment.
    let target = target.to_string();
    std::thread::spawn(move || unsafe {
        use windows::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
        };
        use windows::Win32::UI::Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
        };
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{HSTRING, PCWSTR, w};

        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        if initialized.is_err() {
            tracing::error!("cannot initialize shell COM apartment: {initialized:?}");
            return false;
        }
        let target = HSTRING::from(target);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            // No message loop lives on this worker: finish dispatch before it exits.
            fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: w!("open"),
            lpFile: PCWSTR(target.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        let result = ShellExecuteExW(&mut info);
        CoUninitialize();
        if let Err(error) = &result {
            tracing::warn!("Windows shell launch failed: {error}");
        }
        result.is_ok()
    })
    .join()
    .unwrap_or(false)
}

pub fn open(app_url: &str, path: &str) {
    tracing::info!("opening Restora route {path}");
    // Development overrides must not activate a production PWA/protocol.
    if app_url.trim_end_matches('/') != "https://app.restora.live" {
        open_web(app_url, path);
        return;
    }
    // Prefer the protocol even for a packaged PWA: it carries the requested route into the existing window.
    if let Some(uri) = protocol_uri(path)
        && protocol_handler_registered()
        && shell_open(uri)
    {
        tracing::info!("Restora launch dispatched to web+restora handler");
        return;
    }

    // Legacy installed apps do not understand the protocol yet. Their Start-menu entry is a safe
    // fallback for the ordinary Orders launch; settings uses the HTTPS deep link below.
    if path == "/orders" {
        if open_packaged_pwa() {
            tracing::info!("Restora launch dispatched to provisioned PWA");
            return;
        }
        if browser_pwa_app_id().is_some_and(|app_id| launch_app_id(&app_id)) {
            tracing::info!("Restora launch dispatched to browser PWA");
            return;
        }
    }

    open_web(app_url, path);
}

fn open_web(app_url: &str, path: &str) {
    let url = format!("{}{}", app_url.trim_end_matches('/'), path);
    tracing::info!("opening HTTPS browser fallback at {url}");
    if !shell_open(&url) {
        tracing::error!("could not open Restora at {url}");
        use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        use windows::core::{HSTRING, w};
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(format!(
                    "Windows could not open Restora. Open {url} in Edge or Chrome. The print agent continues in the background. Diagnostics: {}",
                    config::folder().join("agent.log").display()
                )),
                w!("Restora Agent"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{protocol_uri, valid_aumid, valid_start_app_id};

    #[test]
    fn start_menu_query_excludes_agent_and_handles_missing_or_ambiguous_pwas() {
        // Exercise the actual PowerShell filter with deterministic Start-menu fixtures.
        let query = |apps: &str| {
            super::powershell(&format!(
                "function Get-StartApps {{ {apps} }}\n{}",
                super::BROWSER_QUERY
            ))
            .map(|output| String::from_utf8(output.stdout).unwrap())
        };
        let agent = "[pscustomobject]@{Name='Restora Agent';AppID='BadarDigital.RestoraPrint_pub!RestoraAgent'}";
        let pwa = "[pscustomobject]@{Name='Restora';AppID='Chrome._crx_abcdefghijklmnop'}";
        assert!(query(agent).is_none());
        assert_eq!(
            query(&format!("{agent}; {pwa}")).as_deref(),
            Some("Chrome._crx_abcdefghijklmnop")
        );
        assert!(query(&format!("{pwa}; {pwa}")).is_none());
    }

    #[test]
    fn accepts_only_packaged_app_ids() {
        assert!(valid_aumid("BadarDigital.Restora_8wekyb3d8bbwe!App"));
        assert!(!valid_aumid(""));
        assert!(!valid_aumid("Restora"));
        assert!(!valid_aumid(
            "BadarDigital.RestoraPrint_publisher!RestoraAgent"
        ));
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
        assert_eq!(
            protocol_uri("/settings/printers"),
            Some("web+restora://settings")
        );
        assert_eq!(protocol_uri("https://evil.example"), None);
    }
}
