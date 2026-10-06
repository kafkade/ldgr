//! Tests for the **scoped CORS layer** (issue #297, finding F3).
//!
//! The admin/web panel (`apps/web`, a static export) is often served from a
//! different origin than the API, so the server must return CORS headers — but
//! **only** for the operator-configured allowlist (`LDGR_ALLOWED_ORIGINS`), and
//! nothing at all by default. These tests drive a minimal router with the layer
//! attached via `tower`'s `oneshot`.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use tower::ServiceExt;

use ldgr_server::cors::cors_layer;

/// A tiny router with the CORS layer applied exactly as `main.rs` does: attached
/// only when `cors_layer` yields a layer for the given allowlist.
fn app(allowed_origins: &[String]) -> Router {
    let base = Router::new().route("/health", get(|| async { "ok" }));
    match cors_layer(allowed_origins) {
        Some(cors) => base.layer(cors),
        None => base,
    }
}

#[test]
fn empty_or_invalid_allowlist_disables_cors() {
    assert!(cors_layer(&[]).is_none(), "no origins => no CORS layer");
    assert!(
        cors_layer(&["not a\nvalid header".to_string()]).is_none(),
        "an all-invalid allowlist yields no layer"
    );
}

#[tokio::test]
async fn preflight_allows_configured_origin() {
    let origin = "https://admin.example.com";
    let resp = app(&[origin.to_string()])
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/health")
                .header(header::ORIGIN, origin)
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let acao = resp
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|v| v.to_str().ok());
    assert_eq!(acao, Some(origin));
}

#[tokio::test]
async fn preflight_rejects_unlisted_origin() {
    let resp = app(&["https://admin.example.com".to_string()])
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/health")
                .header(header::ORIGIN, "https://evil.example.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "an unlisted origin must not receive an Access-Control-Allow-Origin header"
    );
}

#[tokio::test]
async fn simple_request_carries_cors_header_for_allowed_origin() {
    let origin = "https://admin.example.com";
    let resp = app(&[origin.to_string()])
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .header(header::ORIGIN, origin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let acao = resp
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|v| v.to_str().ok());
    assert_eq!(acao, Some(origin));
}

#[tokio::test]
async fn no_cors_headers_when_allowlist_empty() {
    let origin = "https://admin.example.com";
    let resp = app(&[])
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .header(header::ORIGIN, origin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "with no allowlist the server must not emit CORS headers"
    );
}
