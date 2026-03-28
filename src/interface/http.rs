use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};

const DASHBOARD_HTML: &str = include_str!("../../web/index.html");
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::protocol::format;
use crate::protocol::presets;
use crate::serial::discovery;
use crate::serial::manager::SessionManager;
use crate::serial::port::PortConfig;
use crate::serial::validate_port_path;

// --- App state ---

#[derive(Clone)]
struct AppState {
    manager: Arc<SessionManager>,
    api_key: Option<String>,
}

// --- Auth middleware ---

async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    if let Some(ref expected_key) = state.api_key {
        // Only accept API key via X-API-Key header (not query params,
        // which leak through logs, browser history, and referer headers).
        let provided = req.headers().get("x-api-key").and_then(|v| v.to_str().ok());

        if provided != Some(expected_key.as_str()) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Invalid or missing API key. Use X-API-Key header."})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

// --- Request / response types ---

#[derive(Deserialize)]
struct OpenPortRequest {
    port_path: String,
    baud_rate: Option<u32>,
    mode: Option<String>,
    protocol: Option<String>,
}

#[derive(Deserialize)]
struct WriteDataRequest {
    data: Option<String>,
    hex: Option<String>,
}

#[derive(Deserialize)]
struct SendAndExpectRequest {
    data: String,
    pattern: String,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct ReadLinesQuery {
    count: Option<usize>,
}

#[derive(Deserialize)]
struct SnapshotQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

// --- Helpers ---

/// Format a line as JSON, using binary format for binary-mode lines.
fn format_line_json(l: &crate::serial::port::TimestampedLine) -> serde_json::Value {
    if format::is_binary_line(l) {
        format::format_binary_line(l)
    } else {
        let mut obj = json!({
            "timestamp": l.timestamp.to_rfc3339(),
            "content": l.content,
        });
        if !l.metadata.is_empty() {
            obj["metadata"] = json!(l.metadata);
        }
        obj
    }
}

// --- Handlers ---

async fn dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn list_ports() -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let ports = discovery::list_ports().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;
    Ok(Json(json!(ports)))
}

async fn open_port(
    State(state): State<AppState>,
    Json(req): Json<OpenPortRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    validate_port_path(&req.port_path)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e}))))?;

    let mut config = PortConfig::default();
    if let Some(baud) = req.baud_rate {
        if baud == 0 || baud > 3_000_000 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "baud_rate must be between 1 and 3000000"})),
            ));
        }
        config.baud_rate = baud;
    }

    // Resolve protocol preset.
    let protocol_override = if let Some(ref name) = req.protocol {
        if name.is_empty() || name.len() > 128 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "protocol name must be 1-128 characters"})),
            ));
        }
        let preset = presets::resolve_preset(name).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Unknown protocol preset. Available: modbus_rtu, modbus_ascii"})),
            )
        })?;
        Some(preset)
    } else {
        None
    };

    // Validate mode.
    let mode_str = req.mode.as_deref().unwrap_or("text");
    let effective_mode = if protocol_override.is_some() {
        "binary"
    } else {
        mode_str
    };
    match effective_mode {
        "text" | "binary" => {}
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "mode must be 'text' or 'binary'"})),
            ));
        }
    }

    let session_id = state
        .manager
        .create_session(req.port_path.clone(), config, protocol_override)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "session_id": session_id,
            "port_path": req.port_path,
            "mode": effective_mode,
            "status": "connected"
        })),
    ))
}

async fn list_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    let sessions = state.manager.list_sessions().await;
    Json(json!(sessions))
}

async fn read_lines(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ReadLinesQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let count = params.count.unwrap_or(50).min(1000);

    let conn = state.manager.get_session(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Session not found", "session_id": id})),
        )
    })?;

    let lines = conn.get_recent_lines(count).await;
    let output: Vec<serde_json::Value> = lines.iter().map(format_line_json).collect();

    Ok(Json(json!(output)))
}

async fn write_data(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<WriteDataRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let conn = state.manager.get_session(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Session not found", "session_id": id})),
        )
    })?;

    let (bytes, byte_count) = if let Some(ref hex_str) = req.hex {
        if hex_str.len() > 6144 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Hex string too long (max 6144 chars)"})),
            ));
        }
        let parsed = format::parse_hex(hex_str).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid hex: {}", e)})),
            )
        })?;
        let len = parsed.len();
        (parsed, len)
    } else if let Some(ref data) = req.data {
        let len = data.len();
        (data.as_bytes().to_vec(), len)
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Either 'data' or 'hex' must be provided"})),
        ));
    };

    conn.write_data(&bytes).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;

    Ok(Json(json!({"status": "written", "bytes": byte_count})))
}

async fn send_and_expect(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SendAndExpectRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let timeout_ms = req.timeout_ms.unwrap_or(5000).min(30_000);

    if req.pattern.len() > 1024 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Regex pattern too long (max 1024 chars)"})),
        ));
    }

    let re = regex::RegexBuilder::new(&req.pattern)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid regex pattern: {}", e)})),
            )
        })?;

    let conn = state.manager.get_session(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Session not found", "session_id": id})),
        )
    })?;

    // Subscribe before writing so we don't miss output.
    let mut rx = conn.subscribe();

    conn.write_data(req.data.as_bytes()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;

    let mut collected_lines: Vec<String> = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(Json(json!({
                "matched": false,
                "error": "timeout",
                "message": format!("Pattern '{}' not matched within {}ms", req.pattern, timeout_ms),
                "collected_lines": collected_lines,
            })));
        }

        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(line)) => {
                let matchable = format::matchable_content(&line);
                let matched = re.is_match(matchable);
                if collected_lines.len() < 200 {
                    collected_lines.push(line.content.clone());
                }
                if matched {
                    return Ok(Json(json!({
                        "matched": true,
                        "matched_line": line.content,
                        "collected_lines": collected_lines,
                    })));
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                if collected_lines.len() < 200 {
                    collected_lines.push(format!("[dropped {} messages due to lag]", n));
                }
                continue;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                return Ok(Json(json!({
                    "matched": false,
                    "error": "channel_closed",
                    "collected_lines": collected_lines,
                })));
            }
            Err(_) => {
                return Ok(Json(json!({
                    "matched": false,
                    "error": "timeout",
                    "message": format!("Pattern '{}' not matched within {}ms", req.pattern, timeout_ms),
                    "collected_lines": collected_lines,
                })));
            }
        }
    }
}

async fn snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<SnapshotQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let limit = params.limit.unwrap_or(500).min(5000);

    let conn = state.manager.get_session(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Session not found", "session_id": id})),
        )
    })?;

    let lines = conn.get_recent_lines(limit).await;
    let output: Vec<serde_json::Value> = lines.iter().map(format_line_json).collect();

    Ok(Json(json!({
        "session_id": id,
        "line_count": output.len(),
        "lines": output,
    })))
}

async fn close_port(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    state
        .manager
        .close_session(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({"status": "closed", "session_id": id})))
}

// --- Router builder (public for integration tests) ---

/// Build the axum [`Router`] with all routes, middleware, and state.
///
/// Extracted so that integration tests can drive the router via
/// `tower::ServiceExt::oneshot` without binding to a TCP port.
pub fn build_router(
    manager: Arc<SessionManager>,
    api_key: Option<String>,
    bind: Option<std::net::SocketAddr>,
) -> Router {
    let state = AppState {
        manager,
        api_key: api_key.clone(),
    };

    // Always restrict CORS to the server's own origin. Even in local dev
    // mode without auth, permissive CORS would let any website script a
    // running serialink instance.
    let origin = if let Some(addr) = bind {
        format!("http://{}", addr)
    } else {
        "http://127.0.0.1:8600".to_string()
    };
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::exact(
            origin.parse().unwrap(),
        ))
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::HeaderName::from_static("x-api-key"),
        ]);

    let api_routes = Router::new()
        .route("/api/ports", get(list_ports))
        .route("/api/sessions", post(open_port))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}/lines", get(read_lines))
        .route("/api/sessions/{id}/write", post(write_data))
        .route("/api/sessions/{id}/send-and-expect", post(send_and_expect))
        .route("/api/sessions/{id}/snapshot", get(snapshot))
        .route("/api/sessions/{id}", delete(close_port))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .route("/", get(dashboard))
        .route("/health", get(health))
        .merge(api_routes)
        .layer(cors)
        .with_state(state)
}

// --- Server entrypoint ---

pub async fn run_http_server(
    manager: Arc<SessionManager>,
    bind: std::net::SocketAddr,
    api_key: Option<String>,
) -> anyhow::Result<()> {
    let app = build_router(manager.clone(), api_key.clone(), Some(bind));

    let auth_mode = if api_key.is_some() {
        "API key required"
    } else {
        "no authentication (local dev mode)"
    };

    tracing::info!("Starting HTTP server on {}", bind);
    eprintln!("serialink v{} HTTP API server", env!("CARGO_PKG_VERSION"));
    eprintln!("  Listening: http://{}", bind);
    eprintln!("  Auth: {}", auth_mode);
    eprintln!("  Health: http://{}/health", bind);
    eprintln!("  Dashboard: http://{}/", bind);
    eprintln!("  API:    http://{}/api/...", bind);

    let listener = tokio::net::TcpListener::bind(bind).await?;

    let manager_shutdown = manager.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\nShutting down HTTP server...");
            manager_shutdown.close_all().await;
            eprintln!("All sessions closed.");
        })
        .await?;

    Ok(())
}
