mod escpos;
mod local;
mod printers;
mod slips;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    // The Restora pages allowed to call the agent; RESTORA_ORIGINS (comma separated) replaces them.
    let origins = std::env::var("RESTORA_ORIGINS")
        .unwrap_or_else(|_| "https://app.restora.live,http://localhost:3000".to_string())
        .split(',')
        .map(|o| o.trim().trim_end_matches('/').to_string())
        .filter(|o| !o.is_empty())
        .collect();
    let listener = tokio::net::TcpListener::bind(local::ADDRESS)
        .await
        .expect("port 17777 is busy: is the agent already running?");
    tracing::info!("listening on http://{}", local::ADDRESS);
    axum::serve(listener, local::router(origins)).await.unwrap();
}
