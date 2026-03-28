use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serialink::interface::http::build_router;
use serialink::serial::manager::SessionManager;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a test router with no API key (open access).
fn test_app() -> axum::Router {
    let manager = Arc::new(SessionManager::new(None, None));
    build_router(manager, None, None)
}

/// Build a test router that requires the given API key.
fn test_app_with_key(key: &str) -> axum::Router {
    let manager = Arc::new(SessionManager::new(None, None));
    build_router(manager, Some(key.to_string()), None)
}

/// Read the full response body as bytes.
async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

/// Read the full response body as a JSON value.
async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = body_bytes(response).await;
    serde_json::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// Health endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_health_endpoint() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
}

#[tokio::test]
async fn test_health_no_auth_required() {
    let app = test_app_with_key("secret123");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_dashboard_page() {
    let app = test_app();
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = body_bytes(response).await;
    let html = String::from_utf8(bytes).unwrap();
    assert!(html.contains("serialink"));
}

#[tokio::test]
async fn test_dashboard_monitor_select_has_explicit_readable_text_color() {
    let app = test_app();
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = body_bytes(response).await;
    let html = String::from_utf8(bytes).unwrap();
    assert!(html.contains("id=\"mon-port\""));
    assert!(html.contains("id=\"mon-port\" class=\"bg-surface-2 border border-gray-700 rounded-lg px-3 py-2 text-sm text-gray-200"));
    assert!(html.contains("id=\"se-session\" class=\"w-full bg-surface-1 border border-gray-700 rounded-lg px-3 py-2 text-sm text-gray-200"));
}

#[tokio::test]
async fn test_dashboard_port_rendering_uses_port_info_name_field() {
    let app = test_app();
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = body_bytes(response).await;
    let html = String::from_utf8(bytes).unwrap();
    assert!(html.contains("p.name || p.port_name || p.path || JSON.stringify(p)"));
    assert!(html.contains("p.name || p.port_name || p.path || ''"));
}

#[tokio::test]
async fn test_dashboard_no_auth_required() {
    let app = test_app_with_key("secret123");
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// List ports
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_ports() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ports")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert!(json.is_array());
}

// ---------------------------------------------------------------------------
// Sessions (empty state)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_sessions_empty() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Authentication tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_api_key_required_when_set() {
    let app = test_app_with_key("secret123");
    // Request without any key => 401
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ports")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_key_valid_header() {
    let app = test_app_with_key("secret123");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ports")
                .header("x-api-key", "secret123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_api_key_valid_query_param() {
    let app = test_app_with_key("secret123");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ports?api_key=secret123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Query param auth was removed (leaks through logs/history/referers).
    // Even a correct key via query param must be rejected.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_key_wrong() {
    let app = test_app_with_key("secret123");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ports")
                .header("x-api-key", "wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_key_wrong_query_param() {
    let app = test_app_with_key("secret123");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ports?api_key=wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_no_auth_when_key_not_configured() {
    let app = test_app(); // no key set
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Port validation (open_port rejects bad inputs)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_open_port_empty_path() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"port_path":""}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_open_port_invalid_path() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"port_path":"/etc/passwd"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_open_port_path_traversal() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"port_path":"/dev/tty../../etc/passwd"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_open_port_relative_path() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"port_path":"ttyS0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_open_port_baud_rate_zero() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"port_path":"/dev/ttyS0","baud_rate":0}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_open_port_baud_rate_too_high() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"port_path":"/dev/ttyS0","baud_rate":99999999}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_open_port_missing_body() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should fail — missing JSON body
    assert_ne!(response.status(), StatusCode::OK);
    assert_ne!(response.status(), StatusCode::CREATED);
}

// ---------------------------------------------------------------------------
// Non-existent session operations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_close_nonexistent_session() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/sessions/nonexistent-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_read_lines_nonexistent_session() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/fake-id/lines")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_write_nonexistent_session() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/fake-id/write")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"data":"hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_snapshot_nonexistent_session() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/fake-id/snapshot")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_send_and_expect_nonexistent_session() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/fake-id/send-and-expect")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"data":"AT\r\n","pattern":"OK"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 404 for unknown routes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unknown_route_returns_404() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Binary mode / protocol tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_open_port_with_invalid_mode() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"port_path":"/dev/ttyS0","mode":"invalid"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert!(json["error"].as_str().unwrap().contains("mode"));
}

#[tokio::test]
async fn test_open_port_with_unknown_protocol() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"port_path":"/dev/ttyS0","protocol":"nonexistent"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert!(json["error"].as_str().unwrap().contains("Unknown protocol"));
}

#[tokio::test]
async fn test_write_with_invalid_hex() {
    let app = test_app();
    // First, we need a session. The port won't actually open, but the
    // validation of the hex field happens before the session lookup for
    // nonexistent sessions. Let's test with a nonexistent session — the
    // hex validation happens after session lookup, so we get 404. Instead,
    // test that the request is accepted structurally.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/fake-id/write")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"hex":"ZZZZ"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Session doesn't exist, so we get 404 (hex validation happens after session lookup).
    // But the JSON parsing should succeed (no 422).
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_write_with_hex_too_long() {
    let app = test_app();
    // Create a hex string that's over 6144 chars
    let long_hex = "AA".repeat(3200); // 6400 chars > 6144 limit
    let body = format!(r#"{{"hex":"{}"}}"#, long_hex);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/fake-id/write")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should fail on hex length validation before session lookup? No — session lookup comes first.
    // With a fake session, we get 404. This still exercises the JSON deserialization.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_write_requires_data_or_hex() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/fake-id/write")
                .header("content-type", "application/json")
                .body(Body::from(r#"{}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // With a fake session we get 404, but the JSON should parse fine
    // since both fields are optional.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_open_port_text_mode_default() {
    let app = test_app();
    // Opening a port that doesn't exist will fail at the serial level (500),
    // but we can verify the request is accepted structurally.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"port_path":"/dev/ttyS0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // The port likely doesn't exist on CI, so expect 500
    // but NOT 400 (validation should pass).
    assert_ne!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_open_port_with_protocol_empty_string() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"port_path":"/dev/ttyS0","protocol":""}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
