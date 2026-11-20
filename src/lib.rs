//! Shared native print runtime. Tauri links this crate directly, without starting
//! a second EXE, opening a tray, or exposing privileged IPC to remote pages.
pub mod agent;
pub mod backend;
pub mod config;
pub mod escpos;
pub mod local;
pub mod logo;
pub mod printers;
pub mod slips;
pub mod status;

#[cfg(all(windows, not(feature = "embedded")))]
pub mod launch;
#[cfg(all(windows, not(feature = "embedded")))]
pub mod pwa;

/// Start the same restricted local protocol and server print queue used by the
/// standalone agent. The runtime is owned by this process, so it stops when
/// the Tauri application exits. Binding before spawning detects an existing
/// standalone agent and never silently sends jobs to the wrong process.
#[cfg(windows)]
pub fn start_embedded() -> Result<(), String> {
    use std::sync::Arc;

    let listener = std::net::TcpListener::bind(local::ADDRESS)
        .map_err(|error| format!("Restora printing cannot start on {}: {error}. Close the standalone Restora Agent before opening Desktop.", local::ADDRESS))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let agent = Arc::new(agent::Agent::new());
    std::thread::Builder::new()
        .name("restora-print-runtime".into())
        .spawn(move || {
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("validated Restora print listener");
                tokio::spawn(backend::run(Arc::clone(&agent)));
                // Only the hosted Restora app is allowed to write to the local
                // printing API. Dev access is limited to debug builds.
                let mut origins = vec!["https://app.restora.live".to_string()];
                if cfg!(debug_assertions) {
                    origins.push("http://localhost:3000".to_string());
                    origins.push("http://127.0.0.1:3000".to_string());
                }
                if let Err(error) = axum::serve(listener, local::router(agent, origins)).await {
                    tracing::error!("embedded print listener stopped: {error}");
                }
            });
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(not(windows))]
pub fn start_embedded() -> Result<(), String> {
    Err("Restora built-in raw printing currently requires Windows".into())
}
