//! End-to-end, in-process integration tests that drive the **real** sync
//! compose/apply pipeline ([`ldgr_core::sync::pipeline`]) THROUGH the booted
//! `ldgr-server` axum router.
//!
//! Unlike `server_sync_client_e2e.rs` (which treats the server as a blind blob
//! store and pushes opaque bytes to prove transport + auth), this suite seeds
//! real vault state, composes encrypted batches with
//! [`export_pending_batch`](ldgr_core::sync::pipeline::export_pending_batch),
//! uploads them via the shared [`common`] `RouterSender`, downloads them on a
//! second device, and applies them with
//! [`ingest_batch`](ldgr_core::sync::pipeline::ingest_batch). It asserts:
//!
//! 1. Real bidirectional cross-device propagation (entities materialize on the
//!    peer, field-for-field, in both directions).
//! 2. The FFI/WASM session-key seam (`*_with_session_key`) using a real
//!    [`UnlockedVault`](ldgr_core::crypto::UnlockedVault) end-to-end through the
//!    server.
//! 3. Concurrent divergent edits to the SAME transaction surface a conflict for
//!    user review (ADR-003) — `IngestOutcome.conflicts > 0`, a `StoredConflict`
//!    persisted, NO silent last-write-wins — and the post-merge state still
//!    satisfies the double-entry balanced invariant.
//! 4. A fresh device onboards to consistency by replaying every batch.
//!
//! ## Key handling
//!
//! [`VaultKey`](ldgr_core::crypto::VaultKey) is intentionally non-constructible
//! outside `ldgr-core` (`from_bytes`/`as_bytes` are `pub(crate)`), so these
//! external tests cannot inject or extract raw key bytes. The cross-device tests
//! therefore hold a single [`VaultKey::generate()`] value and share it by
//! reference across both device connections — exactly what two devices of one
//! account do (same vault key, different `device_id`). The session-key seam test
//! (#2) instead derives the raw 32-byte key from a real `UnlockedVault`, the only
//! way a foreign (FFI/WASM) host obtains it.
//!
//! ## Scope note — snapshots
//!
//! Real materialized-state *snapshot* compose/apply is NOT wired into the
//! pipeline yet (only the [`Snapshot`](ldgr_core::sync::Snapshot) type, planning
//! helpers, and an opaque snapshot-blob round-trip exist). Onboarding-to-
//! consistency here is implemented honestly as full event-batch **replay**; a
//! snapshot-based fast path is intentionally out of scope for this suite.

mod common;

use std::collections::BTreeMap;
use std::str::FromStr;

use rust_decimal::Decimal;

use ldgr_core::crypto::{Argon2Params, UnlockedVault, VaultKey, create_vault};
use ldgr_core::storage::accounts::{
    AccountType, ListOptions, NewAccount, create_account_with_sync, get_account, list_accounts,
};
use ldgr_core::storage::schema;
use ldgr_core::storage::sync::{
    SyncContext, get_conflict, list_unresolved_conflicts, mark_events_synced, pending_events,
    record_event, unresolved_conflict_count,
};
use ldgr_core::storage::transactions::{
    NewPosting, NewTransaction, Transaction, TransactionStatus, create_transaction_with_sync,
    get_transaction,
};
use ldgr_core::sync::payload::{self, PostingPayload, TransactionPayload};
use ldgr_core::sync::pipeline::{
    ExportedBatch, IngestOutcome, export_pending_batch, export_pending_batch_with_session_key,
    ingest_batch, ingest_batch_with_session_key, resolve_conflict_keep_remote,
};
use ldgr_core::sync::server::{ListBatchesQuery, RawHttpSender, ServerSyncClient};

use common::client;
use rusqlite::Connection;

// ── Local vault helpers ──────────────────────────────────────────────────────

/// A fresh, schema-initialized in-memory vault (one device's canonical store).
fn fresh_vault() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory vault");
    schema::initialize(&conn).expect("initialize schema");
    conn
}

fn ctx(device: &str, lamport: u64) -> SyncContext {
    SyncContext {
        device_id: device.into(),
        lamport_clock: lamport,
    }
}

/// Seed a balanced two-posting transaction (`Assets:Cash` → `Expenses:Food`)
/// plus its two accounts, all recorded with sync events for `device`. Returns
/// the `(cash_id, food_id, txn_id)`.
fn seed_balanced_books(conn: &Connection, device: &str) -> (String, String, String) {
    let cash = create_account_with_sync(
        conn,
        &NewAccount {
            name: "Assets:Cash".into(),
            account_type: AccountType::Asset,
            commodity: Some("USD".into()),
            parent_id: None,
            note: Some("petty cash".into()),
        },
        &ctx(device, 1),
    )
    .expect("create cash account")
    .id;

    let food = create_account_with_sync(
        conn,
        &NewAccount {
            name: "Expenses:Food".into(),
            account_type: AccountType::Expense,
            commodity: Some("USD".into()),
            parent_id: None,
            note: None,
        },
        &ctx(device, 2),
    )
    .expect("create food account")
    .id;

    let txn = create_transaction_with_sync(
        conn,
        &NewTransaction {
            date: "2024-01-15".into(),
            status: TransactionStatus::Cleared,
            code: Some("REF-42".into()),
            description: "Lunch".into(),
            comment: Some("with team".into()),
            postings: vec![
                NewPosting {
                    account_id: cash.clone(),
                    amount_quantity: Some("-10.00".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                },
                NewPosting {
                    account_id: food.clone(),
                    amount_quantity: Some("10.00".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                },
            ],
        },
        &ctx(device, 3),
    )
    .expect("create transaction")
    .id;

    (cash, food, txn)
}

/// Assert a materialized transaction satisfies the double-entry invariant: its
/// postings sum to exactly zero per commodity.
///
/// ADR-003 §"Post-merge validation" requires validating double-entry invariants
/// after every sync. There is no transaction-level `validate_balanced` helper in
/// core (the `validate*` fns are budget/loan/vault scoped), and synced state
/// lives in `storage` rows rather than `accounting::types::Transaction`, so we
/// compute the per-commodity decimal sum directly here. Amounts are parsed as
/// exact [`Decimal`] values — never floats — per the repo's money rules.
fn assert_transaction_balanced(conn: &Connection, txn_id: &str) {
    let txn = get_transaction(conn, txn_id, &ListOptions::default())
        .expect("query transaction")
        .expect("transaction present");
    assert_balanced(&txn);
}

fn assert_balanced(txn: &Transaction) {
    let mut sums: BTreeMap<String, Decimal> = BTreeMap::new();
    for p in &txn.postings {
        if let (Some(qty), Some(commodity)) = (&p.amount_quantity, &p.amount_commodity) {
            let dec = Decimal::from_str(qty).expect("posting amount is a valid decimal");
            *sums.entry(commodity.clone()).or_default() += dec;
        }
    }
    assert!(
        !sums.is_empty(),
        "transaction {} has no priced postings to balance",
        txn.id
    );
    for (commodity, sum) in sums {
        assert_eq!(
            sum,
            Decimal::ZERO,
            "postings for {commodity} in transaction {} must sum to zero (got {sum})",
            txn.id
        );
    }
}

// ── Server-side push/pull helpers (through the booted router) ─────────────────

/// Register, log in, and create `vault_id` on a freshly booted in-process
/// server. Returns the authenticated client.
async fn logged_in_client(vault_id: &str) -> ServerSyncClient<common::RouterSender> {
    let mut c = client();
    c.register("alice", b"correct horse battery staple")
        .await
        .expect("register");
    c.login("alice", b"correct horse battery staple")
        .await
        .expect("login");
    c.create_vault(vault_id).await.expect("create vault");
    c
}

/// Upload a composed batch to the server.
async fn push_batch<T: RawHttpSender>(
    c: &ServerSyncClient<T>,
    vault_id: &str,
    batch: &ExportedBatch,
) {
    c.put_batch(
        vault_id,
        &batch.device_id,
        &batch.batch_id,
        &batch.ciphertext,
    )
    .await
    .expect("put batch");
}

/// Download every remote batch's ciphertext (in server-listed order).
async fn pull_all_batches<T: RawHttpSender>(
    c: &ServerSyncClient<T>,
    vault_id: &str,
) -> Vec<Vec<u8>> {
    let metas = c
        .list_remote_batches(vault_id, &ListBatchesQuery::default())
        .await
        .expect("list remote batches");
    let mut out = Vec::with_capacity(metas.len());
    for m in metas {
        let bytes = c
            .get_batch(vault_id, &m.device_id, &m.batch_id)
            .await
            .expect("get batch");
        out.push(bytes);
    }
    out
}

// ── 1. Real bidirectional cross-device propagation ───────────────────────────

#[tokio::test]
async fn real_cross_device_propagation_round_trip() {
    let vk = VaultKey::generate();
    let vault_id = "vault-propagation";
    let c = logged_in_client(vault_id).await;

    // Device A: real seeded books (2 accounts + 1 balanced transaction).
    let a = fresh_vault();
    let (cash_id, food_id, txn_id) = seed_balanced_books(&a, "dev_a");
    assert_transaction_balanced(&a, &txn_id);

    // A → server: compose + upload, then mark its outbox synced.
    let a_batch = export_pending_batch(&a, "dev_a", &vk)
        .expect("export A")
        .expect("A has pending events");
    push_batch(&c, vault_id, &a_batch).await;
    mark_events_synced(&a, &a_batch.event_ids).expect("mark A synced");

    // Device B: fresh vault, SAME vault key. Download + apply A's batch.
    let b = fresh_vault();
    let downloaded = pull_all_batches(&c, vault_id).await;
    assert_eq!(downloaded.len(), 1, "exactly one batch on the server");
    let outcome = ingest_batch(&b, "dev_b", &vk, &downloaded[0]).expect("ingest on B");
    assert_eq!(
        outcome,
        IngestOutcome {
            applied: 3,
            conflicts: 0,
            skipped: 0
        }
    );

    // B materializes A's accounts + transaction, and the transaction balances.
    let b_accounts = list_accounts(&b, &ListOptions::default()).expect("list B accounts");
    assert_eq!(b_accounts.len(), 2);
    assert!(
        get_account(&b, &cash_id, &ListOptions::default())
            .unwrap()
            .is_some()
    );
    assert!(
        get_account(&b, &food_id, &ListOptions::default())
            .unwrap()
            .is_some()
    );
    let bt = get_transaction(&b, &txn_id, &ListOptions::default())
        .unwrap()
        .expect("txn on B");
    assert_eq!(bt.description, "Lunch");
    assert_eq!(bt.postings.len(), 2);
    assert_transaction_balanced(&b, &txn_id);

    // ── Reverse direction: B → A ──────────────────────────────────────────────
    let b_only = create_account_with_sync(
        &b,
        &NewAccount {
            name: "Assets:Bank".into(),
            account_type: AccountType::Asset,
            commodity: Some("USD".into()),
            parent_id: None,
            note: None,
        },
        &ctx("dev_b", 1),
    )
    .expect("create B-only account")
    .id;

    let b_batch = export_pending_batch(&b, "dev_b", &vk)
        .expect("export B")
        .expect("B has pending events");
    push_batch(&c, vault_id, &b_batch).await;

    // A pulls B's batch specifically and applies it.
    let b_bytes = c
        .get_batch(vault_id, &b_batch.device_id, &b_batch.batch_id)
        .await
        .expect("get B batch");
    let back = ingest_batch(&a, "dev_a", &vk, &b_bytes).expect("ingest on A");
    assert_eq!(back.applied, 1);
    assert_eq!(back.conflicts, 0);
    assert!(
        get_account(&a, &b_only, &ListOptions::default())
            .unwrap()
            .is_some(),
        "A received B's account in the reverse direction"
    );
}

// ── 2. FFI/WASM session-key seam, end-to-end through the server ───────────────

#[tokio::test]
async fn session_key_seam_propagates_through_server() {
    // A foreign (FFI/WASM) host never holds a `VaultKey`; it holds the raw
    // 32-byte session key exported from a real unlocked vault. Exercise that
    // exact path end-to-end through the booted server.
    let (unlocked, _recovery): (UnlockedVault, _) = create_vault(
        b"correct horse battery staple",
        "seam-vault",
        &Argon2Params::test(),
    )
    .expect("create vault");
    let session_key = unlocked.export_session_key();

    let vault_id = "vault-seam";
    let c = logged_in_client(vault_id).await;

    let a = fresh_vault();
    let acct = create_account_with_sync(
        &a,
        &NewAccount {
            name: "Assets:Checking".into(),
            account_type: AccountType::Asset,
            commodity: Some("USD".into()),
            parent_id: None,
            note: None,
        },
        &ctx("dev_a", 1),
    )
    .expect("seed account")
    .id;

    let batch = export_pending_batch_with_session_key(&a, "dev_a", &session_key)
        .expect("export via session key")
        .expect("pending events");
    push_batch(&c, vault_id, &batch).await;

    let b = fresh_vault();
    let downloaded = pull_all_batches(&c, vault_id).await;
    let outcome = ingest_batch_with_session_key(&b, "dev_b", &session_key, &downloaded[0])
        .expect("ingest via session key");
    assert_eq!(outcome.applied, 1);
    assert_eq!(outcome.conflicts, 0);

    let got = get_account(&b, &acct, &ListOptions::default())
        .unwrap()
        .expect("account present on B");
    assert_eq!(got.name, "Assets:Checking");
}

// ── 3. Concurrent divergent edits surface a conflict (ADR-003) ───────────────

/// Build a `TransactionPayload` from a materialized transaction, applying a
/// mutation closure so two devices can craft genuinely *different* edits to the
/// same entity. Bumps `version` so the event represents an update.
fn edited_transaction_payload(
    txn: &Transaction,
    new_version: i64,
    mutate: impl FnOnce(&mut TransactionPayload),
) -> TransactionPayload {
    let mut p = TransactionPayload {
        id: txn.id.clone(),
        date: txn.date.clone(),
        status: txn.status.as_str().to_string(),
        code: txn.code.clone(),
        description: txn.description.clone(),
        comment: txn.comment.clone(),
        created_at: txn.created_at.clone(),
        modified_at: txn.modified_at.clone(),
        postings: txn
            .postings
            .iter()
            .map(|p| PostingPayload {
                id: p.id.clone(),
                account_id: p.account_id.clone(),
                amount_quantity: p.amount_quantity.clone(),
                amount_commodity: p.amount_commodity.clone(),
                balance_assertion_quantity: p.balance_assertion_quantity.clone(),
                balance_assertion_commodity: p.balance_assertion_commodity.clone(),
                created_at: p.created_at.clone(),
                version: new_version,
            })
            .collect(),
    };
    mutate(&mut p);
    p
}

/// Inject an `update` event for `txn` into `conn`'s outbox so the real
/// [`export_pending_batch`] picks it up. (The storage layer has no
/// `update_transaction_with_sync` emitter yet, so divergent edits are injected
/// at the outbox boundary — still flowing through the real compose/apply
/// pipeline on both ends.)
fn inject_txn_update(
    conn: &Connection,
    device: &str,
    payload: &TransactionPayload,
    lamport: u64,
    version: i64,
) {
    let bytes = payload::to_bytes(payload).expect("serialize transaction payload");
    record_event(
        conn,
        device,
        "transaction",
        &payload.id,
        "update",
        &bytes,
        lamport,
        u32::try_from(version).expect("version fits u32"),
    )
    .expect("record update event");
}

#[tokio::test]
async fn concurrent_divergent_edits_surface_conflict_not_lww() {
    let vk = VaultKey::generate();
    let vault_id = "vault-conflict";
    let c = logged_in_client(vault_id).await;

    // Shared starting point: A seeds a balanced transaction, syncs it to B.
    let a = fresh_vault();
    let (_cash, _food, txn_id) = seed_balanced_books(&a, "dev_a");
    let seed_batch = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
    push_batch(&c, vault_id, &seed_batch).await;
    mark_events_synced(&a, &seed_batch.event_ids).unwrap();

    let b = fresh_vault();
    let seeded = pull_all_batches(&c, vault_id).await;
    ingest_batch(&b, "dev_b", &vk, &seeded[0]).unwrap();
    assert_transaction_balanced(&b, &txn_id);

    // Both devices now know transaction T at version 1. Concurrently, each makes
    // a DIFFERENT edit to T (the classic ADR-003 LWW hazard: amount vs metadata).
    let a_txn = get_transaction(&a, &txn_id, &ListOptions::default())
        .unwrap()
        .unwrap();
    let b_txn = get_transaction(&b, &txn_id, &ListOptions::default())
        .unwrap()
        .unwrap();

    // Device A edits the description (metadata) but keeps it balanced.
    let a_edit = edited_transaction_payload(&a_txn, 2, |p| {
        p.description = "Lunch (A: client dinner)".into();
    });
    inject_txn_update(&a, "dev_a", &a_edit, 2, 2);

    // Device B edits the amounts (still internally balanced on its own).
    let b_edit = edited_transaction_payload(&b_txn, 2, |p| {
        for posting in &mut p.postings {
            if let Some(q) = &posting.amount_quantity {
                if q.starts_with('-') {
                    posting.amount_quantity = Some("-25.00".into());
                } else {
                    posting.amount_quantity = Some("25.00".into());
                }
            }
        }
    });
    inject_txn_update(&b, "dev_b", &b_edit, 2, 2);

    // Each composes its divergent edit and pushes it.
    let a_batch = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
    let b_batch = export_pending_batch(&b, "dev_b", &vk).unwrap().unwrap();
    push_batch(&c, vault_id, &a_batch).await;
    push_batch(&c, vault_id, &b_batch).await;

    // B ingests A's divergent edit while holding its own pending edit to T.
    let b_before = get_transaction(&b, &txn_id, &ListOptions::default())
        .unwrap()
        .unwrap();
    let b_outcome = ingest_batch(&b, "dev_b", &vk, &a_batch.ciphertext).unwrap();

    // Conflict surfaced for review — NOT auto-applied, NOT silently LWW.
    assert!(
        b_outcome.conflicts >= 1,
        "expected a conflict, got {b_outcome:?}"
    );
    assert_eq!(b_outcome.applied, 0, "conflicting edit must not be applied");

    // A `StoredConflict` is persisted with both sides recorded and unresolved.
    assert_eq!(unresolved_conflict_count(&b).unwrap(), 1);
    let conflicts = list_unresolved_conflicts(&b).unwrap();
    assert_eq!(conflicts.len(), 1);
    let conflict = &conflicts[0];
    assert_eq!(conflict.entity_type, "transaction");
    assert_eq!(conflict.entity_id, txn_id);
    assert_ne!(
        conflict.local_event_id, conflict.remote_event_id,
        "conflict must reference distinct local + remote events"
    );
    assert!(!conflict.resolved);
    assert!(conflict.resolution.is_none());
    assert!(
        !conflict.local_payload.is_empty() && !conflict.remote_payload.is_empty(),
        "both payloads retained so a human can review and choose"
    );
    // The remote side's operation + version are now captured (issue #211), so a
    // client can faithfully re-apply the remote payload — not just keep local.
    assert_eq!(conflict.remote_operation, "update");
    assert_eq!(conflict.remote_version, 2);

    // B's local view of T was NOT overwritten by the remote edit.
    let b_after = get_transaction(&b, &txn_id, &ListOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(b_before.version, b_after.version);
    assert_eq!(b_before.description, b_after.description);

    // Post-merge invariant (ADR-003 §"Post-merge validation"): the materialized
    // transaction still balances — the conflict was surfaced, not auto-merged
    // into an unbalanced state.
    assert_transaction_balanced(&b, &txn_id);

    // The reverse ingest (A applies B's batch) is symmetric: also a conflict.
    let a_outcome = ingest_batch(&a, "dev_a", &vk, &b_batch.ciphertext).unwrap();
    assert!(a_outcome.conflicts >= 1, "reverse direction also conflicts");
    assert_eq!(a_outcome.applied, 0);
    assert_eq!(unresolved_conflict_count(&a).unwrap(), 1);
    assert_transaction_balanced(&a, &txn_id);

    // ── Issue #211: A resolves the conflict by KEEPING REMOTE (B's edit). ──
    // A's own edit (description) must be superseded by B's amount edit, and the
    // resolution must be re-broadcast as a new pending event so every device
    // converges — not just kept locally.
    let a_conflict = &list_unresolved_conflicts(&a).unwrap()[0];
    let a_conflict_id = a_conflict.id.clone();
    resolve_conflict_keep_remote(&a, "dev_a", &a_conflict_id).unwrap();

    // The conflict is now resolved as "remote".
    assert_eq!(unresolved_conflict_count(&a).unwrap(), 0);
    let resolved = get_conflict(&a, &a_conflict_id).unwrap().unwrap();
    assert!(resolved.resolved);
    assert_eq!(resolved.resolution.as_deref(), Some("remote"));

    // A now shows B's version, at a winning version strictly above both sides,
    // and it still balances (ADR-003 post-merge invariant).
    let a_resolved = get_transaction(&a, &txn_id, &ListOptions::default())
        .unwrap()
        .unwrap();
    assert!(
        a_resolved.version > 2,
        "keep-remote must land above both concurrent v2 edits"
    );
    assert_ne!(
        a_resolved.description, "Lunch (A: client dinner)",
        "A's local description edit must be overwritten by the remote side"
    );
    assert_transaction_balanced(&a, &txn_id);

    // Exactly one winning event is queued so the resolution propagates.
    let a_pending = pending_events(&a).unwrap();
    assert_eq!(a_pending.len(), 1);
    assert_eq!(a_pending[0].entity_id, txn_id);
    assert_eq!(a_pending[0].operation, "update");
    assert_eq!(i64::from(a_pending[0].version), a_resolved.version);
}

// ── 4. Fresh device onboards to consistency via batch replay ─────────────────

#[tokio::test]
async fn fresh_device_onboards_to_consistency_via_replay() {
    // NOTE (scope): real materialized-state *snapshot* onboarding is not wired
    // into the pipeline yet; a late-joining device reaches consistency by
    // replaying every event batch, which is what we assert here.
    let vk = VaultKey::generate();
    let vault_id = "vault-onboard";
    let c = logged_in_client(vault_id).await;

    // Device A produces history across several separate batches.
    let a = fresh_vault();
    let (_cash, _food, txn_id) = seed_balanced_books(&a, "dev_a");
    let batch1 = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
    push_batch(&c, vault_id, &batch1).await;
    mark_events_synced(&a, &batch1.event_ids).unwrap();

    let extra = create_account_with_sync(
        &a,
        &NewAccount {
            name: "Liabilities:Card".into(),
            account_type: AccountType::Liability,
            commodity: Some("USD".into()),
            parent_id: None,
            note: None,
        },
        &ctx("dev_a", 4),
    )
    .unwrap()
    .id;
    let batch2 = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
    push_batch(&c, vault_id, &batch2).await;
    mark_events_synced(&a, &batch2.event_ids).unwrap();

    // A brand-new device C downloads and replays the full batch history.
    let fresh = fresh_vault();
    let history = pull_all_batches(&c, vault_id).await;
    assert_eq!(history.len(), 2, "two batches of history on the server");

    let mut total_applied = 0u32;
    for blob in &history {
        let outcome = ingest_batch(&fresh, "dev_fresh", &vk, blob).unwrap();
        assert_eq!(outcome.conflicts, 0);
        total_applied += outcome.applied;
    }
    assert_eq!(total_applied, 4, "2 accounts + 1 txn + 1 extra account");

    // The fresh device is now consistent with A.
    let a_accounts = list_accounts(&a, &ListOptions::default()).unwrap();
    let fresh_accounts = list_accounts(&fresh, &ListOptions::default()).unwrap();
    assert_eq!(a_accounts.len(), fresh_accounts.len());
    assert!(
        get_account(&fresh, &extra, &ListOptions::default())
            .unwrap()
            .is_some()
    );
    assert_transaction_balanced(&fresh, &txn_id);
}
