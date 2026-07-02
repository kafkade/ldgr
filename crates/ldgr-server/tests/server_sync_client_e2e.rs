//! End-to-end, in-process integration tests for the core
//! `ServerSyncClient` against the real `ldgr-server` axum router.
//!
//! The shared [`common`] harness bridges the transport-agnostic
//! `RawRequest`/`RawResponse` contract straight into the router via `tower`'s
//! `oneshot` — no sockets are bound and no HTTP client is involved. It holds the
//! server `SharedState` so every `send` rebuilds the router against the **same**
//! state, letting auth tokens and stored blobs persist across calls.
//!
//! These tests exercise both auth schemes: legacy single-secret SRP and the
//! two-secret (2SKD, ADR-008) path. The server is a blind blob store, so blob
//! round-trips use arbitrary opaque bytes — no real vault encryption needed. The
//! real compose/apply pipeline is driven through the same harness in
//! `sync_pipeline_e2e.rs`.

mod common;

use common::{auth_key, client};
use ldgr_core::crypto::SecretKey;
use ldgr_core::sync::server::{ListBatchesQuery, ServerSyncError};

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
    c.login_2skd(username, &mk_auth, &secret_key)
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

    let result = c.login_2skd(username, &mk_auth, &attacker).await;
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
