use crate::{printers, slips};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tower_http::cors::{AllowOrigin, CorsLayer};

pub const ADDRESS: &str = "127.0.0.1:17777";

struct Local {
    origins: Vec<String>,
    /// `orderKey|kind|printer` of slips printed since the agent started. The same slip is never printed twice.
    printed: Mutex<HashSet<String>>,
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
    order: slips::Order,
    #[serde(default)]
    header: slips::Header,
}

pub fn router(origins: Vec<String>) -> Router {
    let shared: Shared = Arc::new(Local {
        origins: origins.clone(),
        printed: Mutex::default(),
    });
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()),
        ))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([header::CONTENT_TYPE])
        // Chrome asks first whether a public site may reach a private address; this says yes (only for the origins above).
        .allow_private_network(true);

    Router::new()
        .route("/status", get(status))
        .route("/printers", get(list_printers))
        .route("/print", post(print))
        .layer(middleware::from_fn_with_state(shared.clone(), guard))
        .layer(cors)
        .with_state(shared)
}

/// Refuses a request from any page that is not Restora, and any whose Host is not this address (DNS rebinding).
async fn guard(State(local): State<Shared>, request: Request, next: Next) -> Response {
    let host_ok = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        == Some(ADDRESS);
    let origin_ok = match request
        .headers()
        .get(header::ORIGIN)
        .and_then(|o| o.to_str().ok())
    {
        Some(origin) => local.origins.iter().any(|allowed| allowed == origin),
        None => true,
    };
    if !host_ok || !origin_ok {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "message": "Not allowed" })),
        )
            .into_response();
    }
    next.run(request).await
}

async fn status() -> Json<serde_json::Value> {
    Json(json!({ "paired": false, "version": env!("CARGO_PKG_VERSION") }))
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

async fn print(State(local): State<Shared>, Json(request): Json<PrintRequest>) -> Response {
    let width = request.width.unwrap_or(48).clamp(24, 64);
    let bytes = match request.kind.as_str() {
        "kitchen" => slips::kitchen(&request.order, width),
        "receipt" => slips::receipt(&request.order, &request.header, width),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "message": "kind must be kitchen or receipt" })),
            )
                .into_response();
        }
    };
    let key = format!("{}|{}|{}", request.order_key, request.kind, request.printer);
    if local.printed.lock().unwrap().contains(&key) {
        return Json(json!({ "printed": false, "duplicate": true })).into_response();
    }
    // The blocking Windows calls stay off the async threads.
    let printer = request.printer.clone();
    let result = tokio::task::spawn_blocking(move || printers::print_raw(&printer, &bytes))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(()) => {
            local.printed.lock().unwrap().insert(key);
            Json(json!({ "printed": true, "duplicate": false })).into_response()
        }
        Err(message) => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "message": message }))).into_response()
        }
    }
}
