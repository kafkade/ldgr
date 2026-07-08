//! End-to-end, in-process integration test for the **FFI** server-sync surface
//! (issue #200): drives `ldgr-ffi`'s [`LdgrSyncClient`] through its foreign
//! async-sender seam ([`FfiHttpSender`]) against the real `ldgr-server` axum
//! router via `tower`'s `oneshot` — no sockets, no real HTTP client.
//!
//! This is the FFI analogue of `server_sync_client_e2e.rs`: it proves the host
//! transport contract (the seam Swift satisfies over `URLSession` and web over
//! `fetch`) round-trips correctly and that auth (single-secret + 2SKD) and
//! batch blob push/pull work end-to-end through the FFI boundary types.
//!
//! Since #201 (core compose/apply pipeline) and its FFI wrappers (#206) landed,
//! the primary round-trip now drives the **real** batch-blob pipeline through
//! the transport: `LdgrVault::export_pending_batch` on device A → `putBatch` →
//! `getBatch` + `LdgrVault::ingest_batch` on device B (issue #220), replacing
//! the earlier synthetic opaque-bytes round-trip. The server stays a blind blob
//! store — it only ever sees ciphertext.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use ldgr_ffi::{FfiArgon2Params, FfiNewPosting, LdgrSyncClient, LdgrVault};
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

// ── Real on-disk vault helpers (issue #220) ──────────────────────────────────────

/// Build a real device-A vault on disk: two accounts + one balanced
/// transaction, leaving the sync outbox populated with pending events ready for
/// [`LdgrVault::export_pending_batch`]. Returns the handle and the transaction
/// id (stable across devices, since sync events carry entity ids).
fn real_device_a(dir: &std::path::Path) -> (LdgrVault, String) {
    let vault = LdgrVault::new(dir.to_string_lossy().to_string()).expect("new vault A");
    vault
        .create_vault("test-pw".to_string(), "Device A".to_string())
        .expect("create vault A");

    let checking = vault
        .add_account(
            "Assets:Checking".to_string(),
            "asset".to_string(),
            Some("USD".to_string()),
        )
        .expect("add checking");
    let food = vault
        .add_account(
            "Expenses:Food".to_string(),
            "expense".to_string(),
            Some("USD".to_string()),
        )
        .expect("add food");
    vault
        .add_transaction(
            "2024-06-15".to_string(),
            "Grocery store".to_string(),
            "cleared".to_string(),
            vec![
                FfiNewPosting {
                    account_id: checking,
                    amount: Some("-50.00".to_string()),
                    commodity: Some("USD".to_string()),
                },
                FfiNewPosting {
                    account_id: food,
                    amount: Some("50.00".to_string()),
                    commodity: Some("USD".to_string()),
                },
            ],
        )
        .expect("add transaction");

    let txn_id = vault.list_transactions().expect("list txns")[0].id.clone();
    (vault, txn_id)
}

/// Enroll a second device of the SAME vault: copy `vault.ldgr` and unlock with
/// A's exported session key. It gets its own fresh, empty database — a distinct
/// device id and no data, exactly like a newly-enrolled device pre-first-sync.
fn sibling_device_b(
    dir_b: &std::path::Path,
    src_dir: &std::path::Path,
    session_key: Vec<u8>,
) -> LdgrVault {
    std::fs::copy(src_dir.join("vault.ldgr"), dir_b.join("vault.ldgr")).expect("copy vault.ldgr");
    let vault = LdgrVault::new(dir_b.to_string_lossy().to_string()).expect("new vault B");
    vault
        .open_with_session_key(session_key)
        .expect("unlock B with session key");
    vault
}

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

    // Device A: a real on-disk vault. Compose its pending events into a REAL
    // encrypted batch blob via the #201 pipeline (through the FFI `LdgrVault`
    // wrapper) and push that ciphertext through the FFI transport — no synthetic
    // opaque bytes.
    let dir_a = tempfile::tempdir().unwrap();
    let (device_a_vault, _txn_id) = real_device_a(dir_a.path());
    let session_key = device_a_vault
        .export_session_key()
        .expect("export session key");
    let device = device_a_vault.sync_status().expect("sync status").device_id;

    let exported = device_a_vault
        .export_pending_batch()
        .expect("export pending batch")
        .expect("device A has pending events to export");
    assert!(!exported.ciphertext.is_empty());
    assert!(!exported.event_ids.is_empty());

    let put = c
        .put_batch(
            vault.clone(),
            device.clone(),
            exported.batch_id.clone(),
            exported.ciphertext.clone(),
        )
        .await
        .expect("put batch");
    assert_eq!(put.size, exported.ciphertext.len() as u64);

    let metas = c
        .list_remote_batches(vault.clone(), None, None, None)
        .await
        .expect("list remote batches");
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].batch_id, exported.batch_id);
    assert_eq!(metas[0].device_id, device);

    // A SECOND handle, reconstructed from the persisted token and sharing the
    // same server state, pulls the blob back byte-for-byte (the server is a
    // blind store; exercises `with_token`).
    let c2 = LdgrSyncClient::with_token(sender, token);
    assert!(c2.is_authenticated().await);
    let fetched = c2
        .get_batch(vault, device, exported.batch_id)
        .await
        .expect("get batch on second handle");
    assert_eq!(
        fetched, exported.ciphertext,
        "batch must round-trip byte-for-byte over FFI"
    );

    // A freshly-enrolled second device ingests the downloaded blob and
    // materializes the real accounting data — proving getBatch → ingestBatch.
    let dir_b = tempfile::tempdir().unwrap();
    let device_b_vault = sibling_device_b(dir_b.path(), dir_a.path(), session_key);
    assert!(device_b_vault.list_transactions().unwrap().is_empty());

    let outcome = device_b_vault.ingest_batch(fetched.clone()).unwrap();
    assert!(outcome.applied > 0, "expected applied > 0, got {outcome:?}");
    assert_eq!(outcome.conflicts, 0);

    let txns = device_b_vault.list_transactions().unwrap();
    assert_eq!(txns.len(), 1, "transaction should materialize on device B");
    assert_eq!(txns[0].description, "Grocery store");
    assert_eq!(device_b_vault.list_accounts().unwrap().len(), 2);

    // Idempotent: re-ingesting the same blob applies nothing.
    let again = device_b_vault.ingest_batch(fetched).unwrap();
    assert_eq!(again.applied, 0);
    assert_eq!(again.conflicts, 0);
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

    c.login_2skd(username, password, secret_key, SALT.to_vec(), test_argon2())
        .await
        .expect("login 2skd");
    assert!(c.is_authenticated().await);

    let vault = "vault-2skd".to_string();
    c.create_vault(vault.clone()).await.expect("create vault");

    // Push a REAL encrypted batch (composed from an on-disk vault) so the 2SKD
    // auth path is exercised against genuine ciphertext, not synthetic bytes.
    let dir_a = tempfile::tempdir().unwrap();
    let (device_a_vault, _txn_id) = real_device_a(dir_a.path());
    let device = device_a_vault.sync_status().expect("sync status").device_id;
    let exported = device_a_vault
        .export_pending_batch()
        .expect("export pending batch")
        .expect("device A has pending events");
    c.put_batch(
        vault.clone(),
        device.clone(),
        exported.batch_id.clone(),
        exported.ciphertext.clone(),
    )
    .await
    .expect("put batch");

    // Pull on a second handle sharing the same server state.
    let token = c.token().await.expect("token");
    let c2 = LdgrSyncClient::with_token(sender, token);
    let fetched = c2
        .get_batch(vault, device, exported.batch_id)
        .await
        .expect("get batch");
    assert_eq!(
        fetched, exported.ciphertext,
        "2skd batch must round-trip byte-for-byte"
    );
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
        .login_2skd(username, password, attacker, SALT.to_vec(), test_argon2())
        .await;
    assert!(
        result.is_err(),
        "login with the wrong Secret Key must fail, got {result:?}"
    );
    assert!(!c.is_authenticated().await);
}

// ── Real batch-blob pipeline through the transport (issue #220) ──────────────────

/// Divergent edits to the SAME transaction on two devices surface a conflict
/// when the second device ingests the first's batch pulled over the FFI
/// transport. Proves `getBatch → ingestBatch` reports conflicts for user review
/// via [`FfiSyncConflict`] — not silent last-write-wins (ADR-003).
#[tokio::test]
async fn ffi_real_batch_conflict_surfaces_through_transport() {
    let (c, sender) = client_with_sender();
    let username = "erin".to_string();
    let password = b"correct horse battery staple".to_vec();
    c.register(username.clone(), password.clone())
        .await
        .expect("register");
    c.login(username.clone(), password.clone())
        .await
        .expect("login");
    let token = c.token().await.expect("token present after login");

    let vault = "vault-conflict".to_string();
    c.create_vault(vault.clone()).await.expect("create vault");

    // Device A: a real vault with a transaction; capture its id + session key.
    let dir_a = tempfile::tempdir().unwrap();
    let (a, txn_id) = real_device_a(dir_a.path());
    let device = a.sync_status().expect("sync status").device_id;
    let session_key = a.export_session_key().expect("export session key");

    // Seed device B: push A's initial batch and ingest it on B so both devices
    // share the transaction before they diverge.
    let seed = a
        .export_pending_batch()
        .expect("export seed batch")
        .expect("device A has pending events");
    c.put_batch(
        vault.clone(),
        device.clone(),
        seed.batch_id.clone(),
        seed.ciphertext.clone(),
    )
    .await
    .expect("put seed batch");
    a.mark_events_synced(seed.event_ids).unwrap();

    let dir_b = tempfile::tempdir().unwrap();
    let b = sibling_device_b(dir_b.path(), dir_a.path(), session_key);
    let c2 = LdgrSyncClient::with_token(sender, token);
    let seed_bytes = c2
        .get_batch(vault.clone(), device.clone(), seed.batch_id)
        .await
        .expect("get seed batch");
    let seeded = b.ingest_batch(seed_bytes).unwrap();
    assert!(seeded.applied > 0, "B should apply the seed batch");
    assert_eq!(b.list_transactions().unwrap().len(), 1);

    // Divergent edit to the SAME entity on both devices: each deletes T while
    // the other's delete is still in flight.
    a.delete_transaction(txn_id.clone()).unwrap();
    b.delete_transaction(txn_id.clone()).unwrap();

    // A composes + pushes its divergent delete; B pulls it over the transport.
    let a_del = a
        .export_pending_batch()
        .expect("export divergent batch")
        .expect("device A has a pending delete");
    c.put_batch(
        vault.clone(),
        device.clone(),
        a_del.batch_id.clone(),
        a_del.ciphertext.clone(),
    )
    .await
    .expect("put divergent batch");
    let a_del_bytes = c2
        .get_batch(vault, device, a_del.batch_id)
        .await
        .expect("get divergent batch");

    // B ingests A's delete while holding its OWN pending delete of T → conflict.
    let outcome = b.ingest_batch(a_del_bytes).unwrap();
    assert!(
        outcome.conflicts >= 1,
        "expected a conflict, got {outcome:?}"
    );
    assert_eq!(outcome.applied, 0, "conflicting edit must not be applied");

    // The conflict is persisted for user review via the FfiSyncConflict surface.
    let conflicts = b.list_conflicts().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].entity_type, "transaction");
    assert_eq!(conflicts[0].entity_id, txn_id);
    assert!(
        !conflicts[0].local_payload.is_empty() && !conflicts[0].remote_payload.is_empty(),
        "both sides retained so a human can review and choose"
    );
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
