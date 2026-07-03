//! End-to-end coverage for the `ldgr-server` blob transport.
//!
//! Spins up a real `ldgr-server` in-process (ephemeral port, in-memory DB,
//! open registration) and drives the CLI's [`ServerTransport`] against it over
//! `reqwest`, proving that encrypted batch and snapshot blobs round-trip
//! byte-for-byte through a live server — including listing, existence checks,
//! and put-if-absent conflict classification.

use std::sync::Arc;
use std::time::Duration;

use ldgr_cli::sync::BlobTransport;
use ldgr_cli::sync::server::{ReqwestSender, ServerTransport};

use ldgr_core::sync::server::ServerSyncClient;
use ldgr_core::sync::transport::{
    TransportErrorKind, batch_path, batches_prefix, snapshot_path, snapshots_prefix,
};

use ldgr_server::auth::srp::SrpHandshakeStore;
use ldgr_server::config::{Config, RegistrationPolicy};
use ldgr_server::state::AppState;
use ldgr_server::storage::ServerDb;

/// Boot an in-process server on an ephemeral loopback port and return its
/// base URL (`http://127.0.0.1:<port>`) together with the shared [`AppState`]
/// so tests can seed the blob store directly.
async fn spawn_server() -> (String, Arc<AppState>) {
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        session_ttl_hours: 720,
        relay_ttl_minutes: 10,
        max_blob_bytes: 50 * 1024 * 1024,
        srp_handshake_ttl_secs: 120,
        registration_policy: RegistrationPolicy::Open,
        admin_email: None,
        default_user_quota_bytes: 1024 * 1024 * 1024,
        server_name: "ldgr-test-server".into(),
    };

    let db = ServerDb::open(":memory:").expect("open in-memory db");
    let state = Arc::new(AppState {
        db,
        srp_handshakes: SrpHandshakeStore::new(Duration::from_mins(2)),
        config,
    });

    let app = ldgr_server::api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    (format!("http://{addr}"), state)
}

#[tokio::test]
async fn server_transport_round_trips_batch_and_snapshot() {
    let (base_url, _state) = spawn_server().await;

    let vault_id = "vault_itest";
    let device_id = "dev_alpha";
    let username = "itest@example.com";
    let password = b"correct horse battery staple";

    // ── Setup: register + login + create vault (the path `sync setup` drives),
    //    capturing the bearer token we then persist and reuse. ──
    let token = {
        let sender = ReqwestSender::new(base_url.clone());
        let mut client = ServerSyncClient::new(sender);
        client.register(username, password).await.expect("register");
        client.login(username, password).await.expect("login");
        client.create_vault(vault_id).await.expect("create vault");
        client.token().expect("session token").to_string()
    };

    // ── Writer transport (token-based, as built from sync-config.json). ──
    let writer = ServerTransport::new(base_url.clone(), token.clone(), vault_id.to_string());

    let batch = batch_path(vault_id, device_id, "batch_0001");
    let batch_bytes = b"\x00\x01encrypted-batch-ciphertext\xff".to_vec();
    writer
        .put_blob(&batch, &batch_bytes)
        .await
        .expect("put batch");

    let snap = snapshot_path(vault_id, "snap_0001");
    let snap_bytes = b"\xde\xad\xbe\xefencrypted-snapshot-ciphertext".to_vec();
    writer
        .put_blob(&snap, &snap_bytes)
        .await
        .expect("put snapshot");

    // ── Reader transport: a second client/device using the same account. ──
    let reader = ServerTransport::new(base_url.clone(), token, vault_id.to_string());

    // Batch is listed and round-trips identically.
    let batches = reader
        .list_blobs(&batches_prefix(vault_id), None)
        .await
        .expect("list batches");
    assert!(
        batches.entries.iter().any(|e| e.path == batch.as_str()),
        "uploaded batch should appear in the listing: {:?}",
        batches.entries
    );
    let got_batch = reader.get_blob(&batch).await.expect("get batch");
    assert_eq!(
        got_batch, batch_bytes,
        "batch bytes must round-trip byte-for-byte"
    );

    // Snapshot is listed and round-trips identically.
    let snaps = reader
        .list_blobs(&snapshots_prefix(vault_id), None)
        .await
        .expect("list snapshots");
    assert!(
        snaps.entries.iter().any(|e| e.path == snap.as_str()),
        "uploaded snapshot should appear in the listing: {:?}",
        snaps.entries
    );
    let got_snap = reader.get_blob(&snap).await.expect("get snapshot");
    assert_eq!(
        got_snap, snap_bytes,
        "snapshot bytes must round-trip byte-for-byte"
    );

    // exists() resolves both present and absent blobs.
    assert!(reader.exists(&batch).await.expect("exists present"));
    let missing = batch_path(vault_id, device_id, "batch_missing");
    assert!(!reader.exists(&missing).await.expect("exists absent"));

    // Put-if-absent: re-uploading the same batch is classified as a Conflict.
    let dup = writer.put_blob(&batch, &batch_bytes).await;
    let err = dup.expect_err("duplicate put should be rejected");
    assert_eq!(
        err.kind,
        TransportErrorKind::Conflict,
        "duplicate upload must classify as Conflict, got: {err}"
    );
}

/// A vault with more batches than a single server page must list *all* of them:
/// `ServerTransport::list_blobs` follows the server's keyset cursor across every
/// page. Blobs are seeded straight into the store so many share a `created_at`
/// timestamp — exercising the `(created_at, path)` tie-break that keeps pages
/// from overlapping or skipping.
#[tokio::test]
async fn server_transport_lists_all_batches_across_pages() {
    let (base_url, state) = spawn_server().await;

    let vault_id = "vault_pages";
    let device_id = "dev_bulk";
    let username = "pages@example.com";
    let password = b"correct horse battery staple";

    let token = {
        let sender = ReqwestSender::new(base_url.clone());
        let mut client = ServerSyncClient::new(sender);
        client.register(username, password).await.expect("register");
        client.login(username, password).await.expect("login");
        client.create_vault(vault_id).await.expect("create vault");
        client.token().expect("session token").to_string()
    };

    // Seed 250 batches directly (server default page size is 100 → 3 pages).
    let total = 250usize;
    let quota = 1024 * 1024 * 1024;
    for i in 0..total {
        let path = batch_path(vault_id, device_id, &format!("batch_{i:04}"));
        let data = format!("ciphertext-{i}").into_bytes();
        state
            .db
            .put_blob(path.as_str(), vault_id, data, "deadbeef", quota)
            .await
            .expect("seed batch");
    }

    let reader = ServerTransport::new(base_url, token, vault_id.to_string());
    let listing = reader
        .list_blobs(&batches_prefix(vault_id), None)
        .await
        .expect("list batches");

    assert!(!listing.has_more, "paginated listing must be fully drained");

    let mut paths: Vec<String> = listing.entries.iter().map(|e| e.path.clone()).collect();
    paths.sort();
    paths.dedup();
    assert_eq!(
        paths.len(),
        total,
        "every seeded batch must appear exactly once across all pages"
    );

    for i in 0..total {
        let expected = batch_path(vault_id, device_id, &format!("batch_{i:04}"));
        assert!(
            paths.binary_search(&expected.as_str().to_string()).is_ok(),
            "missing batch {expected} from paginated listing"
        );
    }
}
