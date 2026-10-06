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
    TransportConfig, batch_path, batches_prefix, device_batches_prefix, parse_batch_path,
};

use super::BlobTransport;

/// `sync_state` key holding the JSON array of remote batch ids already ingested.
const INGESTED_BATCHES_KEY: &str = "cli_ingested_batches";
/// `sync_state` key holding the RFC3339 timestamp of the last successful sync.
const LAST_SYNC_AT_KEY: &str = "cli_last_sync_at";
/// Legacy file (pre-unification) that held a CLI-only device id.
const LEGACY_DEVICE_ID_FILE: &str = "device-id";

/// Sync configuration file, read only to recover a pre-ADR-011 vault id.
const SYNC_CONFIG_FILE: &str = "sync-config.json";

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

/// Resolve this vault's sync identifier, adopting a pre-ADR-011 one on upgrade.
///
/// The identifier is **persisted** in `sync_state` and never re-derived. Earlier
/// builds recomputed it on every call as a djb2 hash of the vault *directory
/// path*, which made it predictable, enumerable, and — because nearly every user
/// keeps the vault at the default path — identical across unrelated accounts, so
/// the first account to register it on a shared server locked everyone else out.
///
/// Resolution order, first match wins:
///
/// 1. the identifier already stored in `sync_state`;
/// 2. the one persisted in `sync-config.json` by a configured server transport;
/// 3. for an already-configured Dropbox/WebDAV vault, which stores no identifier
///    anywhere, the legacy path-derived value — recomputed **once** and then
///    frozen, so existing remote blobs under `vault_<hash>/` stay reachable;
/// 4. otherwise a fresh random identifier.
///
/// Steps 2 and 3 are the upgrade path: without them an existing user would get a
/// brand-new identifier and be silently orphaned from everything they had synced.
pub fn resolve_vault_id(conn: &Connection, vault_dir: &Path) -> Result<String> {
    if let Some(existing) = existing_vault_id(conn, vault_dir)? {
        return Ok(existing);
    }
    sync_storage::vault_id(conn).context("failed to resolve vault id")
}

/// The identifier this vault is already known by, without minting one.
///
/// Returns `None` only for a vault that has never synced, which is the single
/// case where a caller may choose a brand-new identifier (or adopt one from the
/// server). Steps 1-3 of [`resolve_vault_id`].
pub fn existing_vault_id(conn: &Connection, vault_dir: &Path) -> Result<Option<String>> {
    if let Some(id) = sync_storage::get_state(conn, sync_storage::VAULT_ID_KEY)
        .context("failed to read vault id state")?
    {
        return Ok(Some(id));
    }

    let Some(legacy) = legacy_vault_id(vault_dir) else {
        return Ok(None);
    };
    let adopted = sync_storage::adopt_vault_id(conn, &legacy)
        .context("failed to adopt the existing vault id")?;
    Ok(Some(adopted))
}

/// Record the identifier the server put in force.
///
/// The server's response is authoritative — it may hand back a different
/// identifier than the one requested when that one is already owned by another
/// account — so setup persists what it receives rather than what it asked for.
pub fn persist_vault_id(conn: &Connection, id: &str) -> Result<()> {
    sync_storage::set_vault_id(conn, id).context("failed to persist the vault id")
}

/// The identifier an already-configured vault used before ADR-011, or `None`
/// when sync was never set up (in which case a fresh random id is correct).
///
/// Migration-only. Nothing else may derive an identifier from the vault path.
fn legacy_vault_id(vault_dir: &Path) -> Option<String> {
    let config_path = vault_dir.join(SYNC_CONFIG_FILE);
    if !config_path.exists() {
        return None;
    }

    // A server transport persisted its identifier; prefer that exact value over
    // re-deriving, since the vault directory may have moved since setup.
    let configured = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|json| serde_json::from_str::<TransportConfig>(&json).ok());
    if let Some(TransportConfig::Server { vault_id, .. }) = configured
        && !vault_id.is_empty()
    {
        return Some(vault_id);
    }

    // Dropbox/WebDAV store no identifier, so reproduce the one their blobs are
    // already filed under.
    Some(path_derived_vault_id(vault_dir))
}

/// Reproduce the pre-ADR-011 path-derived identifier (djb2 over the vault
/// directory path).
///
/// Migration-only, and deliberately private: this is exactly the weak,
/// guessable, collision-prone derivation ADR-011 removes. It exists so an
/// upgrading Dropbox/WebDAV vault can adopt the identifier its existing remote
/// blobs are stored under, once, before freezing it in `sync_state`.
fn path_derived_vault_id(vault_dir: &Path) -> String {
    let mut hash: u64 = 5381;
    for &b in vault_dir.to_string_lossy().as_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u64::from(b));
    }
    format!("vault_{hash:016x}")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory SQLite");
        ldgr_core::storage::schema::initialize(&conn).expect("schema");
        conn
    }

    fn write_config(dir: &Path, config: &TransportConfig) {
        std::fs::write(
            dir.join(SYNC_CONFIG_FILE),
            serde_json::to_string_pretty(config).unwrap(),
        )
        .unwrap();
    }

    fn server_config(vault_id: &str) -> TransportConfig {
        TransportConfig::Server {
            base_url: "https://sync.example.com".into(),
            username: Some("user@example.com".into()),
            vault_id: vault_id.into(),
            device_id: "device-a".into(),
        }
    }

    // ── Branch 4: never-synced vault ────────────────────────────────────────

    #[test]
    fn unconfigured_vault_mints_a_random_id_once() {
        let dir = tempfile::tempdir().unwrap();
        let conn = vault_db();

        let first = resolve_vault_id(&conn, dir.path()).unwrap();
        assert!(
            ldgr_core::sync::is_random_vault_id(&first),
            "expected a random id, got {first}"
        );

        let second = resolve_vault_id(&conn, dir.path()).unwrap();
        assert_eq!(
            first, second,
            "the identifier must be persisted, not re-minted"
        );
    }

    #[test]
    fn two_vaults_at_the_same_path_get_different_ids() {
        // The whole point of ADR-011: the identifier no longer depends on where
        // the vault lives, so two accounts on the default path cannot collide.
        let dir = tempfile::tempdir().unwrap();
        let a = resolve_vault_id(&vault_db(), dir.path()).unwrap();
        let b = resolve_vault_id(&vault_db(), dir.path()).unwrap();
        assert_ne!(a, b);
    }

    // ── Branch 2: upgrading a configured server vault ───────────────────────

    #[test]
    fn configured_server_vault_keeps_its_identifier() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), &server_config("vault_0123456789abcdef"));
        let conn = vault_db();

        let resolved = resolve_vault_id(&conn, dir.path()).unwrap();
        assert_eq!(
            resolved, "vault_0123456789abcdef",
            "an upgrading client must keep the identifier its blobs are filed under"
        );
    }

    #[test]
    fn configured_server_identifier_survives_moving_the_vault_directory() {
        // The persisted identifier wins over anything derived from the path, so
        // renaming the vault directory no longer silently repoints sync.
        let original = tempfile::tempdir().unwrap();
        write_config(
            original.path(),
            &server_config("v1_abcdefabcdefabcdefabcdefabcdefab"),
        );
        let conn = vault_db();
        let before = resolve_vault_id(&conn, original.path()).unwrap();

        let moved = tempfile::tempdir().unwrap();
        write_config(
            moved.path(),
            &server_config("v1_abcdefabcdefabcdefabcdefabcdefab"),
        );
        let after = resolve_vault_id(&conn, moved.path()).unwrap();

        assert_eq!(before, after);
        assert_eq!(after, "v1_abcdefabcdefabcdefabcdefabcdefab");
    }

    // ── Branch 3: upgrading a configured Dropbox/WebDAV vault ───────────────

    #[test]
    fn configured_blob_provider_adopts_the_path_derived_identifier() {
        // Dropbox and WebDAV store no identifier anywhere, so an upgrading vault
        // must reproduce the one its remote blobs already live under — otherwise
        // everything previously synced becomes unreachable.
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            &TransportConfig::WebDav {
                base_url: "https://dav.example.com".into(),
                username: Some("user".into()),
            },
        );
        let conn = vault_db();

        let resolved = resolve_vault_id(&conn, dir.path()).unwrap();
        assert_eq!(resolved, path_derived_vault_id(dir.path()));
        assert!(ldgr_core::sync::is_legacy_vault_id(&resolved));
    }

    #[test]
    fn an_adopted_legacy_identifier_is_frozen_not_re_derived() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            &TransportConfig::Dropbox {
                app_key: "key".into(),
                account_hint: None,
            },
        );
        let conn = vault_db();
        let adopted = resolve_vault_id(&conn, dir.path()).unwrap();

        // Moving the vault afterwards must NOT change the identifier — the old
        // derivation would have silently orphaned every synced blob.
        let moved = tempfile::tempdir().unwrap();
        write_config(
            moved.path(),
            &TransportConfig::Dropbox {
                app_key: "key".into(),
                account_hint: None,
            },
        );
        assert_eq!(resolve_vault_id(&conn, moved.path()).unwrap(), adopted);
    }

    // ── Branch 1 / precedence ───────────────────────────────────────────────

    #[test]
    fn a_stored_identifier_wins_over_the_configuration() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), &server_config("vault_0123456789abcdef"));
        let conn = vault_db();

        persist_vault_id(&conn, "v1_11112222333344445555666677778888").unwrap();
        assert_eq!(
            resolve_vault_id(&conn, dir.path()).unwrap(),
            "v1_11112222333344445555666677778888"
        );
    }

    #[test]
    fn existing_vault_id_reports_none_only_for_a_never_synced_vault() {
        let dir = tempfile::tempdir().unwrap();
        let conn = vault_db();
        assert!(existing_vault_id(&conn, dir.path()).unwrap().is_none());

        write_config(dir.path(), &server_config("vault_0123456789abcdef"));
        assert_eq!(
            existing_vault_id(&conn, dir.path()).unwrap().as_deref(),
            Some("vault_0123456789abcdef")
        );
    }

    #[test]
    fn a_fresh_setup_mints_rather_than_deriving() {
        // `sync setup` freezes the identifier before writing the new config, so
        // a vault that has never synced mints a random one even for Dropbox and
        // WebDAV — which record no identifier of their own. Resolving against an
        // empty directory reproduces that pre-write state.
        let dir = tempfile::tempdir().unwrap();
        let conn = vault_db();
        let minted = resolve_vault_id(&conn, dir.path()).unwrap();
        assert!(ldgr_core::sync::is_random_vault_id(&minted), "{minted}");

        // Writing the config afterwards must not change it.
        write_config(
            dir.path(),
            &TransportConfig::Dropbox {
                app_key: "key".into(),
                account_hint: None,
            },
        );
        assert_eq!(resolve_vault_id(&conn, dir.path()).unwrap(), minted);
    }

    #[test]
    fn path_derivation_reproduces_the_pre_adr_011_identifier() {
        // Pins the exact legacy algorithm (djb2, seed 5381, multiplier 33) so a
        // future refactor cannot silently change which blobs a legacy vault
        // adopts. This value is the hash of the literal path below.
        assert_eq!(
            path_derived_vault_id(Path::new("/home/user/.ldgr")),
            "vault_9eeca790b42e1fb1"
        );
    }
}
