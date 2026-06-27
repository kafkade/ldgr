//! End-to-end, in-process integration test for the **FFI** server-sync surface
//! (issue #200): drives `ldgr-ffi`'s [`LdgrSyncClient`] through its foreign
//! async-sender seam ([`FfiHttpSender`]) against the real `ldgr-server` axum
//! router via `tower`'s `oneshot` — no sockets, no real HTTP client.
//!
//! This is the FFI analogue of `server_sync_client_e2e.rs`: it proves the host
//! transport contract (the seam Swift satisfies over `URLSession` and web over
//! `fetch`) round-trips correctly and that auth (single-secret + 2SKD) and
//! opaque blob push/pull work end-to-end through the FFI boundary types.
//!
//! The server is a blind blob store, so blob round-trips use arbitrary opaque
//! bytes — exactly the ciphertext the (future #201) batch-blob pipeline will
//! produce; no real vault encryption is needed to exercise the transport.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use ldgr_ffi::{FfiArgon2Params, LdgrSyncClient};
use ldgr_ffi::{FfiHttpMethod, FfiHttpSender, FfiRawRequest, FfiRawResponse, FfiSyncError};
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
        server_name: "ffi-e2e-server".into(),
    }
}

/// An [`FfiHttpSender`] (the host I/O seam) that dispatches each request into the
/// axum router built from a shared [`AppState`](state::AppState). Cloning the
/// state per call means every request hits the same DB + SRP handshake store, so
/// tokens and stored blobs persist across calls — exactly like a real host
/// hitting one server.
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

#[async_trait::async_trait]
impl FfiHttpSender for RouterSender {
    async fn send(&self, request: FfiRawRequest) -> Result<FfiRawResponse, FfiSyncError> {
        // Build the request URI: path plus an optional query string.
        let mut uri = request.path.clone();
        if !request.query.is_empty() {
            let qs: Vec<String> = request
                .query
                .iter()
                .map(|kv| format!("{}={}", kv.name, kv.value))
                .collect();
            uri.push('?');
            uri.push_str(&qs.join("&"));
        }

        let method = match request.method {
            FfiHttpMethod::Get => "GET",
            FfiHttpMethod::Post => "POST",
            FfiHttpMethod::Put => "PUT",
            FfiHttpMethod::Delete => "DELETE",
        };

        let mut builder = Request::builder().method(method).uri(uri);
        for kv in &request.headers {
            builder = builder.header(&kv.name, &kv.value);
        }
        let req = builder
            .body(Body::from(request.body))
            .map_err(|e| FfiSyncError::Transport {
                message: e.to_string(),
            })?;

        // Rebuild the router against the shared state so token/session/blob
        // state persists between calls.
        let router = api::router(self.state.clone());
        let resp = router
            .oneshot(req)
            .await
            .map_err(|e| FfiSyncError::Transport {
                message: e.to_string(),
            })?;

        let status = resp.status().as_u16();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .map_err(|e| FfiSyncError::Transport {
                message: e.to_string(),
            })?
            .to_vec();
        Ok(FfiRawResponse { status, body })
    }
}

/// Build a fresh FFI client and the shared sender it is bound to. Returning the
/// `Arc<RouterSender>` lets a second client share the same server state (to
/// simulate a second device pulling what the first pushed).
fn client_with_sender() -> (Arc<LdgrSyncClient>, Arc<RouterSender>) {
    let sender = Arc::new(RouterSender::new());
    let client = LdgrSyncClient::new(sender.clone());
    (client, sender)
}

fn test_argon2() -> FfiArgon2Params {
    // Mirrors `Argon2Params::test()` — minimal, NOT for production.
    FfiArgon2Params {
        memory_cost_kib: 64,
        iterations: 1,
        parallelism: 1,
    }
}

const SALT: &[u8] = b"argon-salt-16byte";
const HELLO: &[u8] = b"\x00\x01\xfe\xff opaque ciphertext blob";

// ── Single-secret (legacy SRP) ──────────────────────────────────────────────────

#[tokio::test]
async fn ffi_single_secret_full_round_trip() {
    let (c, sender) = client_with_sender();
    let username = "alice".to_string();
    let password = b"correct horse battery staple".to_vec();

    c.register(username.clone(), password.clone())
        .await
        .expect("register");
    c.login(username.clone(), password.clone())
        .await
        .expect("login");
    assert!(c.is_authenticated().await);
    let token = c.token().await.expect("token present after login");

    let vault = "vault-1".to_string();
    c.create_vault(vault.clone()).await.expect("create vault");

    // Push an opaque (ciphertext-shaped) batch through the FFI boundary.
    let device = "device-a".to_string();
    let batch = "batch-0001".to_string();
    let put = c
        .put_batch(vault.clone(), device.clone(), batch.clone(), HELLO.to_vec())
        .await
        .expect("put batch");
    assert_eq!(put.size, HELLO.len() as u64);

    let metas = c
        .list_remote_batches(vault.clone(), None, None, None)
        .await
        .expect("list remote batches");
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].batch_id, batch);
    assert_eq!(metas[0].device_id, device);

    // A SECOND handle, reconstructed from the persisted token and sharing the
    // same server state, pulls the blob back byte-for-byte (simulating a second
    // device, and exercising `with_token`).
    let c2 = LdgrSyncClient::with_token(sender, token);
    assert!(c2.is_authenticated().await);
    let fetched = c2
        .get_batch(vault, device, batch)
        .await
        .expect("get batch on second handle");
    assert_eq!(
        fetched, HELLO,
        "batch must round-trip byte-for-byte over FFI"
    );
}

#[tokio::test]
async fn ffi_login_with_wrong_password_is_rejected() {
    let (c, _s) = client_with_sender();
    c.register("bob".into(), b"right-password".to_vec())
        .await
        .expect("register");

    let result = c.login("bob".into(), b"wrong-password".to_vec()).await;
    assert!(
        result.is_err(),
        "login with the wrong password must fail, got {result:?}"
    );
    assert!(!c.is_authenticated().await);
}

// ── Two-secret (2SKD, ADR-008) ──────────────────────────────────────────────────

#[tokio::test]
async fn ffi_two_secret_full_round_trip() {
    let (c, sender) = client_with_sender();
    let username = "carol".to_string();
    let account_id = uuid::Uuid::from_bytes([0x42; 16]);
    let password = b"correct horse battery staple".to_vec();
    // The Secret Key is derived in-FFI from its canonical text form.
    let secret_key = ldgr_core::crypto::SecretKey::generate(account_id).encode();

    c.register_2skd(
        username.clone(),
        account_id.to_string(),
        password.clone(),
        secret_key.clone(),
        SALT.to_vec(),
        test_argon2(),
    )
    .await
    .expect("register 2skd");

    c.login_2skd(
        username,
        account_id.to_string(),
        password,
        secret_key,
        SALT.to_vec(),
        test_argon2(),
    )
    .await
    .expect("login 2skd");
    assert!(c.is_authenticated().await);

    let vault = "vault-2skd".to_string();
    c.create_vault(vault.clone()).await.expect("create vault");

    let device = "device-2skd".to_string();
    let batch = "batch-2skd-1".to_string();
    c.put_batch(vault.clone(), device.clone(), batch.clone(), HELLO.to_vec())
        .await
        .expect("put batch");

    // Pull on a second handle sharing the same server state.
    let token = c.token().await.expect("token");
    let c2 = LdgrSyncClient::with_token(sender, token);
    let fetched = c2.get_batch(vault, device, batch).await.expect("get batch");
    assert_eq!(fetched, HELLO, "2skd batch must round-trip byte-for-byte");
}

#[tokio::test]
async fn ffi_login_2skd_with_wrong_secret_key_is_rejected() {
    let (c, _s) = client_with_sender();
    let username = "dave".to_string();
    let account_id = uuid::Uuid::from_bytes([0x77; 16]);
    let password = b"correct horse battery staple".to_vec();
    let registered = ldgr_core::crypto::SecretKey::generate(account_id).encode();
    // Correct password, wrong Secret Key.
    let attacker = ldgr_core::crypto::SecretKey::generate(account_id).encode();

    c.register_2skd(
        username.clone(),
        account_id.to_string(),
        password.clone(),
        registered,
        SALT.to_vec(),
        test_argon2(),
    )
    .await
    .expect("register 2skd");

    let result = c
        .login_2skd(
            username,
            account_id.to_string(),
            password,
            attacker,
            SALT.to_vec(),
            test_argon2(),
        )
        .await;
    assert!(
        result.is_err(),
        "login with the wrong Secret Key must fail, got {result:?}"
    );
    assert!(!c.is_authenticated().await);
}

// ── Unauthenticated access ──────────────────────────────────────────────────────

#[tokio::test]
async fn ffi_unauthenticated_call_is_rejected() {
    let (c, _s) = client_with_sender();
    // No register/login: the core client guards locally before sending.
    let result = c.create_vault("nope".into()).await;
    assert!(
        matches!(result, Err(FfiSyncError::NotAuthenticated)),
        "expected NotAuthenticated, got {result:?}"
    );
}
