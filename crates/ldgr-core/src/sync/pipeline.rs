//! Batch-blob compose/apply pipeline: pending events ↔ encrypted batch blob.
//!
//! This is the composition layer that ties together the existing sync
//! primitives — it adds no new crypto, merge, or conflict policy. It is pure
//! computation: events + [`VaultKey`] in → ciphertext out; ciphertext +
//! [`VaultKey`] + local `SQLite` state in → applied count + persisted conflicts
//! out. The only I/O surface is the passed-in [`Connection`] (the canonical
//! vault); there is no networking or filesystem access, so the same code is
//! reused by the CLI (reqwest), FFI (Swift), and any future WASM-storage host.
//!
//! - [`export_pending_batch`] composes all currently-pending outbox events into
//!   one encrypted blob ready for upload. It does **not** mark events synced —
//!   the caller does that after a successful upload, using the returned
//!   `event_ids`.
//! - [`ingest_batch`] decrypts a downloaded blob, three-way merges it against
//!   local state via [`merge_events`], applies the cleanly-merged events to the
//!   canonical tables, persists conflicts for user review, and advances the
//!   local vector/Lamport clocks.
//!
//! The on-the-wire blob format is documented in `docs/sync-blob-format.md`.

use rusqlite::Connection;
use thiserror::Error;
use uuid::Uuid;

use crate::crypto::{CryptoError, SealedEnvelope, VaultKey, decrypt_item, encrypt_item};
use crate::storage::error::StorageError;
use crate::storage::sync as sync_store;
use crate::storage::{accounts, transactions};

use super::conflicts::merge_events;
use super::events::{
    EntityType, EventBatch, Operation, SyncEvent, VectorClock, create_batch, deserialize_batch,
    serialize_batch, total_order,
};
use super::payload::{self, AccountPayload, DeletePayload, TransactionPayload};

/// `sync_state` key holding the persisted local vector clock (JSON).
const VECTOR_CLOCK_KEY: &str = "sync:vector_clock";

/// Errors from the compose/apply pipeline.
#[derive(Debug, Error)]
pub enum PipelineError {
    /// A canonical-store read or write failed.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// Decryption or key unwrapping failed (wrong vault key, tampered blob).
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),

    /// The blob, batch, or event payload could not be (de)serialized, or an
    /// outbox row held an unrecognized entity-type/operation string.
    #[error("blob format error: {0}")]
    Format(String),

    /// The batch referenced an entity type that has no storage module yet
    /// (`price`/`budget`/`goal`). Tracked by issue #203.
    #[error("unsupported entity type for apply: {0}")]
    UnsupportedEntity(String),
}

/// A composed, encrypted batch blob ready for upload.
///
/// After a successful upload the caller must mark [`Self::event_ids`] synced via
/// [`crate::storage::sync::mark_events_synced`]; the pipeline never marks them
/// itself (an upload could fail).
#[derive(Debug, Clone)]
pub struct ExportedBatch {
    /// Random id for the blob (suitable as the `{batch}.enc` filename).
    pub batch_id: String,
    /// The device that produced the batch.
    pub device_id: String,
    /// The canonical encrypted blob bytes (see `docs/sync-blob-format.md`).
    pub ciphertext: Vec<u8>,
    /// Ids of the outbox events included — mark these synced after upload.
    pub event_ids: Vec<String>,
}

/// Outcome of applying a downloaded batch blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestOutcome {
    /// Events applied cleanly to the canonical tables.
    pub applied: u32,
    /// Conflicts detected and persisted for user review.
    pub conflicts: u32,
    /// Events skipped as already-seen or stale (no-op).
    pub skipped: u32,
}

/// Compose all currently-pending events for `device_id` into a single encrypted
/// batch blob. Returns `Ok(None)` if the outbox is empty.
///
/// The returned [`ExportedBatch::ciphertext`] is the canonical blob; the caller
/// uploads it and then marks [`ExportedBatch::event_ids`] synced.
pub fn export_pending_batch(
    conn: &Connection,
    device_id: &str,
    vault_key: &VaultKey,
) -> Result<Option<ExportedBatch>, PipelineError> {
    let stored = sync_store::pending_events(conn)?;
    if stored.is_empty() {
        return Ok(None);
    }

    let mut events = Vec::with_capacity(stored.len());
    let mut event_ids = Vec::with_capacity(stored.len());
    for s in &stored {
        event_ids.push(s.id.clone());
        events.push(stored_to_event(s)?);
    }

    let clock = load_local_clock(conn, device_id)?;
    persist_local_clock(conn, &clock)?;

    let batch = create_batch(device_id, events, &clock);
    let ciphertext = seal_batch(vault_key, &batch)?;

    Ok(Some(ExportedBatch {
        batch_id: Uuid::now_v7().to_string(),
        device_id: device_id.to_string(),
        ciphertext,
        event_ids,
    }))
}

/// Apply a downloaded encrypted batch blob against local state.
///
/// Decrypts → deserializes → three-way merges (via [`merge_events`]) → applies
/// the cleanly-merged events to the canonical tables → persists conflicts →
/// advances and persists the local vector/Lamport clocks. The whole apply is
/// performed inside one `SQLite` transaction, so it is atomic.
///
/// Idempotent: re-ingesting the same (or an older) blob is a no-op because the
/// merged vector clock dominates the batch's clock, so [`merge_events`] skips
/// every event.
pub fn ingest_batch(
    conn: &Connection,
    local_device_id: &str,
    vault_key: &VaultKey,
    ciphertext: &[u8],
) -> Result<IngestOutcome, PipelineError> {
    let batch = open_batch(vault_key, ciphertext)?;

    let local_stored = sync_store::pending_events(conn)?;
    let mut local_pending = Vec::with_capacity(local_stored.len());
    for s in &local_stored {
        local_pending.push(stored_to_event(s)?);
    }

    let local_clock = load_local_clock(conn, local_device_id)?;
    let remote_clock = batch.vector_clock.clone();
    let now = chrono::Utc::now().to_rfc3339();

    let merge = merge_events(
        &local_pending,
        &batch.events,
        &local_clock,
        &remote_clock,
        &now,
    );

    // Apply atomically: entity writes + conflict rows + clock advances.
    let tx = conn.unchecked_transaction().map_err(StorageError::from)?;

    let mut applied_sorted = merge.applied.clone();
    applied_sorted.sort_by(total_order);

    let mut applied = 0u32;
    let mut stale = 0u32;
    for ev in &applied_sorted {
        if apply_event(&tx, ev)? {
            applied += 1;
        } else {
            stale += 1;
        }
    }

    let stored_conflicts: Vec<sync_store::StoredConflict> =
        merge.conflicts.iter().map(conflict_to_stored).collect();
    sync_store::store_conflicts(&tx, &stored_conflicts)?;

    let mut new_clock = local_clock;
    new_clock.merge(&remote_clock);
    persist_local_clock(&tx, &new_clock)?;

    if let Some(max_remote) = batch.events.iter().map(|e| e.lamport_clock).max() {
        sync_store::observe_lamport(&tx, max_remote)?;
    }

    tx.commit().map_err(StorageError::from)?;

    let conflicts = u32::try_from(merge.conflicts.len()).unwrap_or(u32::MAX);
    let skipped = u32::try_from(merge.skipped)
        .unwrap_or(u32::MAX)
        .saturating_add(stale);

    Ok(IngestOutcome {
        applied,
        conflicts,
        skipped,
    })
}

// ── Session-key convenience entry points ─────────────────────────────────────
//
// The [`VaultKey`] type is deliberately non-constructible outside `ldgr-core`
// (`VaultKey::from_bytes` is `pub(crate)`), which keeps the sensitive key type
// encapsulated. FFI / WASM hosts only ever hold the raw 32-byte session key
// (exported via `UnlockedVault::export_session_key`), so these thin wrappers
// rebuild the [`VaultKey`] inside the crate and delegate to the canonical
// pipeline functions above. No additional crypto is performed here.

/// Like [`export_pending_batch`], but accepting the raw 32-byte vault session
/// key (as exported by `UnlockedVault::export_session_key`) instead of a
/// [`VaultKey`]. Intended for FFI/WASM hosts that cannot construct a
/// [`VaultKey`] directly.
pub fn export_pending_batch_with_session_key(
    conn: &Connection,
    device_id: &str,
    session_key: &[u8; 32],
) -> Result<Option<ExportedBatch>, PipelineError> {
    export_pending_batch(conn, device_id, &VaultKey::from_bytes(*session_key))
}

/// Like [`ingest_batch`], but accepting the raw 32-byte vault session key
/// instead of a [`VaultKey`]. Intended for FFI/WASM hosts.
pub fn ingest_batch_with_session_key(
    conn: &Connection,
    local_device_id: &str,
    session_key: &[u8; 32],
    ciphertext: &[u8],
) -> Result<IngestOutcome, PipelineError> {
    ingest_batch(
        conn,
        local_device_id,
        &VaultKey::from_bytes(*session_key),
        ciphertext,
    )
}

// ── Blob framing ─────────────────────────────────────────────────────────────

/// Seal a batch into the canonical blob: `json(encrypt_item(vk, json(batch)))`.
fn seal_batch(vault_key: &VaultKey, batch: &EventBatch) -> Result<Vec<u8>, PipelineError> {
    let plaintext = serialize_batch(batch).map_err(PipelineError::Format)?;
    let envelope = encrypt_item(vault_key, &plaintext)?;
    serde_json::to_vec(&envelope)
        .map_err(|e| PipelineError::Format(format!("failed to serialize sealed envelope: {e}")))
}

/// Inverse of [`seal_batch`]: decrypt and deserialize a canonical blob.
fn open_batch(vault_key: &VaultKey, ciphertext: &[u8]) -> Result<EventBatch, PipelineError> {
    let envelope: SealedEnvelope = serde_json::from_slice(ciphertext)
        .map_err(|e| PipelineError::Format(format!("failed to parse sealed envelope: {e}")))?;
    let plaintext = decrypt_item(vault_key, &envelope)?;
    deserialize_batch(&plaintext).map_err(PipelineError::Format)
}

// ── Apply dispatch ───────────────────────────────────────────────────────────

/// Apply one cleanly-merged remote event to the canonical tables.
///
/// Returns `true` if the entity row was written, `false` if the event was stale
/// (older/equal version) or a delete for an unknown entity. Never records a
/// `sync_events` outbox row — applied remote events must not echo into our own
/// outbox.
fn apply_event(conn: &Connection, ev: &SyncEvent) -> Result<bool, PipelineError> {
    let version = i64::from(ev.version);
    match ev.entity_type {
        EntityType::Account => match ev.operation {
            Operation::Create | Operation::Update => {
                let p: AccountPayload = payload::from_bytes(&ev.payload)
                    .map_err(|e| PipelineError::Format(format!("account payload: {e}")))?;
                Ok(accounts::apply_remote_account(conn, &p, version)?)
            }
            Operation::Delete => {
                let p: DeletePayload = payload::from_bytes(&ev.payload)
                    .map_err(|e| PipelineError::Format(format!("delete payload: {e}")))?;
                Ok(accounts::apply_remote_account_delete(conn, &p.id, version)?)
            }
        },
        EntityType::Transaction => match ev.operation {
            Operation::Create | Operation::Update => {
                let p: TransactionPayload = payload::from_bytes(&ev.payload)
                    .map_err(|e| PipelineError::Format(format!("transaction payload: {e}")))?;
                Ok(transactions::apply_remote_transaction(conn, &p, version)?)
            }
            Operation::Delete => {
                let p: DeletePayload = payload::from_bytes(&ev.payload)
                    .map_err(|e| PipelineError::Format(format!("delete payload: {e}")))?;
                Ok(transactions::apply_remote_transaction_delete(
                    conn, &p.id, version,
                )?)
            }
        },
        // Price/Budget/Goal have no storage module yet — fail closed so we never
        // silently drop financial data. Tracked by #203.
        other => Err(PipelineError::UnsupportedEntity(other.as_str().to_string())),
    }
}

// ── Conversions & clock helpers ──────────────────────────────────────────────

/// Convert an outbox row into a [`SyncEvent`], surfacing a clear error on a
/// corrupt entity-type/operation string.
fn stored_to_event(s: &sync_store::StoredSyncEvent) -> Result<SyncEvent, PipelineError> {
    let entity_type = EntityType::parse_str(&s.entity_type)
        .ok_or_else(|| PipelineError::Format(format!("unknown entity type: {}", s.entity_type)))?;
    let operation = Operation::parse_str(&s.operation)
        .ok_or_else(|| PipelineError::Format(format!("unknown operation: {}", s.operation)))?;
    Ok(SyncEvent {
        id: s.id.clone(),
        device_id: s.device_id.clone(),
        lamport_clock: s.lamport_clock,
        entity_type,
        entity_id: s.entity_id.clone(),
        operation,
        payload: s.payload.clone(),
        version: s.version,
        created_at: s.created_at.clone(),
    })
}

/// Map a detected [`super::conflicts::SyncConflict`] onto its persisted form.
fn conflict_to_stored(c: &super::conflicts::SyncConflict) -> sync_store::StoredConflict {
    sync_store::StoredConflict {
        id: c.id.clone(),
        entity_type: c.entity_type.clone(),
        entity_id: c.entity_id.clone(),
        local_event_id: c.local_event.id.clone(),
        remote_event_id: c.remote_event.id.clone(),
        local_payload: c.local_event.payload.clone(),
        remote_payload: c.remote_event.payload.clone(),
        detected_at: c.detected_at.clone(),
        resolved: false,
        resolution: None,
    }
}

/// Load the local vector clock: persisted knowledge of other devices merged
/// with this device's own component (the count of events it has emitted).
fn load_local_clock(conn: &Connection, device_id: &str) -> Result<VectorClock, PipelineError> {
    let mut clock = match sync_store::get_state(conn, VECTOR_CLOCK_KEY)? {
        Some(json) => serde_json::from_str(&json)
            .map_err(|e| PipelineError::Format(format!("corrupt vector clock: {e}")))?,
        None => VectorClock::default(),
    };

    let own = sync_store::device_event_count(conn, device_id)?;
    let entry = clock.clocks.entry(device_id.to_string()).or_insert(0);
    *entry = (*entry).max(own);

    Ok(clock)
}

/// Persist the local vector clock as JSON in `sync_state`.
fn persist_local_clock(conn: &Connection, clock: &VectorClock) -> Result<(), PipelineError> {
    let json = serde_json::to_string(clock)
        .map_err(|e| PipelineError::Format(format!("failed to serialize vector clock: {e}")))?;
    sync_store::set_state(conn, VECTOR_CLOCK_KEY, &json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::VaultKey;
    use crate::storage::accounts::{
        AccountType, ListOptions, NewAccount, create_account_with_sync, get_account, list_accounts,
        soft_delete_account_with_sync,
    };
    use crate::storage::schema;
    use crate::storage::sync::{SyncContext, mark_events_synced, pending_event_count};
    use crate::storage::transactions::{
        NewPosting, NewTransaction, TransactionStatus, create_transaction_with_sync,
        get_transaction,
    };

    fn vault() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        schema::initialize(&conn).unwrap();
        conn
    }

    fn ctx(device: &str, lamport: u64) -> SyncContext {
        SyncContext {
            device_id: device.into(),
            lamport_clock: lamport,
        }
    }

    fn seed_account(conn: &Connection, device: &str, lamport: u64, name: &str) -> String {
        create_account_with_sync(
            conn,
            &NewAccount {
                name: name.into(),
                account_type: AccountType::Asset,
                commodity: Some("USD".into()),
                parent_id: None,
                note: Some("seed note".into()),
            },
            &ctx(device, lamport),
        )
        .unwrap()
        .id
    }

    #[test]
    fn export_empty_outbox_is_none() {
        let conn = vault();
        let vk = VaultKey::generate();
        assert!(export_pending_batch(&conn, "dev_a", &vk).unwrap().is_none());
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn round_trip_reproduces_entities_on_second_vault() {
        let vk = VaultKey::generate();

        // Device A: two accounts (one a child of the other) + a transaction with
        // a code, comment, and a balance assertion.
        let a = vault();
        let cash = create_account_with_sync(
            &a,
            &NewAccount {
                name: "Assets:Cash".into(),
                account_type: AccountType::Asset,
                commodity: Some("USD".into()),
                parent_id: None,
                note: Some("petty cash".into()),
            },
            &ctx("dev_a", 1),
        )
        .unwrap();
        let food = create_account_with_sync(
            &a,
            &NewAccount {
                name: "Expenses:Food".into(),
                account_type: AccountType::Expense,
                commodity: Some("USD".into()),
                parent_id: Some(cash.id.clone()),
                note: None,
            },
            &ctx("dev_a", 2),
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
                        balance_assertion_quantity: Some("90.00".into()),
                        balance_assertion_commodity: Some("USD".into()),
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
            &ctx("dev_a", 3),
        )
        .unwrap();

        let exported = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();

        // Device B: fresh vault, same key, no shared state.
        let b = vault();
        let outcome = ingest_batch(&b, "dev_b", &vk, &exported.ciphertext).unwrap();
        assert_eq!(outcome.applied, 3);
        assert_eq!(outcome.conflicts, 0);

        // Accounts reproduced field-for-field.
        let a_accounts = list_accounts(&a, &ListOptions::default()).unwrap();
        let b_accounts = list_accounts(&b, &ListOptions::default()).unwrap();
        assert_eq!(a_accounts.len(), b_accounts.len());
        for (ax, bx) in a_accounts.iter().zip(b_accounts.iter()) {
            assert_eq!(ax.id, bx.id);
            assert_eq!(ax.name, bx.name);
            assert_eq!(ax.account_type, bx.account_type);
            assert_eq!(ax.commodity, bx.commodity);
            assert_eq!(ax.parent_id, bx.parent_id);
            assert_eq!(ax.note, bx.note);
            assert_eq!(ax.created_at, bx.created_at);
            assert_eq!(ax.modified_at, bx.modified_at);
            assert_eq!(ax.version, bx.version);
            assert_eq!(ax.deleted, bx.deleted);
        }

        // Transaction (with postings) reproduced field-for-field.
        let at = get_transaction(&a, &txn.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        let bt = get_transaction(&b, &txn.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(at.id, bt.id);
        assert_eq!(at.date, bt.date);
        assert_eq!(at.status, bt.status);
        assert_eq!(at.code, bt.code);
        assert_eq!(at.description, bt.description);
        assert_eq!(at.comment, bt.comment);
        assert_eq!(at.created_at, bt.created_at);
        assert_eq!(at.modified_at, bt.modified_at);
        assert_eq!(at.version, bt.version);
        assert_eq!(at.postings.len(), bt.postings.len());
        for (ap, bp) in at.postings.iter().zip(bt.postings.iter()) {
            assert_eq!(ap.id, bp.id);
            assert_eq!(ap.account_id, bp.account_id);
            assert_eq!(ap.amount_quantity, bp.amount_quantity);
            assert_eq!(ap.amount_commodity, bp.amount_commodity);
            assert_eq!(ap.balance_assertion_quantity, bp.balance_assertion_quantity);
            assert_eq!(
                ap.balance_assertion_commodity,
                bp.balance_assertion_commodity
            );
            assert_eq!(ap.posting_order, bp.posting_order);
            assert_eq!(ap.created_at, bp.created_at);
            assert_eq!(ap.version, bp.version);
        }

        // Apply did not echo into B's outbox.
        assert_eq!(pending_event_count(&b).unwrap(), 0);
    }

    #[test]
    fn session_key_entry_points_round_trip() {
        // The FFI/WASM seam never holds a `VaultKey` (it is non-constructible
        // outside this crate); it only has the raw 32-byte session key. Verify
        // the `*_with_session_key` wrappers reproduce the entity on a second
        // device exactly like the `VaultKey`-typed path, and reject a wrong key.
        let vk = VaultKey::generate();
        let session_key = *vk.as_bytes();

        let a = vault();
        let acct = seed_account(&a, "dev_a", 1, "Assets:Checking");

        let exported = export_pending_batch_with_session_key(&a, "dev_a", &session_key)
            .unwrap()
            .expect("a pending batch");
        assert_eq!(exported.device_id, "dev_a");
        assert!(!exported.event_ids.is_empty());

        let b = vault();
        let outcome =
            ingest_batch_with_session_key(&b, "dev_b", &session_key, &exported.ciphertext).unwrap();
        assert_eq!(outcome.applied, 1);
        assert_eq!(outcome.conflicts, 0);

        let got = get_account(&b, &acct, &ListOptions::default())
            .unwrap()
            .expect("account present on B");
        assert_eq!(got.name, "Assets:Checking");

        // A wrong session key must fail to decrypt — never silently accepted.
        let wrong = *VaultKey::generate().as_bytes();
        let c = vault();
        assert!(ingest_batch_with_session_key(&c, "dev_c", &wrong, &exported.ciphertext).is_err());
    }

    #[test]
    fn ingest_is_idempotent() {
        let vk = VaultKey::generate();
        let a = vault();
        seed_account(&a, "dev_a", 1, "Assets:Cash");
        let exported = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();

        let b = vault();
        let first = ingest_batch(&b, "dev_b", &vk, &exported.ciphertext).unwrap();
        assert_eq!(first.applied, 1);

        // Re-ingesting the exact same blob applies nothing (vector-clock skip).
        let second = ingest_batch(&b, "dev_b", &vk, &exported.ciphertext).unwrap();
        assert_eq!(second.applied, 0);

        // Still exactly one account, unchanged.
        assert_eq!(list_accounts(&b, &ListOptions::default()).unwrap().len(), 1);
    }

    #[test]
    fn cross_direction_blobs_are_interchangeable() {
        let vk = VaultKey::generate();

        // A creates account X; B ingests it.
        let a = vault();
        let x = seed_account(&a, "dev_a", 1, "Assets:Cash");
        let from_a = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
        let b = vault();
        ingest_batch(&b, "dev_b", &vk, &from_a.ciphertext).unwrap();
        mark_events_synced(&a, &from_a.event_ids).unwrap();

        // B creates its own account Y; A ingests B's blob (reverse direction).
        let y = seed_account(&b, "dev_b", 1, "Assets:Bank");
        let from_b = export_pending_batch(&b, "dev_b", &vk).unwrap().unwrap();
        let outcome = ingest_batch(&a, "dev_a", &vk, &from_b.ciphertext).unwrap();
        assert_eq!(outcome.applied, 1);

        // A now has both accounts.
        assert!(
            get_account(&a, &x, &ListOptions::default())
                .unwrap()
                .is_some()
        );
        assert!(
            get_account(&a, &y, &ListOptions::default())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn delete_propagates() {
        let vk = VaultKey::generate();
        let a = vault();
        let id = seed_account(&a, "dev_a", 1, "Assets:Cash");
        let create_blob = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
        mark_events_synced(&a, &create_blob.event_ids).unwrap();

        let b = vault();
        ingest_batch(&b, "dev_b", &vk, &create_blob.ciphertext).unwrap();
        assert!(
            get_account(&b, &id, &ListOptions::default())
                .unwrap()
                .is_some()
        );

        // A deletes the account, exports, B ingests the delete.
        soft_delete_account_with_sync(&a, &id, &ctx("dev_a", 2)).unwrap();
        let delete_blob = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
        let outcome = ingest_batch(&b, "dev_b", &vk, &delete_blob.ciphertext).unwrap();
        assert_eq!(outcome.applied, 1);
        assert!(
            get_account(&b, &id, &ListOptions::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn concurrent_edit_same_entity_surfaces_conflict() {
        let vk = VaultKey::generate();

        // A creates account X; B ingests it (now both know X).
        let a = vault();
        let x = seed_account(&a, "dev_a", 1, "Assets:Cash");
        let create_blob = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
        mark_events_synced(&a, &create_blob.event_ids).unwrap();
        let b = vault();
        ingest_batch(&b, "dev_b", &vk, &create_blob.ciphertext).unwrap();

        // Concurrently: A deletes X and exports; B also deletes X locally
        // (a pending, unsynced edit to the same entity).
        soft_delete_account_with_sync(&a, &x, &ctx("dev_a", 2)).unwrap();
        let a_delete = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();
        soft_delete_account_with_sync(&b, &x, &ctx("dev_b", 2)).unwrap();

        let before = get_account(&b, &x, &ListOptions::default()).unwrap();
        let outcome = ingest_batch(&b, "dev_b", &vk, &a_delete.ciphertext).unwrap();

        // Same entity touched on both devices → conflict, not a silent apply.
        assert_eq!(outcome.conflicts, 1);
        assert_eq!(outcome.applied, 0);
        assert_eq!(
            crate::storage::sync::unresolved_conflict_count(&b).unwrap(),
            1
        );

        // B's local view of X was not overwritten by the remote event.
        let after = get_account(&b, &x, &ListOptions::default()).unwrap();
        assert_eq!(before.map(|v| v.version), after.map(|v| v.version));
    }

    #[test]
    fn wrong_vault_key_fails_to_ingest() {
        let vk = VaultKey::generate();
        let other = VaultKey::generate();
        let a = vault();
        seed_account(&a, "dev_a", 1, "Assets:Cash");
        let exported = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();

        let b = vault();
        let err = ingest_batch(&b, "dev_b", &other, &exported.ciphertext).unwrap_err();
        assert!(matches!(err, PipelineError::Crypto(_)));
    }

    #[test]
    fn blob_is_canonical_sealed_envelope_framing() {
        let vk = VaultKey::generate();
        let a = vault();
        seed_account(&a, "dev_a", 1, "Assets:Cash");
        let exported = export_pending_batch(&a, "dev_a", &vk).unwrap().unwrap();

        // Blob is exactly a JSON-serialized SealedEnvelope whose plaintext is a
        // JSON-serialized EventBatch.
        let envelope: SealedEnvelope = serde_json::from_slice(&exported.ciphertext).unwrap();
        let plaintext = decrypt_item(&vk, &envelope).unwrap();
        let batch = deserialize_batch(&plaintext).unwrap();
        assert_eq!(batch.device_id, "dev_a");
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].entity_type, EntityType::Account);
    }

    #[test]
    fn corrupt_blob_is_a_format_error() {
        let vk = VaultKey::generate();
        let b = vault();
        let err = ingest_batch(&b, "dev_b", &vk, b"not a sealed envelope").unwrap_err();
        assert!(matches!(err, PipelineError::Format(_)));
    }

    #[test]
    fn unsupported_entity_type_fails_closed() {
        // Hand-build a batch with a Price event and seal it; ingest must error.
        let vk = VaultKey::generate();
        let ev = SyncEvent {
            id: "evt".into(),
            device_id: "dev_a".into(),
            lamport_clock: 1,
            entity_type: EntityType::Price,
            entity_id: "p1".into(),
            operation: Operation::Create,
            payload: b"{}".to_vec(),
            version: 1,
            created_at: "2024-01-15T00:00:00Z".into(),
        };
        let mut clock = VectorClock::default();
        clock.tick("dev_a");
        let batch = create_batch("dev_a", vec![ev], &clock);
        let ciphertext = super::seal_batch(&vk, &batch).unwrap();

        let b = vault();
        let err = ingest_batch(&b, "dev_b", &vk, &ciphertext).unwrap_err();
        assert!(matches!(err, PipelineError::UnsupportedEntity(_)));
    }

    // ── apply_event: Update / staleness / unknown-delete ─────────────────────
    // The outbox has no `update_*_with_sync` emitter yet, so Update and
    // staleness paths are exercised by fabricating events directly.

    fn account_event(id: &str, op: Operation, version: u32, name: &str) -> SyncEvent {
        let bytes = payload::to_bytes(&AccountPayload {
            id: id.into(),
            name: name.into(),
            account_type: "asset".into(),
            commodity: Some("USD".into()),
            parent_id: None,
            note: None,
            created_at: "2024-01-15T00:00:00Z".into(),
            modified_at: "2024-01-15T00:00:00Z".into(),
        })
        .unwrap();
        SyncEvent {
            id: format!("evt-{id}-{version}"),
            device_id: "dev_a".into(),
            lamport_clock: u64::from(version),
            entity_type: EntityType::Account,
            entity_id: id.into(),
            operation: op,
            payload: bytes,
            version,
            created_at: "2024-01-15T00:00:00Z".into(),
        }
    }

    #[test]
    fn apply_update_supersedes_lower_version() {
        let conn = vault();
        assert!(
            apply_event(
                &conn,
                &account_event("X", Operation::Create, 1, "Assets:Cash")
            )
            .unwrap()
        );
        // Higher-version update renames the account.
        assert!(
            apply_event(
                &conn,
                &account_event("X", Operation::Update, 2, "Assets:Wallet")
            )
            .unwrap()
        );
        let acc = get_account(&conn, "X", &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(acc.name, "Assets:Wallet");
        assert_eq!(acc.version, 2);
    }

    #[test]
    fn apply_stale_event_is_skipped() {
        let conn = vault();
        assert!(
            apply_event(
                &conn,
                &account_event("X", Operation::Create, 2, "Assets:Wallet")
            )
            .unwrap()
        );
        // An older-version event for the same entity is stale → skipped.
        assert!(
            !apply_event(
                &conn,
                &account_event("X", Operation::Update, 1, "Assets:Cash")
            )
            .unwrap()
        );
        let acc = get_account(&conn, "X", &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(acc.name, "Assets:Wallet");
        assert_eq!(acc.version, 2);
    }

    #[test]
    fn apply_delete_of_unknown_entity_is_noop() {
        let conn = vault();
        let del = SyncEvent {
            id: "evt-del".into(),
            device_id: "dev_a".into(),
            lamport_clock: 5,
            entity_type: EntityType::Account,
            entity_id: "ghost".into(),
            operation: Operation::Delete,
            payload: payload::to_bytes(&DeletePayload { id: "ghost".into() }).unwrap(),
            version: 2,
            created_at: "2024-01-15T00:00:00Z".into(),
        };
        assert!(!apply_event(&conn, &del).unwrap());
        assert!(
            get_account(&conn, "ghost", &ListOptions::default())
                .unwrap()
                .is_none()
        );
    }
}
