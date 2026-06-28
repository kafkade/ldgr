//! Shared in-process test harness for the `ldgr-server` integration suite.
//!
//! [`RouterSender`] implements [`RawHttpSender`] by dispatching each
//! transport-agnostic `RawRequest` straight into the **real** axum router via
//! `tower`'s [`oneshot`](tower::ServiceExt::oneshot) — no sockets are bound and
//! no HTTP client is involved. The sender holds the [`SharedState`] so every
//! `send` rebuilds the router against the **same** state, letting auth tokens
//! and stored blobs persist across calls.
//!
//! This module is the single source of truth for the harness so the
//! transport+auth tests (`server_sync_client_e2e.rs`) and the real
//! compose/apply pipeline tests (`sync_pipeline_e2e.rs`) cannot drift apart.

// Each integration-test binary that declares `mod common;` uses a different
// subset of these helpers; suppress dead-code warnings for the unused remainder.
#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use ldgr_core::crypto::{Argon2Params, AuthKey, derive_auth_key, derive_master_key};
use ldgr_core::sync::server::{
    RawHttpSender, RawRequest, RawResponse, ServerSyncClient, ServerSyncError,
};
use ldgr_server::config::{Config, RegistrationPolicy};
use ldgr_server::{api, auth, state, storage};

/// Build the in-memory server [`Config`] used by every integration test.
pub fn open_config() -> Config {
    Config {
        bind_addr: "127.0.0.1:8080".parse().unwrap(),
        db_path: ":memory:".into(),
        session_ttl_hours: 720,
        relay_ttl_minutes: 10,
        max_blob_bytes: 52_428_800,
        srp_handshake_ttl_secs: 120,
        registration_policy: RegistrationPolicy::Open,
        admin_email: None,
        default_user_quota_bytes: 1_073_741_824,
        server_name: "e2e-server".into(),
    }
}

/// A [`RawHttpSender`] that dispatches each request into the axum router built
/// from a shared [`AppState`](state::AppState). Cloning the state per call means
/// every request hits the same DB and SRP handshake store.
#[derive(Clone)]
pub struct RouterSender {
    state: state::SharedState,
}

impl RouterSender {
    /// Boot a fresh in-memory server (empty DB, fresh SRP handshake store).
    pub fn new() -> Self {
        let db = storage::ServerDb::open(":memory:").expect("open in-memory db");
        let config = open_config();
        let srp_ttl = std::time::Duration::from_secs(config.srp_handshake_ttl_secs);
        let app_state = Arc::new(state::AppState {
            db,
            srp_handshakes: auth::srp::SrpHandshakeStore::new(srp_ttl),
            config,
        });
        Self { state: app_state }
    }
}

impl Default for RouterSender {
    fn default() -> Self {
        Self::new()
    }
}

impl RawHttpSender for RouterSender {
    async fn send(&self, request: RawRequest) -> Result<RawResponse, ServerSyncError> {
        // Build the request URI: path plus an optional query string.
        let mut uri = request.path.clone();
        if !request.query.is_empty() {
            let qs: Vec<String> = request
                .query
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            uri.push('?');
            uri.push_str(&qs.join("&"));
        }

        let mut builder = Request::builder().method(request.method.as_str()).uri(uri);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let req = builder
            .body(Body::from(request.body))
            .map_err(|e| ServerSyncError::Transport(e.to_string()))?;

        // Rebuild the router against the shared state so token/session/blob
        // state persists between calls.
        let router = api::router(self.state.clone());
        let resp = router
            .oneshot(req)
            .await
            .map_err(|e| ServerSyncError::Transport(e.to_string()))?;

        let status = resp.status().as_u16();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .map_err(|e| ServerSyncError::Transport(e.to_string()))?
            .to_vec();
        Ok(RawResponse { status, body })
    }
}

/// A [`ServerSyncClient`] wired to a freshly booted in-process server.
pub fn client() -> ServerSyncClient<RouterSender> {
    ServerSyncClient::new(RouterSender::new())
}

/// Derive the master auth key (`MK_auth`) from a password, as a client would.
pub fn auth_key(password: &[u8]) -> AuthKey {
    let mk = derive_master_key(password, b"argon-salt-16byte", &Argon2Params::test())
        .expect("derive master key");
    derive_auth_key(&mk).expect("derive auth key")
}
