//! Testable launch primitives shared by Windows PWA discovery.
#[cfg(any(windows, test))]
pub fn browser_app_id(id: &str) -> bool {
    (id.starts_with("Chrome._crx_") || id.starts_with("MSEdge."))
        && id.len() <= 180
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// Queries must produce only small output (one app ID), below pipe capacity.
#[cfg(any(windows, test))]
pub fn query_output(
    command: &mut std::process::Command,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::{io, time::Instant};
    let mut child = command.spawn()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output(),
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(error) => error,
                    _ => io::Error::new(io::ErrorKind::TimedOut, "PWA lookup timed out"),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_lookup_cannot_relaunch_agent_or_other_packages() {
        assert!(browser_app_id("Chrome._crx_abcdefghijklmnop"));
        assert!(browser_app_id("MSEdge.app.abcdefghijklmnop"));
        for id in [
            "BadarDigital.RestoraPrint_publisher!RestoraAgent",
            "Restora Agent",
            "MSEdge.foo;exit",
            "Chrome._crx_foo\\bar",
            "",
        ] {
            assert!(!browser_app_id(id), "{id}");
        }
    }

    #[test]
    fn query_timeout_kills_and_reaps_child() {
        let mut command;
        #[cfg(windows)]
        {
            command = std::process::Command::new("powershell.exe");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
        }
        #[cfg(not(windows))]
        {
            command = std::process::Command::new("sleep");
            command.arg("30");
        }
        let start = std::time::Instant::now();
        let error = query_output(&mut command, std::time::Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }
}
