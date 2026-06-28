//! End-to-end coverage for the CLI sync **round-trip** through the real
//! pipeline (issues #205 + #209).
//!
//! Spins up an in-process `ldgr-server`, then drives the CLI bridge helpers
//! ([`ldgr_cli::sync::bridge`]) against two independent vault databases that
//! share one vault key — exactly how two devices would sync. Proves that:
//!
//! 1. a mutation on device A emits an outbox event, `push_pending` uploads it,
//!    and `pull_and_apply` on device B materializes the entity (and the pull
//!    cursor makes a second pull a no-op); and
//! 2. concurrent edits to the same entity surface as a persisted conflict for
//!    review (ADR-003 — no silent last-write-wins).

use std::sync::Arc;
use std::time::Duration;

use ldgr_cli::sync::bridge::{cli_sync_context, pull_and_apply, push_pending};
use ldgr_cli::sync::server::{ReqwestSender, ServerTransport};

use ldgr_core::storage::accounts::{
    AccountType, AccountUpdate, ListOptions, NewAccount, create_account_with_sync,
    get_account_by_name, list_accounts, update_account_with_sync,
};
use ldgr_core::storage::sync::{device_id, pending_event_count, unresolved_conflict_count};
use ldgr_core::storage::transactions::{
    NewPosting, NewTransaction, TransactionStatus, create_transaction_with_sync, get_transaction,
};

use ldgr_core::sync::server::ServerSyncClient;

use ldgr_server::auth::srp::SrpHandshakeStore;
use ldgr_server::config::{Config, RegistrationPolicy};
use ldgr_server::state::AppState;
use ldgr_server::storage::ServerDb;

use rusqlite::Connection;

/// The shared 32-byte vault session key both devices unlock with.
const VAULT_KEY: [u8; 32] = [0x42; 32];

/// A fresh in-memory vault DB with the full schema initialized.
fn vault() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory vault");
    ldgr_core::storage::schema::initialize(&conn).expect("initialize schema");
    conn
}

/// Boot an in-process server on an ephemeral loopback port; return its base URL.
async fn spawn_server() -> String {
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

    let app = ldgr_server::api::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    format!("http://{addr}")
}

/// Register + login + create the vault, returning the bearer token used to build
/// each device's transport.
async fn provision(base_url: &str, username: &str, vault_id: &str) -> String {
    let sender = ReqwestSender::new(base_url.to_string());
    let mut client = ServerSyncClient::new(sender);
    client
        .register(username, b"correct horse battery staple")
        .await
        .expect("register");
    client
        .login(username, b"correct horse battery staple")
        .await
        .expect("login");
    client.create_vault(vault_id).await.expect("create vault");
    client.token().expect("session token").to_string()
}

#[tokio::test]
async fn cli_sync_round_trips_account_and_transaction() {
    let base_url = spawn_server().await;
    let vault_id = "vault_roundtrip";
    let token = provision(&base_url, "roundtrip@example.com", vault_id).await;

    // Two devices, same key, independent databases.
    let a = vault();
    let b = vault();
    let dev_a = device_id(&a).unwrap();
    let dev_b = device_id(&b).unwrap();
    assert_ne!(dev_a, dev_b, "devices must have distinct ids");

    let transport_a = ServerTransport::new(base_url.clone(), token.clone(), vault_id.to_string());
    let transport_b = ServerTransport::new(base_url.clone(), token, vault_id.to_string());

    // ── Device A mutates: two accounts + a transaction across them. ──
    let cash = create_account_with_sync(
        &a,
        &NewAccount {
            name: "Assets:Cash".into(),
            account_type: AccountType::Asset,
            commodity: Some("USD".into()),
            parent_id: None,
            note: Some("petty cash".into()),
        },
        &cli_sync_context(&a).unwrap(),
    )
    .unwrap();
    let food = create_account_with_sync(
        &a,
        &NewAccount {
            name: "Expenses:Food".into(),
            account_type: AccountType::Expense,
            commodity: Some("USD".into()),
            parent_id: None,
            note: None,
        },
        &cli_sync_context(&a).unwrap(),
    )
    .unwrap();
    let txn = create_transaction_with_sync(
        &a,
        &NewTransaction {
            date: "2024-01-15".into(),
            status: TransactionStatus::Cleared,
            code: Some("REF-42".into()),
            description: "Lunch".into(),
            comment: Some("with team".into()),
            postings: vec![
                NewPosting {
                    account_id: cash.id.clone(),
                    amount_quantity: Some("-10.00".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                },
                NewPosting {
                    account_id: food.id.clone(),
                    amount_quantity: Some("10.00".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                },
            ],
        },
        &cli_sync_context(&a).unwrap(),
    )
    .unwrap();

    assert_eq!(
        pending_event_count(&a).unwrap(),
        3,
        "3 events pending pre-push"
    );

    // ── Push from A. ──
    let push = push_pending(&a, &transport_a, vault_id, &dev_a, &VAULT_KEY)
        .await
        .expect("push");
    assert_eq!(push.batches_pushed, 1);
    assert_eq!(push.events_pushed, 3);
    assert_eq!(
        pending_event_count(&a).unwrap(),
        0,
        "events marked synced after push"
    );

    // ── Pull into B. ──
    let pull = pull_and_apply(&b, &transport_b, vault_id, &dev_b, &VAULT_KEY)
        .await
        .expect("pull");
    assert_eq!(pull.batches_ingested, 1);
    assert_eq!(pull.applied, 3);
    assert_eq!(pull.conflicts, 0);

    // Accounts + transaction materialized on B.
    let b_accounts = list_accounts(&b, &ListOptions::default()).unwrap();
    assert_eq!(b_accounts.len(), 2, "both accounts reproduced on B");
    let bt = get_transaction(&b, &txn.id, &ListOptions::default())
        .unwrap()
        .expect("transaction reproduced on B");
    assert_eq!(bt.description, "Lunch");
    assert_eq!(bt.postings.len(), 2);

    // ── Pulling again is a no-op (cursor skips the already-applied batch). ──
    let pull_again = pull_and_apply(&b, &transport_b, vault_id, &dev_b, &VAULT_KEY)
        .await
        .expect("second pull");
    assert_eq!(
        pull_again.batches_ingested, 0,
        "already-applied batch skipped"
    );
}

#[tokio::test]
async fn cli_sync_surfaces_conflict_on_concurrent_edits() {
    let base_url = spawn_server().await;
    let vault_id = "vault_conflict";
    let token = provision(&base_url, "conflict@example.com", vault_id).await;

    let a = vault();
    let b = vault();
    let dev_a = device_id(&a).unwrap();
    let dev_b = device_id(&b).unwrap();

    let transport_a = ServerTransport::new(base_url.clone(), token.clone(), vault_id.to_string());
    let transport_b = ServerTransport::new(base_url.clone(), token, vault_id.to_string());

    // A creates an account and pushes; B pulls so both know it (version 1).
    let acct = create_account_with_sync(
        &a,
        &NewAccount {
            name: "Assets:Shared".into(),
            account_type: AccountType::Asset,
            commodity: Some("USD".into()),
            parent_id: None,
            note: None,
        },
        &cli_sync_context(&a).unwrap(),
    )
    .unwrap();
    push_pending(&a, &transport_a, vault_id, &dev_a, &VAULT_KEY)
        .await
        .expect("push create");
    let pulled = pull_and_apply(&b, &transport_b, vault_id, &dev_b, &VAULT_KEY)
        .await
        .expect("pull create");
    assert_eq!(pulled.applied, 1);

    // ── Concurrent edits: B renames locally (un-pushed) while A renames + pushes. ──
    let b_acct = get_account_by_name(&b, "Assets:Shared")
        .unwrap()
        .expect("account on B");
    update_account_with_sync(
        &b,
        &b_acct.id,
        &AccountUpdate {
            name: "Assets:RenamedByB".into(),
            account_type: b_acct.account_type,
            commodity: b_acct.commodity.clone(),
            parent_id: b_acct.parent_id.clone(),
            note: b_acct.note.clone(),
            expected_version: b_acct.version,
        },
        &cli_sync_context(&b).unwrap(),
    )
    .unwrap();

    update_account_with_sync(
        &a,
        &acct.id,
        &AccountUpdate {
            name: "Assets:RenamedByA".into(),
            account_type: acct.account_type,
            commodity: acct.commodity.clone(),
            parent_id: acct.parent_id.clone(),
            note: acct.note.clone(),
            expected_version: acct.version,
        },
        &cli_sync_context(&a).unwrap(),
    )
    .unwrap();
    push_pending(&a, &transport_a, vault_id, &dev_a, &VAULT_KEY)
        .await
        .expect("push A's edit");

    // B pulls A's conflicting edit — its own un-pushed edit collides.
    let conflicted = pull_and_apply(&b, &transport_b, vault_id, &dev_b, &VAULT_KEY)
        .await
        .expect("pull conflicting edit");
    assert!(
        conflicted.conflicts >= 1,
        "concurrent edit must surface a conflict, got {conflicted:?}"
    );
    assert!(
        unresolved_conflict_count(&b).unwrap() >= 1,
        "conflict persisted for review"
    );

    // Local-wins-pending-review: B still shows its own rename until resolved.
    assert!(
        get_account_by_name(&b, "Assets:RenamedByB")
            .unwrap()
            .is_some(),
        "B keeps its local version pending resolution"
    );
}
