use crate::agent::{Agent, Slip};
use crate::{printers, slips};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub const ADDRESS: &str = "127.0.0.1:17777";

struct Local {
    agent: Arc<Agent>,
    origins: Vec<String>,
}

type Shared = Arc<Local>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrintRequest {
    printer: String,
    /// "kitchen" or "receipt".
    kind: String,
    /// The order's clientId when it has one, else its _id: what tells a slip printed offline from the synced order's.
    order_key: String,
    /// Characters per line: 48 (80 mm, default) or 32 (58 mm).
    width: Option<usize>,
    copies: Option<u32>,
    order: slips::Order,
    #[serde(default)]
    header: slips::Header,
}

#[derive(Deserialize)]
struct PairRequest {
    code: String,
}

/// `origins` are the pages allowed to call the agent (the Restora web app); the agent's own setup page is always one.
pub fn router(agent: Arc<Agent>, mut origins: Vec<String>) -> Router {
    origins.push(format!("http://{ADDRESS}"));
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins.iter().filter_map(|o| HeaderValue::from_str(o).ok())))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([header::CONTENT_TYPE])
        // Chrome asks first whether a public site may reach a private address; this says yes (only for the origins above).
        .allow_private_network(true);

    let local = Arc::new(Local { agent, origins });
    Router::new()
        .route("/", get(setup_page))
        .route("/setup/pair", post(pair))
        .route("/status", get(status))
        .route("/printers", get(list_printers))
        .route("/config", get(config))
        .route("/print", post(print))
        .layer(middleware::from_fn_with_state(local.clone(), guard))
        .layer(cors)
        .with_state(local)
}

/// Refuses a request from any page that is not Restora, and any whose Host is not this address (DNS rebinding).
async fn guard(State(local): State<Shared>, request: Request, next: Next) -> Response {
    let host_ok = request.headers().get(header::HOST).and_then(|h| h.to_str().ok()) == Some(ADDRESS);
    let origin_ok = match request.headers().get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
        Some(origin) => local.origins.iter().any(|allowed| allowed == origin),
        None => true,
    };
    if !host_ok || !origin_ok {
        return (StatusCode::FORBIDDEN, Json(json!({ "message": "Not allowed" }))).into_response();
    }
    next.run(request).await
}

/// Where staff type the pairing code from Settings. Served by the agent itself, so no browser permission is needed.
async fn setup_page() -> Html<&'static str> {
    Html(include_str!("setup.html"))
}

async fn pair(State(local): State<Shared>, Json(request): Json<PairRequest>) -> Response {
    match local.agent.pair(request.code.trim()).await {
        Ok(()) => Json(json!({ "paired": true })).into_response(),
        Err(message) => (StatusCode::BAD_REQUEST, Json(json!({ "message": message }))).into_response(),
    }
}

async fn status(State(local): State<Shared>) -> Json<serde_json::Value> {
    Json(json!({ "paired": local.agent.paired(), "version": env!("CARGO_PKG_VERSION") }))
}

/// The printers as staff set them in Restora (name, what each prints, paper), for the web app when it has no internet.
async fn config(State(local): State<Shared>) -> Json<serde_json::Value> {
    Json(json!({ "paired": local.agent.paired(), "printers": local.agent.printers() }))
}

async fn list_printers() -> Response {
    match printers::list() {
        Ok(names) => Json(json!(names)).into_response(),
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "message": message }))).into_response(),
    }
}

async fn print(State(local): State<Shared>, Json(request): Json<PrintRequest>) -> Response {
    let agent = local.agent.clone();
    // The blocking Windows calls stay off the async threads.
    let result = tokio::task::spawn_blocking(move || {
        let slip = Slip { printer: &request.printer, kind: &request.kind, order_key: &request.order_key, width: request.width.unwrap_or(48).clamp(24, 64), copies: request.copies.unwrap_or(1), order: &request.order, header: &request.header };
        agent.print(&slip)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(printed) => Json(json!({ "printed": printed, "duplicate": !printed })).into_response(),
        Err(message) => (StatusCode::BAD_GATEWAY, Json(json!({ "message": message }))).into_response(),
    }
}
