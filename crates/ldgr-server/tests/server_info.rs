//! In-process integration tests for the public server-discovery endpoints
//! (#177): `GET /api/v1/server/info` and `GET /api/v1/server/ping`. Both are
//! unauthenticated. Requests are dispatched straight into the axum router via
//! `tower`'s `oneshot` — no sockets bound.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use ldgr_server::config::{Config, RegistrationPolicy};
use ldgr_server::{PROTOCOL_VERSION, api, auth, settings, state, storage};

fn config_with(policy: RegistrationPolicy, server_name: &str) -> Config {
    Config {
        bind_addr: "127.0.0.1:8080".parse().unwrap(),
        db_path: ":memory:".into(),
        session_ttl_hours: 720,
        relay_ttl_minutes: 10,
        max_blob_bytes: 52_428_800,
        srp_handshake_ttl_secs: 120,
        registration_policy: policy,
        admin_email: None,
        default_user_quota_bytes: 1_073_741_824,
        server_name: server_name.into(),
    }
}

struct Harness {
    state: state::SharedState,
}

impl Harness {
    fn with_config(config: Config) -> Self {
        let db = storage::ServerDb::open(":memory:").expect("open in-memory db");
        let srp_ttl = std::time::Duration::from_secs(config.srp_handshake_ttl_secs);
        let app_state = Arc::new(state::AppState {
            db,
            srp_handshakes: auth::srp::SrpHandshakeStore::new(srp_ttl),
            config,
        });
        Self { state: app_state }
    }

    fn router(&self) -> Router {
        api::router(self.state.clone())
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = self.router().oneshot(req).await.expect("router oneshot");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }
}

#[tokio::test]
async fn server_info_reports_version_protocol_and_capabilities() {
    let h = Harness::with_config(config_with(RegistrationPolicy::Open, "my-server"));
    let (status, body) = h.get("/api/v1/server/info").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "my-server");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(body["min_protocol_version"], PROTOCOL_VERSION);
    assert_eq!(body["max_protocol_version"], PROTOCOL_VERSION);
    assert_eq!(body["registration_policy"], "open");
    assert_eq!(body["public_registration"], true);
    // 2SKD has been supported since #172.
    assert_eq!(body["two_secret_auth"], true);
}

#[tokio::test]
async fn server_info_public_registration_false_when_not_open() {
    let h = Harness::with_config(config_with(RegistrationPolicy::InviteOnly, "ldgr-server"));
    let (status, body) = h.get("/api/v1/server/info").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["registration_policy"], "invite-only");
    assert_eq!(body["public_registration"], false);
}

#[tokio::test]
async fn server_info_reflects_persisted_registration_policy() {
    // Env/config bootstraps as `open`, but a persisted override must win — and
    // the advertised policy must match what `register` actually enforces.
    let h = Harness::with_config(config_with(RegistrationPolicy::Open, "ldgr-server"));

    let (_, body) = h.get("/api/v1/server/info").await;
    assert_eq!(body["registration_policy"], "open");
    assert_eq!(body["public_registration"], true);

    h.state
        .db
        .set_setting(settings::KEY_REGISTRATION_POLICY, "admin-only")
        .await
        .expect("persist setting");

    let (status, body) = h.get("/api/v1/server/info").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["registration_policy"], "admin-only");
    assert_eq!(body["public_registration"], false);
}

#[tokio::test]
async fn server_ping_is_a_cheap_liveness_probe() {
    let h = Harness::with_config(config_with(RegistrationPolicy::InviteOnly, "ping-me"));
    let (status, body) = h.get("/api/v1/server/ping").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["pong"], true);
    assert_eq!(body["name"], "ping-me");
    assert_eq!(body["protocol_version"], PROTOCOL_VERSION);
}

#[tokio::test]
async fn discovery_endpoints_need_no_auth() {
    let h = Harness::with_config(config_with(RegistrationPolicy::InviteOnly, "ldgr-server"));
    // No Authorization header is set on either request.
    let (info_status, _) = h.get("/api/v1/server/info").await;
    let (ping_status, _) = h.get("/api/v1/server/ping").await;
    assert_eq!(info_status, StatusCode::OK);
    assert_eq!(ping_status, StatusCode::OK);
}

#[tokio::test]
async fn health_endpoint_still_works() {
    let h = Harness::with_config(config_with(RegistrationPolicy::Open, "ldgr-server"));
    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = h.router().oneshot(req).await.expect("router oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert_eq!(&bytes[..], b"ok");
}
