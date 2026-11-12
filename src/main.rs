// Release builds on Windows have no console window: the tray icon is the only thing shown.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent;
mod backend;
mod config;
mod escpos;
mod local;
mod logo;
mod printers;
mod slips;
mod status;
#[cfg(windows)]
mod tray;

use std::sync::{Arc, Mutex};

/// Windows has no console to log to, so the log is a file next to the settings.
fn start_logging() {
    let file = std::fs::OpenOptions::new().create(true).append(true).open(config::folder().join("agent.log"));
    match file {
        Ok(file) if cfg!(windows) => tracing_subscriber::fmt().with_ansi(false).with_writer(Mutex::new(file)).init(),
        _ => tracing_subscriber::fmt::init(),
    }
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
    start_logging();
    #[cfg(windows)]
    let interactive = !launched_at_startup();
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
                tray::open(&local::printer_settings_url());
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
    runtime.spawn(async move { axum::serve(listener, local::router(agent, origins)).await.unwrap() });
    // Interactive launch opens Restora. Windows auto-start stays in the tray
    // without opening a browser every time the employee signs in.
    #[cfg(windows)]
    if interactive {
        if paired {
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
    tray::run(&format!("http://{}", local::ADDRESS), &local::app_url(), agent_for_tray);
    // Elsewhere (development) the agent runs until it is stopped.
    #[cfg(not(windows))]
    runtime.block_on(tokio::signal::ctrl_c()).ok();
}
