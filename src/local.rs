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

/// The customer-facing web app. Overrides are for local development only.
pub fn app_url() -> String {
    std::env::var("RESTORA_APP")
        .ok()
        .filter(|url| url.starts_with("https://") || url.starts_with("http://localhost:"))
        .unwrap_or_else(|| "https://app.restora.live".to_string())
        .trim_end_matches('/')
        .to_string()
}

pub fn printer_settings_url() -> String {
    format!("{}/settings/printers", app_url())
}

/// Opens the setup page in the browser, so staff can type the pairing code. Elsewhere (development) it only logs where it is.
pub fn open_setup() {
    let url = format!("http://{ADDRESS}");
    #[cfg(windows)]
    crate::tray::open(&url);
    #[cfg(not(windows))]
    tracing::info!("set this agent up at {url}");
}

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
    /// The web app's selected restaurant branch; must match the paired agent.
    branch_id: Option<String>,
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
    /// Sent by the web app: the API it uses, so the dev site pairs with the dev API.
    api: Option<String>,
}

/// `origins` are the pages allowed to call the agent (the Restora web app); the agent's own setup page is always one.
pub fn router(agent: Arc<Agent>, mut origins: Vec<String>) -> Router {
    origins.push(format!("http://{ADDRESS}"));
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()),
        ))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([header::CONTENT_TYPE])
        // Current Chrome/Edge Local Network Access uses the loopback-network permission
        // and standard CORS; the old Private Network Access response header is not needed.

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

/// Refuses non-Restora browser requests, DNS rebinding, and originless writes.
/// An Origin header is required for POST: CORS alone never authorizes a request.
async fn guard(State(local): State<Shared>, request: Request, next: Next) -> Response {
    let host_ok = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        == Some(ADDRESS);
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|o| o.to_str().ok());
    let origin_ok =
        origin.is_none_or(|origin| local.origins.iter().any(|allowed| allowed == origin));
    let writable = request.method() != axum::http::Method::GET
        && request.method() != axum::http::Method::HEAD
        && request.method() != axum::http::Method::OPTIONS;
    if !host_ok || !origin_ok || (writable && origin.is_none()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "message": "Not allowed" })),
        )
            .into_response();
    }
    next.run(request).await
}

/// Where staff connect this PC: a button that opens Restora (which pairs it), or the pairing code typed in. Served by
/// the agent itself, so no browser permission is needed. `RESTORA_APP` replaces the web app's address.
async fn setup_page() -> Html<String> {
    let page = include_str!("setup.html");
    Html(page.replace("https://app.restora.live", &app_url()))
}

async fn pair(State(local): State<Shared>, Json(request): Json<PairRequest>) -> Response {
    match local
        .agent
        .pair(request.code.trim(), request.api.as_deref())
        .await
    {
        Ok(()) => Json(json!({ "paired": true })).into_response(),
        Err(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "message": message }))).into_response()
        }
    }
}

/// Everything the setup page shows: connection, printers and the recent slips (see status.rs).
async fn status(State(local): State<Shared>) -> Json<crate::status::Snapshot> {
    let agent = local.agent.clone();
    Json(
        tokio::task::spawn_blocking(move || crate::status::snapshot(&agent))
            .await
            .unwrap(),
    )
}

/// The printers as staff set them in Restora (name, what each prints, paper), for the web app when it has no internet.
async fn config(State(local): State<Shared>) -> Json<serde_json::Value> {
    let branch = local
        .agent
        .config
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|config| config.branch_id.clone());
    Json(
        json!({ "paired": local.agent.paired(), "branchId": branch, "printers": local.agent.printers() }),
    )
}

async fn list_printers() -> Response {
    match printers::list() {
        Ok(names) => Json(json!(names)).into_response(),
        Err(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "message": message })),
        )
            .into_response(),
    }
}

fn branch_allowed(paired_branch: Option<&str>, selected_branch: Option<&str>) -> bool {
    paired_branch.is_none_or(|branch| selected_branch == Some(branch))
}

async fn print(State(local): State<Shared>, Json(mut request): Json<PrintRequest>) -> Response {
    let agent = local.agent.clone();
    if !agent.paired() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "message": "Connect this agent to Restora first." })),
        )
            .into_response();
    }
    let paired_branch = agent
        .config
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|config| config.branch_id.clone());
    if !branch_allowed(paired_branch.as_deref(), request.branch_id.as_deref()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "message": "This PC belongs to a different Restora branch." })),
        )
            .into_response();
    }
    if request.order_key.trim().is_empty()
        || request.order_key.len() > 200
        || request.printer.len() > 200
        || !matches!(request.kind.as_str(), "kitchen" | "receipt")
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "message": "Invalid print job." })),
        )
            .into_response();
    }
    let printers = agent.printers();
    let allowed = printers.as_array().is_some_and(|rows| {
        rows.iter().any(|printer| {
            printer["systemName"] == request.printer
                && printer["present"] == true
                && printer["prints"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == &request.kind))
        })
    });
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "message": "This printer is not configured for this slip in Restora." })),
        )
            .into_response();
    }
    if request.kind == "receipt" {
        request.header.logo_file = crate::logo::load(request.header.logo.as_deref()).await;
    }
    // The blocking Windows calls stay off the async threads.
    let result = tokio::task::spawn_blocking(move || {
        let slip = Slip {
            printer: &request.printer,
            kind: &request.kind,
            order_key: &request.order_key,
            width: request.width.unwrap_or(48).clamp(24, 64),
            copies: request.copies.unwrap_or(1),
            order: &request.order,
            header: &request.header,
        };
        agent.print(&slip)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(printed) => Json(json!({ "printed": printed, "duplicate": !printed })).into_response(),
        Err(message) => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "message": message }))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::branch_allowed;

    #[test]
    fn a_paired_agent_never_prints_for_another_branch() {
        assert!(branch_allowed(Some("branch-a"), Some("branch-a")));
        assert!(!branch_allowed(Some("branch-a"), Some("branch-b")));
        assert!(!branch_allowed(Some("branch-a"), None));
        // Older installations have no branch until they reconnect.
        assert!(branch_allowed(None, None));
    }
}
