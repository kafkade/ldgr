//! End-to-end, in-process integration tests for the core
//! [`ServerSyncClient`](ldgr_core::sync::server::ServerSyncClient) against the
//! real `ldgr-server` axum router.
//!
//! A [`RawHttpSender`] implementation bridges the transport-agnostic
//! `RawRequest`/`RawResponse` contract straight into the router via `tower`'s
//! `oneshot` — no sockets are bound and no HTTP client is involved. The harness
//! holds the [`SharedState`](state::SharedState) so every `send` rebuilds the
//! router against the **same** state, letting auth tokens and stored blobs
//! persist across calls.
//!
//! These tests exercise both auth schemes: legacy single-secret SRP and the
//! two-secret (2SKD, ADR-008) path. The server is a blind blob store, so blob
//! round-trips use arbitrary opaque bytes — no real vault encryption needed.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use ldgr_core::crypto::{Argon2Params, AuthKey, SecretKey, derive_auth_key, derive_master_key};
use ldgr_core::sync::server::{
    ListBatchesQuery, RawHttpSender, RawRequest, RawResponse, ServerSyncClient, ServerSyncError,
};
use ldgr_server::config::{Config, RegistrationPolicy};
use ldgr_server::{api, auth, state, storage};

// ── Harness ─────────────────────────────────────────────────────────────────────

fn open_config() -> Config {
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
struct RouterSender {
    state: state::SharedState,
}

impl RouterSender {
    fn new() -> Self {
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

fn client() -> ServerSyncClient<RouterSender> {
    ServerSyncClient::new(RouterSender::new())
}

/// Derive the master auth key (`MK_auth`) from a password, as a client would.
fn auth_key(password: &[u8]) -> AuthKey {
    let mk = derive_master_key(password, b"argon-salt-16byte", &Argon2Params::test())
        .expect("derive master key");
    derive_auth_key(&mk).expect("derive auth key")
}

const HELLO: &[u8] = b"\x00\x01\xfe\xff opaque ciphertext blob";

// ── Single-secret (legacy SRP) ──────────────────────────────────────────────────

#[tokio::test]
async fn single_secret_full_round_trip() {
    let mut c = client();
    let username = "alice";
    let password = b"correct horse battery staple";

    c.register(username, password).await.expect("register");
    c.login(username, password).await.expect("login");
    assert!(c.is_authenticated());

    let vault = "vault-1";
    c.create_vault(vault).await.expect("create vault");

    // Upload an encrypted batch and read it back byte-for-byte.
    let device = "device-a";
    let batch = "batch-0001";
    c.put_batch(vault, device, batch, HELLO)
        .await
        .expect("put batch");

    let listed = c
        .list_batches(vault, &ListBatchesQuery::default())
        .await
        .expect("list batches");
    assert_eq!(listed.entries.len(), 1);

    let remote = c
        .list_remote_batches(vault, &ListBatchesQuery::default())
        .await
        .expect("list remote batches");
    assert_eq!(remote.len(), 1);
    assert_eq!(remote[0].batch_id, batch);
    assert_eq!(remote[0].device_id, device);

    let fetched = c.get_batch(vault, device, batch).await.expect("get batch");
    assert_eq!(fetched, HELLO, "batch must round-trip byte-for-byte");

    // Snapshots round-trip too.
    let snapshot = "snap-0001";
    let snap_bytes = b"\xde\xad\xbe\xef snapshot blob".to_vec();
    c.put_snapshot(vault, snapshot, &snap_bytes)
        .await
        .expect("put snapshot");
    let fetched_snap = c.get_snapshot(vault, snapshot).await.expect("get snapshot");
    assert_eq!(
        fetched_snap, snap_bytes,
        "snapshot must round-trip byte-for-byte"
    );
}

#[tokio::test]
async fn login_with_wrong_password_is_rejected() {
    let mut c = client();
    c.register("bob", b"right-password")
        .await
        .expect("register");

    let result = c.login("bob", b"wrong-password").await;
    assert!(
        result.is_err(),
        "login with the wrong password must fail, got {result:?}"
    );
}

// ── Two-secret (2SKD, ADR-008) ──────────────────────────────────────────────────

#[tokio::test]
async fn two_secret_full_round_trip() {
    let mut c = client();
    let username = "carol";
    let account_id = uuid::Uuid::from_bytes([0x42; 16]);
    let mk_auth = auth_key(b"correct horse battery staple");
    let secret_key = SecretKey::generate(account_id);

    c.register_2skd(username, &account_id, &mk_auth, &secret_key)
        .await
        .expect("register 2skd");
    c.login_2skd(username, &account_id, &mk_auth, &secret_key)
        .await
        .expect("login 2skd");
    assert!(c.is_authenticated());

    // An authenticated call must succeed and a blob must round-trip.
    let vault = "vault-2skd";
    c.create_vault(vault).await.expect("create vault");

    let device = "device-2skd";
    let batch = "batch-2skd-1";
    c.put_batch(vault, device, batch, HELLO)
        .await
        .expect("put batch");
    let fetched = c.get_batch(vault, device, batch).await.expect("get batch");
    assert_eq!(fetched, HELLO, "2skd batch must round-trip byte-for-byte");
}

#[tokio::test]
async fn login_2skd_with_wrong_secret_key_is_rejected() {
    let mut c = client();
    let username = "dave";
    let account_id = uuid::Uuid::from_bytes([0x77; 16]);
    let mk_auth = auth_key(b"correct horse battery staple");
    let registered = SecretKey::generate(account_id);
    let attacker = SecretKey::generate(account_id); // correct password, wrong Secret Key

    c.register_2skd(username, &account_id, &mk_auth, &registered)
        .await
        .expect("register 2skd");

    let result = c
        .login_2skd(username, &account_id, &mk_auth, &attacker)
        .await;
    assert!(
        result.is_err(),
        "login with the wrong Secret Key must fail, got {result:?}"
    );
    assert!(!c.is_authenticated());
}

// ── Unauthenticated access ──────────────────────────────────────────────────────

#[tokio::test]
async fn unauthenticated_call_is_rejected() {
    let c = client();
    // No register/login: the client guards locally before sending.
    let result = c.create_vault("nope").await;
    assert_eq!(result.unwrap_err(), ServerSyncError::NotAuthenticated);
}
