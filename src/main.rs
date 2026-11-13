// Release builds on Windows have no console window: the tray icon is the only thing shown.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent;
mod backend;
mod config;
mod escpos;
mod local;
mod logo;
mod printers;
#[cfg(windows)]
mod pwa;
mod slips;
mod status;
#[cfg(windows)]
mod tray;

use std::sync::{Arc, Mutex};

/// Windows has no console to log to, so the log is a file next to the settings.
fn start_logging() {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config::folder().join("agent.log"));
    match file {
        Ok(file) if cfg!(windows) => tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .init(),
        _ => tracing_subscriber::fmt::init(),
    }
}

#[cfg(windows)]
fn embedded_parent(args: &[String]) -> Result<Option<u32>, &'static str> {
    if !args.iter().any(|argument| argument == "--embedded") {
        return Ok(None);
    }
    let parent = args
        .windows(2)
        .find(|pair| pair[0] == "--parent-pid")
        .and_then(|pair| pair[1].parse::<u32>().ok())
        .filter(|pid| *pid > 0)
        .ok_or("Embedded printing requires a valid --parent-pid")?;
    Ok(Some(parent))
}

/// Stop the silent engine when its owning Electron process exits, including after a crash.
#[cfg(windows)]
fn exit_with_parent(parent_pid: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_ACCESS_RIGHTS, WaitForSingleObject,
    };
    std::thread::spawn(move || unsafe {
        // PROCESS_SYNCHRONIZE: wait for the real parent handle, not a reusable PID.
        let Ok(handle) = OpenProcess(PROCESS_ACCESS_RIGHTS(0x0010_0000), false, parent_pid) else {
            std::process::exit(0);
        };
        WaitForSingleObject(handle, INFINITE);
        let _ = CloseHandle(handle);
        std::process::exit(0);
    });
}

#[cfg(windows)]
fn launched_at_startup() -> bool {
    if std::env::args().any(|argument| argument == "--background") {
        return true;
    }
    #[cfg(feature = "store")]
    {
        // Packaged desktop apps expose an explicit StartupTask activation kind.
        if let Ok(activation) = windows::ApplicationModel::AppInstance::GetActivatedEventArgs() {
            if let Ok(kind) = activation.Kind() {
                return kind == windows::ApplicationModel::Activation::ActivationKind::StartupTask;
            }
        }
    }
    false
}

fn main() {
    #[cfg(windows)]
    let embedded_pid = match embedded_parent(&std::env::args().collect::<Vec<_>>()) {
        Ok(pid) => pid,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    start_logging();
    #[cfg(windows)]
    if let Some(parent) = embedded_pid {
        exit_with_parent(parent);
    }
    #[cfg(windows)]
    // Opt-in until the signed PWA + single-installer has passed Windows acceptance.
    // The existing Store and standalone printing behavior remains unchanged.
    #[cfg(windows)]
    let headless = embedded_pid.is_none() && std::env::args().any(|arg| arg == "--headless");
    #[cfg(windows)]
    let interactive = embedded_pid.is_none() && !launched_at_startup();
    // The Restora pages allowed to call the agent; RESTORA_ORIGINS (comma separated) replaces them.
    let origins = std::env::var("RESTORA_ORIGINS")
        .unwrap_or_else(|_| "https://app.restora.live,https://dev-app.restora.live,https://restora-live.web.app,https://restora-live.firebaseapp.com,https://restora-live-dev.web.app,https://restora-live-dev.firebaseapp.com,http://localhost:3000".to_string())
        .split(',')
        .map(|o| o.trim().trim_end_matches('/').to_string())
        .filter(|o| !o.is_empty())
        .collect();
    let agent = Arc::new(agent::Agent::new());
    let runtime = tokio::runtime::Runtime::new().expect("the async runtime");

    let listener = match runtime.block_on(tokio::net::TcpListener::bind(local::ADDRESS)) {
        Ok(listener) => listener,
        Err(err) => {
            // Most likely the agent is already running: open Restora instead of another agent window.
            tracing::error!("cannot listen on {}: {err}", local::ADDRESS);
            #[cfg(windows)]
            if interactive {
                if headless {
                    pwa::open();
                } else {
                    tray::open(&local::printer_settings_url());
                }
            }
            #[cfg(not(windows))]
            local::open_setup();
            return;
        }
    };
    tracing::info!("listening on http://{}", local::ADDRESS);
    runtime.spawn(backend::run(agent.clone()));
    let paired = agent.paired();
    #[cfg(windows)]
    let agent_for_tray = agent.clone();
    runtime.spawn(async move {
        axum::serve(listener, local::router(agent, origins))
            .await
            .unwrap()
    });
    // Interactive launch opens Restora. Windows auto-start stays in the tray
    // without opening a browser every time the employee signs in.
    #[cfg(windows)]
    if interactive {
        if headless {
            pwa::open();
        } else if paired {
            tray::open(&local::app_url());
        } else {
            tray::open(&local::printer_settings_url());
        }
    }
    #[cfg(not(windows))]
    if !paired {
        local::open_setup();
    }

    #[cfg(windows)]
    if embedded_pid.is_some() || headless {
        // Electron remains parent-bound. The opt-in headless standalone process
        // runs the same engine without a tray, local dashboard or browser on startup.
        runtime.block_on(std::future::pending::<()>());
    } else {
        tray::run(
            &format!("http://{}", local::ADDRESS),
            &local::app_url(),
            agent_for_tray,
        );
    }
    // Elsewhere (development) the agent runs until it is stopped.
    #[cfg(not(windows))]
    runtime.block_on(tokio::signal::ctrl_c()).ok();
}

#[cfg(all(test, windows))]
mod embedded_tests {
    use super::embedded_parent;

    #[test]
    fn embedded_mode_requires_parent_and_standalone_is_unchanged() {
        let args = |parts: &[&str]| {
            parts
                .iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(embedded_parent(&args(&["engine"])), Ok(None));
        assert_eq!(
            embedded_parent(&args(&["engine", "--background"])),
            Ok(None)
        );
        assert_eq!(
            embedded_parent(&args(&["engine", "--embedded"])),
            Err("Embedded printing requires a valid --parent-pid")
        );
        assert_eq!(
            embedded_parent(&args(&["engine", "--embedded", "--parent-pid", "0"])),
            Err("Embedded printing requires a valid --parent-pid")
        );
        assert_eq!(
            embedded_parent(&args(&["engine", "--embedded", "--parent-pid", "123"])),
            Ok(Some(123))
        );
    }
}
