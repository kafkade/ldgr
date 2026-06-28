//! Bridge between the CLI blob transport and the core sync **pipeline**.
//!
//! Historically the CLI `sync push`/`pull` commands shuffled `*.enc` files
//! through `sync-outbox/` and `sync-inbox/` directories that nothing else ever
//! touched, so changes never actually synced. These helpers replace that
//! vestigial file-blob model with the real pipeline that iOS/web already use:
//!
//! - **push**: [`ldgr_core::sync::pipeline::export_pending_batch_with_session_key`]
//!   composes the pending `SQLite` outbox events into one encrypted batch blob,
//!   we upload it via the transport, then mark those events synced.
//! - **pull**: we list/download remote batch blobs and feed each through
//!   [`ldgr_core::sync::pipeline::ingest_batch_with_session_key`], which merges
//!   them into the canonical tables and persists conflicts for review.
//!
//! State lives in one place — the vault DB:
//! - push progress = the outbox `synced` flag (`mark_events_synced`);
//! - pull progress = a small `cli_ingested_batches` cursor in `sync_state`
//!   (purely a download optimisation — ingest is idempotent regardless);
//! - `cli_last_sync_at` for status display.
//!
//! The device id is the **DB** device id (`storage::sync::device_id`) so push
//! attribution, self-batch filtering on pull, and the pipeline's vector clock
//! all agree.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use ldgr_core::storage::sync as sync_storage;
use ldgr_core::sync::pipeline::{
    IngestOutcome, export_pending_batch_with_session_key, ingest_batch_with_session_key,
};
use ldgr_core::sync::transport::{
    batch_path, batches_prefix, device_batches_prefix, parse_batch_path,
};

use super::BlobTransport;

/// `sync_state` key holding the JSON array of remote batch ids already ingested.
const INGESTED_BATCHES_KEY: &str = "cli_ingested_batches";
/// `sync_state` key holding the RFC3339 timestamp of the last successful sync.
const LAST_SYNC_AT_KEY: &str = "cli_last_sync_at";
/// Legacy file (pre-unification) that held a CLI-only device id.
const LEGACY_DEVICE_ID_FILE: &str = "device-id";

/// Build a [`sync_storage::SyncContext`] for the next local mutation: the
/// vault's DB device id plus a freshly-ticked Lamport clock.
///
/// Mirrors the FFI `next_sync_context` precedent so every CLI write records an
/// outbox event atomically with the data change.
pub fn cli_sync_context(conn: &Connection) -> Result<sync_storage::SyncContext> {
    let device_id = sync_storage::device_id(conn).context("failed to read device id")?;
    let lamport_clock = sync_storage::tick_lamport(conn).context("failed to tick lamport clock")?;
    Ok(sync_storage::SyncContext {
        device_id,
        lamport_clock,
    })
}

/// Resolve the canonical (DB) device id for this vault, migrating a legacy
/// file-based id on first use.
///
/// Before unification the CLI stored a device id in a `device-id` file while the
/// pipeline used `sync_state`. If the legacy file exists and the DB has no id
/// yet, we seed the DB with the file's value so a device already registered with
/// a server keeps its identity. Thereafter the DB id is authoritative.
pub fn resolve_device_id(conn: &Connection, vault_dir: &Path) -> Result<String> {
    if sync_storage::get_state(conn, "device_id")
        .context("failed to read device id state")?
        .is_none()
    {
        let legacy_path = vault_dir.join(LEGACY_DEVICE_ID_FILE);
        if let Ok(contents) = std::fs::read_to_string(&legacy_path) {
            let legacy = contents.trim();
            if !legacy.is_empty() {
                sync_storage::set_state(conn, "device_id", legacy)
                    .context("failed to migrate legacy device id")?;
            }
        }
    }

    // `device_id` auto-generates into `sync_state` if still unset.
    sync_storage::device_id(conn).context("failed to resolve device id")
}

/// Summary of a `push` run.
#[derive(Debug, Clone, Copy, Default)]
pub struct PushSummary {
    /// Number of batch blobs uploaded (0 or 1 — one batch per push today).
    pub batches_pushed: u32,
    /// Number of outbox events included in the pushed batch.
    pub events_pushed: usize,
}

/// Export the pending outbox as one encrypted batch, upload it, and mark the
/// included events synced.
///
/// Returns an empty summary when there is nothing pending. On a transport
/// `Conflict` (the batch id already exists remotely) the events are still
/// marked synced, since the blob is immutable and present.
pub async fn push_pending(
    conn: &Connection,
    transport: &dyn BlobTransport,
    vault_id: &str,
    device_id: &str,
    session_key: &[u8; 32],
) -> Result<PushSummary> {
    // Best-effort directory provisioning (no-op for object stores).
    transport
        .ensure_directory(&batches_prefix(vault_id))
        .await
        .ok();
    transport
        .ensure_directory(&device_batches_prefix(vault_id, device_id))
        .await
        .ok();

    let Some(batch) = export_pending_batch_with_session_key(conn, device_id, session_key)
        .context("failed to export pending sync batch")?
    else {
        return Ok(PushSummary::default());
    };

    let blob_path = batch_path(vault_id, device_id, &batch.batch_id);
    match transport.put_blob(&blob_path, &batch.ciphertext).await {
        Ok(_) => {}
        Err(e) if e.kind == ldgr_core::sync::TransportErrorKind::Conflict => {
            // Blob already present — immutable, so treat as already-pushed.
        }
        Err(e) => return Err(e).context("failed to upload sync batch"),
    }

    sync_storage::mark_events_synced(conn, &batch.event_ids)
        .context("failed to mark events synced")?;
    set_last_sync_now(conn)?;

    Ok(PushSummary {
        batches_pushed: 1,
        events_pushed: batch.event_ids.len(),
    })
}

/// Summary of a `pull` run.
#[derive(Debug, Clone, Default)]
pub struct PullSummary {
    /// Remote batch blobs downloaded and ingested this run.
    pub batches_ingested: u32,
    /// Aggregated ingest outcome across all batches.
    pub applied: u32,
    pub conflicts: u32,
    pub skipped: u32,
}

/// List remote batch blobs, ingest any not produced by this device and not yet
/// applied, and report the aggregate outcome.
///
/// Ingest is idempotent (vector-clock dominance), so the `cli_ingested_batches`
/// cursor is only an optimisation to avoid re-downloading. Conflicting remote
/// events are persisted for review by the pipeline (local-wins-pending-review).
pub async fn pull_and_apply(
    conn: &Connection,
    transport: &dyn BlobTransport,
    vault_id: &str,
    device_id: &str,
    session_key: &[u8; 32],
) -> Result<PullSummary> {
    let mut ingested = load_ingested_batches(conn)?;

    // Page through the full batch listing.
    let prefix = batches_prefix(vault_id);
    let mut entries = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let result = transport
            .list_blobs(&prefix, cursor.as_deref())
            .await
            .context("failed to list remote batches")?;
        entries.extend(result.entries);
        if !result.has_more {
            break;
        }
        cursor = result.cursor;
    }

    let mut summary = PullSummary::default();
    for entry in &entries {
        let Some(batch_ref) = parse_batch_path(&entry.path) else {
            continue;
        };
        // Skip our own batches and ones we've already applied.
        if batch_ref.device_id == device_id || ingested.contains(&batch_ref.batch_id) {
            continue;
        }

        let blob_path = batch_path(vault_id, &batch_ref.device_id, &batch_ref.batch_id);
        let data = transport
            .get_blob(&blob_path)
            .await
            .with_context(|| format!("failed to download batch {}", batch_ref.batch_id))?;

        let outcome: IngestOutcome =
            ingest_batch_with_session_key(conn, device_id, session_key, &data)
                .with_context(|| format!("failed to ingest batch {}", batch_ref.batch_id))?;

        summary.batches_ingested += 1;
        summary.applied += outcome.applied;
        summary.conflicts += outcome.conflicts;
        summary.skipped += outcome.skipped;

        ingested.push(batch_ref.batch_id.clone());
        // Persist incrementally so a mid-run failure still records progress.
        save_ingested_batches(conn, &ingested)?;
    }

    if summary.batches_ingested > 0 {
        set_last_sync_now(conn)?;
    }

    Ok(summary)
}

/// Timestamp of the last successful push/pull, for status display.
pub fn last_sync_at(conn: &Connection) -> Result<Option<String>> {
    sync_storage::get_state(conn, LAST_SYNC_AT_KEY).context("failed to read last-sync timestamp")
}

fn set_last_sync_now(conn: &Connection) -> Result<()> {
    sync_storage::set_state(conn, LAST_SYNC_AT_KEY, &chrono::Utc::now().to_rfc3339())
        .context("failed to record last-sync timestamp")
}

fn load_ingested_batches(conn: &Connection) -> Result<Vec<String>> {
    match sync_storage::get_state(conn, INGESTED_BATCHES_KEY)
        .context("failed to read ingested-batch cursor")?
    {
        Some(json) => serde_json::from_str(&json).context("failed to parse ingested-batch cursor"),
        None => Ok(Vec::new()),
    }
}

fn save_ingested_batches(conn: &Connection, ids: &[String]) -> Result<()> {
    let json = serde_json::to_string(ids).context("failed to serialize ingested-batch cursor")?;
    sync_storage::set_state(conn, INGESTED_BATCHES_KEY, &json)
        .context("failed to persist ingested-batch cursor")
}
