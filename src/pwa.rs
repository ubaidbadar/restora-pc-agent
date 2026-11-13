//! Launch Restora's installed PWA before falling back to a normal browser tab.
use crate::{config, tray};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

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

/// Start-menu AppIDs for Chrome/Edge PWAs are not package-family AUMIDs, but
/// they are still opaque Windows identifiers. Accept only safe identifier bytes.
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

/// The signed Restora PWA package, when provisioned by the single-installer release.
fn open_packaged_pwa() -> bool {
    let Ok(contents) = std::fs::read_to_string(config::folder().join("pwa-aumid.txt")) else {
        return false;
    };
    let aumid = contents.trim();
    if !valid_aumid(aumid) {
        return false;
    }

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

    installed && launch_app_id(aumid)
}

/// Chrome and Edge register installed PWAs in the Windows Start menu. Get-StartApps
/// returns the actual AppUserModelID, which launches the browser's own app shortcut
/// with its original browser profile instead of opening the site's URL in a tab.
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

/// Opens the installed PWA window if available; otherwise uses the normal browser URL.
/// The focus-existing manifest policy keeps an active order screen from being navigated away.
pub fn open(url: &str) {
    if open_packaged_pwa() {
        return;
    }
    if browser_pwa_app_id().is_some_and(|app_id| launch_app_id(&app_id)) {
        return;
    }
    // No installed PWA (or more than one ambiguous profile): use the safe browser fallback.
    tray::open(url);
}

#[cfg(test)]
mod tests {
    use super::{valid_aumid, valid_start_app_id};

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
}
