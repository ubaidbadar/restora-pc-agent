//! Launch the verified, packaged Restora PWA when installed for this Windows user.
//! The provisioning step writes only the AUMID. We do not set browser policies,
//! install extensions, bypass package signing, or fabricate an Edge app window.
use crate::{config, local, tray};
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

pub fn open() {
    if let Ok(contents) = std::fs::read_to_string(config::folder().join("pwa-aumid.txt")) {
        let aumid = contents.trim();
        if valid_aumid(aumid) {
            // Get-StartApps is the supported Windows inventory for installed app IDs.
            // Do not invoke a stale AUMID after a customer uninstalls the PWA.
            let query = format!(
                "if (Get-StartApps | Where-Object {{ $_.AppID -eq '{aumid}' }} | Select-Object -First 1) {{ exit 0 }}; exit 1"
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
    // Fallback remains usable without a signed MSIX package, or after uninstall.
    // The website itself never receives privileges from the native agent.
    tray::open(&local::app_url());
}

#[cfg(test)]
mod tests {
    use super::valid_aumid;

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
}
