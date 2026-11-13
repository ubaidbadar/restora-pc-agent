//! Best-effort, per-user Edge PWA provisioning for portable Windows builds.
//! A managed Edge policy installs the app when an eligible Edge profile is active.
//! Packaged MSIX/Store builds are intentionally excluded: HKCU writes can be virtualized.
use std::io;
use winreg::{RegKey, enums::HKEY_CURRENT_USER};
use serde_json::{Value, json};

const EDGE_POLICY_KEY: &str = r"Software\Policies\Microsoft\Edge";
const EDGE_POLICY_NAME: &str = "WebAppInstallForceList";
const APP_URL: &str = "https://app.restora.live";
const DISABLED_FILE: &str = "pwa-install.off";

fn settings_opt_out() -> std::path::PathBuf {
    crate::config::folder().join(DISABLED_FILE)
}

/// Add or remove just Restora. Never overwrite other administrators' web-app entries
/// and never replace malformed policy data, which needs human inspection.
fn change_entries(existing: &str, install: bool) -> Result<Option<String>, serde_json::Error> {
    let mut entries: Vec<Value> = serde_json::from_str(existing)?;
    let contains = entries
        .iter()
        .any(|item| item.get("url").and_then(Value::as_str) == Some(APP_URL));
    if install {
        if contains {
            return Ok(None);
        }
        entries.push(json!({
            "url": APP_URL,
            "default_launch_container": "window",
            "create_desktop_shortcut": true,
            "fallback_app_name": "Restora"
        }));
    } else {
        if !contains {
            return Ok(None);
        }
        entries.retain(|item| item.get("url").and_then(Value::as_str) != Some(APP_URL));
    }
    Ok(Some(serde_json::to_string(&entries)?))
}

fn update_policy(install: bool) -> io::Result<bool> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(EDGE_POLICY_KEY)?;
    let existing = match key.get_value::<String, _>(EDGE_POLICY_NAME) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => "[]".to_string(),
        Err(error) => return Err(error),
    };
    let Some(updated) = change_entries(&existing, install).map_err(io::Error::other)? else {
        return Ok(false);
    };
    if !install && updated == "[]" {
        key.delete_value(EDGE_POLICY_NAME)?;
    } else {
        key.set_value(EDGE_POLICY_NAME, &updated)?;
    }
    Ok(true)
}

/// Called on ordinary portable agent starts; a printer failure must never
/// depend on PWA provisioning succeeding.
pub fn ensure_installed() -> io::Result<()> {
    if settings_opt_out().exists() {
        return Ok(());
    }
    if update_policy(true)? {
        tracing::info!("registered Restora for silent Edge PWA installation (current Windows user)");
    }
    Ok(())
}

/// Explicit maintenance commands; removing the policy allows Edge to remove the
/// managed PWA and prevents the next agent launch from reinstalling it.
pub fn configure(install: bool) -> io::Result<()> {
    if install {
        let _ = std::fs::remove_file(settings_opt_out());
    } else {
        std::fs::write(settings_opt_out(), b"")?;
    }
    if update_policy(install)? {
        tracing::info!("updated Edge PWA policy; install enabled={install}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{APP_URL, change_entries};
    use serde_json::{Value, json};

    #[test]
    fn adds_windowed_desktop_app_and_is_idempotent() {
        let updated = change_entries("[]", true).unwrap().unwrap();
        let entries: Vec<Value> = serde_json::from_str(&updated).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["url"], APP_URL);
        assert_eq!(entries[0]["default_launch_container"], "window");
        assert_eq!(entries[0]["create_desktop_shortcut"], true);
        assert_eq!(change_entries(&updated, true).unwrap(), None);
    }

    #[test]
    fn preserves_unrelated_entries_and_removes_only_restora() {
        let other = json!({"url":"https://business.example","default_launch_container":"tab"});
        let before = serde_json::to_string(&vec![other.clone()]).unwrap();
        let with_app = change_entries(&before, true).unwrap().unwrap();
        let without_app = change_entries(&with_app, false).unwrap().unwrap();
        let entries: Vec<Value> = serde_json::from_str(&without_app).unwrap();
        assert_eq!(entries, vec![other]);
        assert_eq!(change_entries(&without_app, false).unwrap(), None);
    }

    #[test]
    fn malformed_existing_policy_is_never_replaced() {
        assert!(change_entries("{bad json", true).is_err());
        assert!(change_entries("{}", true).is_err());
    }
}
