//! `UniFFI` bindings for ldgr-core.
//!
//! Provides a thread-safe `LdgrVault` object that wraps the core library's
//! crypto, storage, and accounting modules for use from Swift/Kotlin via FFI.

// UniFFI scaffolding uses #[unsafe(no_mangle)] for FFI symbol exports.
#![allow(unsafe_code)]
// UniFFI requires owned String params at the FFI boundary.
#![allow(clippy::needless_pass_by_value)]

use std::path::PathBuf;
use std::sync::Mutex;

use ldgr_core::accounting::reports;
use ldgr_core::accounting::types as acct;
use ldgr_core::crypto::{
    self, Argon2Params, UnlockedVault, encode_recovery_key, open_vault, restore_vault_from_session,
    serialize_vault,
};
use ldgr_core::storage::accounts::{self, AccountType, ListOptions, NewAccount};
use ldgr_core::storage::error::StorageError;
use ldgr_core::storage::schema;
use ldgr_core::storage::sync as sync_storage;
use ldgr_core::storage::transactions::{self, NewPosting, NewTransaction, TransactionStatus};
use rusqlite::Connection;

uniffi::include_scaffolding!("ldgr");

mod sync;
pub use sync::*;

// ── Error Type ─────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum LdgrError {
    #[error("vault is locked")]
    VaultLocked,
    #[error("invalid password")]
    InvalidPassword,
    #[error("crypto error: {0}")]
    CryptoError(String),
    #[error("storage error: {0}")]
    StorageError(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("I/O error: {0}")]
    IoError(String),
}

impl From<crypto::CryptoError> for LdgrError {
    fn from(e: crypto::CryptoError) -> Self {
        match e {
            crypto::CryptoError::UnwrapFailed => Self::InvalidPassword,
            other => Self::CryptoError(other.to_string()),
        }
    }
}

impl From<StorageError> for LdgrError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::NotFound(msg) => Self::NotFound(msg),
            StorageError::Conflict(msg) => Self::Conflict(msg),
            StorageError::InvalidInput(msg) => Self::InvalidInput(msg),
            other => Self::StorageError(other.to_string()),
        }
    }
}

impl From<std::io::Error> for LdgrError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e.to_string())
    }
}

impl From<rusqlite::Error> for LdgrError {
    fn from(e: rusqlite::Error) -> Self {
        Self::StorageError(format!("database error: {e}"))
    }
}

impl From<ldgr_core::sync::pipeline::PipelineError> for LdgrError {
    fn from(e: ldgr_core::sync::pipeline::PipelineError) -> Self {
        use ldgr_core::sync::pipeline::PipelineError;
        match e {
            PipelineError::Storage(s) => s.into(),
            PipelineError::Crypto(c) => c.into(),
            PipelineError::Format(msg) => {
                Self::InvalidInput(format!("sync blob format error: {msg}"))
            }
            PipelineError::UnsupportedEntity(ent) => {
                Self::InvalidInput(format!("unsupported sync entity: {ent}"))
            }
        }
    }
}

// ── FFI Record Types ───────────────────────────────────────────────────────────

pub struct FfiAccount {
    pub id: String,
    pub name: String,
    pub account_type: String,
    pub commodity: Option<String>,
}

pub struct FfiNewPosting {
    pub account_id: String,
    pub amount: Option<String>,
    pub commodity: Option<String>,
}

pub struct FfiPosting {
    pub id: String,
    pub account_id: String,
    pub amount: Option<String>,
    pub commodity: Option<String>,
}

pub struct FfiTransaction {
    pub id: String,
    pub date: String,
    pub description: String,
    pub status: String,
    pub postings: Vec<FfiPosting>,
}

pub struct FfiBalanceEntry {
    pub account: String,
    pub amount: String,
    pub commodity: String,
}

// ── Sync FFI Types ─────────────────────────────────────────────────────────────

pub struct FfiSyncEvent {
    pub id: String,
    pub device_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub lamport_clock: u64,
    pub synced: bool,
}

pub struct FfiSyncConflict {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub local_payload: String,
    pub remote_payload: String,
    pub detected_at: String,
}

pub struct FfiSyncStatus {
    pub pending_event_count: u64,
    pub unresolved_conflict_count: u64,
    pub last_sync_at: Option<String>,
    pub device_id: String,
}

#[derive(Debug)]
pub struct FfiExportedBatch {
    pub batch_id: String,
    pub device_id: String,
    pub ciphertext: Vec<u8>,
    pub event_ids: Vec<String>,
}

#[derive(Debug)]
pub struct FfiIngestOutcome {
    pub applied: u32,
    pub conflicts: u32,
    pub skipped: u32,
}

// ── Vault State ────────────────────────────────────────────────────────────────

/// Internal vault state — either locked or holding an open connection.
#[allow(clippy::large_enum_variant)]
enum VaultState {
    Locked,
    Unlocked {
        #[allow(dead_code)]
        vault: UnlockedVault,
        conn: Connection,
    },
}

// ── LdgrVault Object ───────────────────────────────────────────────────────────

pub struct LdgrVault {
    vault_dir: PathBuf,
    vault_path: PathBuf,
    db_path: PathBuf,
    state: Mutex<VaultState>,
}

impl LdgrVault {
    /// Create a new vault handle pointing at the given directory.
    ///
    /// Does not open or create anything — call `create_vault` or `open` next.
    pub fn new(path: String) -> Result<Self, LdgrError> {
        let vault_dir = PathBuf::from(&path);
        let vault_path = vault_dir.join("vault.ldgr");
        let db_path = vault_dir.join("vault.db");

        Ok(Self {
            vault_dir,
            vault_path,
            db_path,
            state: Mutex::new(VaultState::Locked),
        })
    }

    /// Create a new vault with the given password and name.
    ///
    /// Returns the recovery key as a human-readable string.
    /// The caller MUST present this to the user — it cannot be retrieved later.
    pub fn create_vault(&self, password: String, name: String) -> Result<String, LdgrError> {
        // Ensure directory exists
        std::fs::create_dir_all(&self.vault_dir)?;

        if self.vault_path.exists() {
            return Err(LdgrError::Conflict(
                "vault already exists at this path".to_string(),
            ));
        }

        // Use mobile Argon2 params for iOS
        let (vault, recovery_key) =
            crypto::create_vault(password.as_bytes(), &name, &Argon2Params::mobile())?;

        // Serialize and write vault file atomically
        let vault_bytes = serialize_vault(&vault)?;
        atomic_write(&self.vault_path, &vault_bytes)?;

        // Create and initialize SQLite database
        let conn = Connection::open(&self.db_path)?;
        schema::initialize(&conn)?;

        let recovery_string = encode_recovery_key(&recovery_key);

        let mut state = self.state.lock().expect("mutex poisoned");
        *state = VaultState::Unlocked { vault, conn };

        Ok(recovery_string)
    }

    /// Unlock an existing vault with the given password.
    pub fn open(&self, password: String) -> Result<(), LdgrError> {
        if !self.vault_path.exists() {
            return Err(LdgrError::NotFound(format!(
                "vault file not found: {}",
                self.vault_path.display()
            )));
        }

        let data = std::fs::read(&self.vault_path)?;
        let vault = open_vault(&data, password.as_bytes())?;

        let conn = Connection::open(&self.db_path)?;
        // Ensure schema is initialized (idempotent)
        schema::initialize(&conn)?;

        let mut state = self.state.lock().expect("mutex poisoned");
        *state = VaultState::Unlocked { vault, conn };

        Ok(())
    }

    /// Lock the vault, dropping all in-memory keys and closing the database.
    pub fn close(&self) {
        let mut state = self.state.lock().expect("mutex poisoned");
        *state = VaultState::Locked;
    }

    /// Export the session key (vault key bytes) for secure caching.
    ///
    /// Returns 32 bytes. The caller MUST protect these bytes — they grant
    /// full read/write access to all vault data. On iOS, store in the
    /// Keychain with biometric access control.
    pub fn export_session_key(&self) -> Result<Vec<u8>, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        match &*state {
            VaultState::Locked => Err(LdgrError::VaultLocked),
            VaultState::Unlocked { vault, .. } => Ok(vault.export_session_key().to_vec()),
        }
    }

    /// Unlock the vault using a previously exported session key.
    ///
    /// Skips Argon2id password derivation — used for biometric unlock
    /// where the session key was stored in the OS keychain.
    pub fn open_with_session_key(&self, key: Vec<u8>) -> Result<(), LdgrError> {
        let key_bytes: [u8; 32] = key.try_into().map_err(|v: Vec<u8>| {
            LdgrError::InvalidInput(format!(
                "session key must be exactly 32 bytes, got {}",
                v.len()
            ))
        })?;

        if !self.vault_path.exists() {
            return Err(LdgrError::NotFound(format!(
                "vault file not found: {}",
                self.vault_path.display()
            )));
        }

        let data = std::fs::read(&self.vault_path)?;
        let vault = restore_vault_from_session(&data, &key_bytes)?;

        let conn = Connection::open(&self.db_path)?;
        schema::initialize(&conn)?;

        let mut state = self.state.lock().expect("mutex poisoned");
        *state = VaultState::Unlocked { vault, conn };

        Ok(())
    }

    /// Check whether the vault is currently unlocked.
    pub fn is_unlocked(&self) -> bool {
        let state = self.state.lock().expect("mutex poisoned");
        matches!(*state, VaultState::Unlocked { .. })
    }

    /// Get the vault name. Requires the vault to be unlocked.
    pub fn vault_name(&self) -> Result<String, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        match &*state {
            VaultState::Locked => Err(LdgrError::VaultLocked),
            VaultState::Unlocked { vault, .. } => Ok(vault.metadata().name.clone()),
        }
    }

    /// Create a new account. Returns the account ID.
    pub fn add_account(
        &self,
        name: String,
        account_type: String,
        commodity: Option<String>,
    ) -> Result<String, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        let acct_type = parse_account_type(&account_type)?;
        let input = NewAccount {
            name,
            account_type: acct_type,
            commodity,
            parent_id: None,
            note: None,
        };
        let ctx = next_sync_context(conn)?;
        let account = accounts::create_account_with_sync(conn, &input, &ctx)?;
        Ok(account.id)
    }

    /// List all accounts.
    pub fn list_accounts(&self) -> Result<Vec<FfiAccount>, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        let opts = ListOptions::default();
        let accts = accounts::list_accounts(conn, &opts)?;

        Ok(accts
            .into_iter()
            .map(|a| FfiAccount {
                id: a.id,
                name: a.name,
                account_type: format_account_type(a.account_type),
                commodity: a.commodity,
            })
            .collect())
    }

    /// Add a new transaction. Returns the transaction ID.
    ///
    /// Validates that all referenced accounts exist and amounts parse as decimals.
    pub fn add_transaction(
        &self,
        date: String,
        description: String,
        status: String,
        postings: Vec<FfiNewPosting>,
    ) -> Result<String, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        // Validate date format (YYYY-MM-DD)
        validate_date(&date)?;

        let tx_status = parse_transaction_status(&status)?;

        // Validate and convert postings
        if postings.is_empty() {
            return Err(LdgrError::InvalidInput(
                "transaction must have at least one posting".to_string(),
            ));
        }

        let mut new_postings = Vec::with_capacity(postings.len());
        for p in &postings {
            // Verify account exists
            let acct = accounts::get_account_by_name(conn, &p.account_id)?;
            if acct.is_none() {
                // Also try by ID
                let by_id = accounts::get_account(conn, &p.account_id, &ListOptions::default())?;
                if by_id.is_none() {
                    return Err(LdgrError::NotFound(format!(
                        "account not found: {}",
                        p.account_id
                    )));
                }
            }

            // Validate amount if present
            if let Some(ref amt) = p.amount
                && amt.parse::<rust_decimal::Decimal>().is_err()
            {
                return Err(LdgrError::InvalidInput(format!("invalid amount: {amt}")));
            }

            new_postings.push(NewPosting {
                account_id: p.account_id.clone(),
                amount_quantity: p.amount.clone(),
                amount_commodity: p.commodity.clone(),
                balance_assertion_quantity: None,
                balance_assertion_commodity: None,
            });
        }

        let input = NewTransaction {
            date,
            status: tx_status,
            code: None,
            description,
            comment: None,
            postings: new_postings,
        };

        let txn =
            transactions::create_transaction_with_sync(conn, &input, &next_sync_context(conn)?)?;
        Ok(txn.id)
    }

    /// List all transactions.
    pub fn list_transactions(&self) -> Result<Vec<FfiTransaction>, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        let opts = ListOptions::default();
        let txns = transactions::list_transactions(conn, &opts)?;

        Ok(txns.into_iter().map(to_ffi_transaction).collect())
    }

    /// Soft-delete a transaction by ID.
    pub fn delete_transaction(&self, id: String) -> Result<(), LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        transactions::soft_delete_transaction_with_sync(conn, &id, &next_sync_context(conn)?)?;
        Ok(())
    }

    /// Compute account balances, optionally filtered by account name and date range.
    pub fn balance(
        &self,
        account_filter: Option<String>,
        begin_date: Option<String>,
        end_date: Option<String>,
    ) -> Result<Vec<FfiBalanceEntry>, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        // Load transactions from storage
        let opts = ListOptions::default();
        let store_txns = transactions::list_transactions(conn, &opts)?;

        // Convert to accounting types for the report engine
        let acct_txns = to_accounting_txns(&store_txns);

        let report = reports::compute_balance(
            &acct_txns,
            account_filter.as_deref(),
            begin_date.as_deref(),
            end_date.as_deref(),
        );

        Ok(report
            .accounts
            .into_iter()
            .flat_map(|ab| {
                ab.balances
                    .into_iter()
                    .map(move |(commodity, amount)| FfiBalanceEntry {
                        account: ab.account.clone(),
                        amount: amount.to_string(),
                        commodity,
                    })
            })
            .collect())
    }

    // ── Sync Methods ───────────────────────────────────────────────────────────

    /// Get the current sync status (pending events, conflicts, last sync time).
    pub fn sync_status(&self) -> Result<FfiSyncStatus, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        let pending = u64::from(sync_storage::pending_event_count(conn)?);
        let conflicts = u64::from(sync_storage::unresolved_conflict_count(conn)?);
        let last_sync = sync_storage::get_state(conn, "last_sync_at")?;
        let device_id = sync_storage::device_id(conn)?;

        Ok(FfiSyncStatus {
            pending_event_count: pending,
            unresolved_conflict_count: conflicts,
            last_sync_at: last_sync,
            device_id,
        })
    }

    /// Get all pending (un-synced) sync events.
    pub fn pending_sync_events(&self) -> Result<Vec<FfiSyncEvent>, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        let events = sync_storage::pending_events(conn)?;
        Ok(events.into_iter().map(to_ffi_sync_event).collect())
    }

    /// Mark sync events as synced after successful push to remote.
    pub fn mark_events_synced(&self, event_ids: Vec<String>) -> Result<(), LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        sync_storage::mark_events_synced(conn, &event_ids)?;
        sync_storage::set_state(conn, "last_sync_at", &chrono::Utc::now().to_rfc3339())?;
        Ok(())
    }

    /// Get all unresolved sync conflicts requiring user review.
    pub fn list_conflicts(&self) -> Result<Vec<FfiSyncConflict>, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        let conflicts = sync_storage::list_unresolved_conflicts(conn)?;
        Ok(conflicts
            .into_iter()
            .map(|c| FfiSyncConflict {
                id: c.id,
                entity_type: c.entity_type,
                entity_id: c.entity_id,
                local_payload: String::from_utf8_lossy(&c.local_payload).into_owned(),
                remote_payload: String::from_utf8_lossy(&c.remote_payload).into_owned(),
                detected_at: c.detected_at,
            })
            .collect())
    }

    /// Resolve a sync conflict by choosing a resolution strategy.
    ///
    /// Resolution must be one of: `keep_local`, `keep_remote`.
    pub fn resolve_conflict(
        &self,
        conflict_id: String,
        resolution: String,
    ) -> Result<(), LdgrError> {
        if resolution != "keep_local" && resolution != "keep_remote" {
            return Err(LdgrError::InvalidInput(format!(
                "invalid resolution: {resolution} (expected: keep_local, keep_remote)"
            )));
        }

        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        sync_storage::resolve_conflict(conn, &conflict_id, &resolution)?;
        Ok(())
    }

    /// Store sync conflicts detected during merge for later user review.
    pub fn store_conflicts(&self, conflicts: Vec<FfiSyncConflict>) -> Result<(), LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let conn = require_conn(&state)?;

        let stored: Vec<sync_storage::StoredConflict> = conflicts
            .into_iter()
            .map(|c| sync_storage::StoredConflict {
                id: c.id,
                entity_type: c.entity_type,
                entity_id: c.entity_id,
                local_event_id: String::new(),
                remote_event_id: String::new(),
                local_payload: c.local_payload.into_bytes(),
                remote_payload: c.remote_payload.into_bytes(),
                detected_at: c.detected_at,
                resolved: false,
                resolution: None,
            })
            .collect();

        sync_storage::store_conflicts(conn, &stored)?;
        Ok(())
    }

    /// Compose all currently-pending sync events into one encrypted batch blob
    /// ready for upload. Returns `null` if there are no pending events.
    ///
    /// Does **not** mark events synced — the caller uploads the ciphertext and
    /// then calls [`Self::mark_events_synced`] with the returned `event_ids`.
    pub fn export_pending_batch(&self) -> Result<Option<FfiExportedBatch>, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let (vault, conn) = match &*state {
            VaultState::Locked => return Err(LdgrError::VaultLocked),
            VaultState::Unlocked { vault, conn } => (vault, conn),
        };

        let session_key = vault.export_session_key();
        let device_id = sync_storage::device_id(conn)?;
        let exported = ldgr_core::sync::pipeline::export_pending_batch_with_session_key(
            conn,
            &device_id,
            &session_key,
        )?;

        Ok(exported.map(|b| FfiExportedBatch {
            batch_id: b.batch_id,
            device_id: b.device_id,
            ciphertext: b.ciphertext,
            event_ids: b.event_ids,
        }))
    }

    /// Apply a downloaded encrypted batch blob against local state.
    ///
    /// Decrypts, three-way merges, applies cleanly-merged events to the
    /// canonical tables, and persists any conflicts for later user review
    /// (retrievable via [`Self::list_conflicts`]). Returns the applied /
    /// conflict / skipped counts. Idempotent: re-ingesting a seen blob is a
    /// no-op.
    pub fn ingest_batch(&self, ciphertext: Vec<u8>) -> Result<FfiIngestOutcome, LdgrError> {
        let state = self.state.lock().expect("mutex poisoned");
        let (vault, conn) = match &*state {
            VaultState::Locked => return Err(LdgrError::VaultLocked),
            VaultState::Unlocked { vault, conn } => (vault, conn),
        };

        let session_key = vault.export_session_key();
        let device_id = sync_storage::device_id(conn)?;
        let outcome = ldgr_core::sync::pipeline::ingest_batch_with_session_key(
            conn,
            &device_id,
            &session_key,
            &ciphertext,
        )?;

        Ok(FfiIngestOutcome {
            applied: outcome.applied,
            conflicts: outcome.conflicts,
            skipped: outcome.skipped,
        })
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn require_conn<'a>(
    state: &'a std::sync::MutexGuard<'_, VaultState>,
) -> Result<&'a Connection, LdgrError> {
    match &**state {
        VaultState::Locked => Err(LdgrError::VaultLocked),
        VaultState::Unlocked { conn, .. } => Ok(conn),
    }
}

/// Build a [`SyncContext`] for the next local mutation: the vault's device id
/// plus a freshly-ticked Lamport clock. Used by the `_with_sync` storage
/// variants so every write atomically records an outbox event for sync.
fn next_sync_context(conn: &Connection) -> Result<sync_storage::SyncContext, LdgrError> {
    let device_id = sync_storage::device_id(conn)?;
    let lamport_clock = sync_storage::tick_lamport(conn)?;
    Ok(sync_storage::SyncContext {
        device_id,
        lamport_clock,
    })
}

fn parse_account_type(s: &str) -> Result<AccountType, LdgrError> {
    match s.to_lowercase().as_str() {
        "asset" | "assets" => Ok(AccountType::Asset),
        "liability" | "liabilities" => Ok(AccountType::Liability),
        "income" | "revenue" => Ok(AccountType::Income),
        "expense" | "expenses" => Ok(AccountType::Expense),
        "equity" => Ok(AccountType::Equity),
        _ => Err(LdgrError::InvalidInput(format!(
            "invalid account type: {s} (expected: asset, liability, income, expense, equity)"
        ))),
    }
}

fn format_account_type(t: AccountType) -> String {
    match t {
        AccountType::Asset => "asset",
        AccountType::Liability => "liability",
        AccountType::Income => "income",
        AccountType::Expense => "expense",
        AccountType::Equity => "equity",
    }
    .to_string()
}

fn parse_transaction_status(s: &str) -> Result<TransactionStatus, LdgrError> {
    match s.to_lowercase().as_str() {
        "unmarked" | "" => Ok(TransactionStatus::Unmarked),
        "pending" | "!" => Ok(TransactionStatus::Pending),
        "cleared" | "*" => Ok(TransactionStatus::Cleared),
        _ => Err(LdgrError::InvalidInput(format!(
            "invalid transaction status: {s} (expected: unmarked, pending, cleared)"
        ))),
    }
}

fn format_transaction_status(s: TransactionStatus) -> String {
    match s {
        TransactionStatus::Unmarked => "unmarked",
        TransactionStatus::Pending => "pending",
        TransactionStatus::Cleared => "cleared",
    }
    .to_string()
}

fn validate_date(date: &str) -> Result<(), LdgrError> {
    // Accept YYYY-MM-DD format
    if date.len() != 10 {
        return Err(LdgrError::InvalidInput(format!(
            "invalid date format: {date} (expected YYYY-MM-DD)"
        )));
    }
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3
        || parts[0].len() != 4
        || parts[1].len() != 2
        || parts[2].len() != 2
        || parts[0].parse::<u16>().is_err()
        || parts[1].parse::<u8>().is_err()
        || parts[2].parse::<u8>().is_err()
    {
        return Err(LdgrError::InvalidInput(format!(
            "invalid date format: {date} (expected YYYY-MM-DD)"
        )));
    }
    Ok(())
}

fn to_ffi_transaction(txn: transactions::Transaction) -> FfiTransaction {
    FfiTransaction {
        id: txn.id,
        date: txn.date,
        description: txn.description,
        status: format_transaction_status(txn.status),
        postings: txn
            .postings
            .into_iter()
            .map(|p| FfiPosting {
                id: p.id,
                account_id: p.account_id,
                amount: p.amount_quantity,
                commodity: p.amount_commodity,
            })
            .collect(),
    }
}

/// Write data atomically via temp file + rename.
fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<(), LdgrError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ── Storage → Accounting Type Conversion ───────────────────────────────────────

fn to_accounting_txns(txns: &[transactions::Transaction]) -> Vec<acct::Transaction> {
    txns.iter().map(to_accounting_txn).collect()
}

fn to_ffi_sync_event(e: sync_storage::StoredSyncEvent) -> FfiSyncEvent {
    FfiSyncEvent {
        id: e.id,
        device_id: e.device_id,
        entity_type: e.entity_type,
        entity_id: e.entity_id,
        operation: e.operation,
        lamport_clock: e.lamport_clock,
        synced: e.synced,
    }
}

fn to_accounting_txn(txn: &transactions::Transaction) -> acct::Transaction {
    acct::Transaction {
        date: txn.date.clone(),
        status: match txn.status {
            TransactionStatus::Unmarked => acct::Status::Unmarked,
            TransactionStatus::Pending => acct::Status::Pending,
            TransactionStatus::Cleared => acct::Status::Cleared,
        },
        code: txn.code.clone(),
        description: txn.description.clone(),
        comment: txn.comment.clone(),
        tags: std::collections::HashMap::new(),
        source_line: 0,
        postings: txn.postings.iter().map(to_accounting_posting).collect(),
    }
}

fn to_accounting_posting(p: &transactions::Posting) -> acct::Posting {
    let amount = match (&p.amount_quantity, &p.amount_commodity) {
        (Some(qty), commodity) => qty.parse().ok().map(|q| acct::Amount {
            quantity: q,
            commodity: commodity.as_deref().unwrap_or("").to_string(),
        }),
        _ => None,
    };

    acct::Posting {
        account: p.account_id.clone(),
        amount,
        balance_assertion: None,
        status: acct::Status::Unmarked,
        comment: None,
        tags: std::collections::HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_account_types() {
        assert!(parse_account_type("asset").is_ok());
        assert!(parse_account_type("Asset").is_ok());
        assert!(parse_account_type("ASSETS").is_ok());
        assert!(parse_account_type("liability").is_ok());
        assert!(parse_account_type("income").is_ok());
        assert!(parse_account_type("expense").is_ok());
        assert!(parse_account_type("equity").is_ok());
        assert!(parse_account_type("invalid").is_err());
    }

    #[test]
    fn parse_transaction_statuses() {
        assert!(parse_transaction_status("unmarked").is_ok());
        assert!(parse_transaction_status("pending").is_ok());
        assert!(parse_transaction_status("cleared").is_ok());
        assert!(parse_transaction_status("*").is_ok());
        assert!(parse_transaction_status("!").is_ok());
        assert!(parse_transaction_status("").is_ok());
        assert!(parse_transaction_status("invalid").is_err());
    }

    #[test]
    fn validate_dates() {
        assert!(validate_date("2024-01-15").is_ok());
        assert!(validate_date("2024-12-31").is_ok());
        assert!(validate_date("not-a-date").is_err());
        assert!(validate_date("2024/01/15").is_err());
        assert!(validate_date("24-01-15").is_err());
    }

    #[test]
    fn smoke_test_vault_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let vault = LdgrVault::new(path).unwrap();
        assert!(!vault.is_unlocked());

        // Create vault
        let recovery_key = vault
            .create_vault("test-pw".to_string(), "Test Vault".to_string())
            .unwrap();
        assert!(!recovery_key.is_empty());
        assert!(vault.is_unlocked());
        assert_eq!(vault.vault_name().unwrap(), "Test Vault");

        // Add account
        let acct_id = vault
            .add_account(
                "Assets:Checking".to_string(),
                "asset".to_string(),
                Some("USD".to_string()),
            )
            .unwrap();
        assert!(!acct_id.is_empty());

        let acct_id2 = vault
            .add_account(
                "Expenses:Food".to_string(),
                "expense".to_string(),
                Some("USD".to_string()),
            )
            .unwrap();

        // List accounts
        let accts = vault.list_accounts().unwrap();
        assert_eq!(accts.len(), 2);

        // Add transaction
        let tx_id = vault
            .add_transaction(
                "2024-06-15".to_string(),
                "Grocery store".to_string(),
                "cleared".to_string(),
                vec![
                    FfiNewPosting {
                        account_id: acct_id.clone(),
                        amount: Some("-50.00".to_string()),
                        commodity: Some("USD".to_string()),
                    },
                    FfiNewPosting {
                        account_id: acct_id2.clone(),
                        amount: Some("50.00".to_string()),
                        commodity: Some("USD".to_string()),
                    },
                ],
            )
            .unwrap();
        assert!(!tx_id.is_empty());

        // List transactions
        let txns = vault.list_transactions().unwrap();
        assert_eq!(txns.len(), 1);
        assert_eq!(txns[0].description, "Grocery store");

        // Query balance
        let bal = vault.balance(None, None, None).unwrap();
        assert!(!bal.is_empty());

        // Close and reopen
        vault.close();
        assert!(!vault.is_unlocked());
        assert!(vault.vault_name().is_err());

        vault.open("test-pw".to_string()).unwrap();
        assert!(vault.is_unlocked());
        assert_eq!(vault.vault_name().unwrap(), "Test Vault");

        // Data persisted
        let txns = vault.list_transactions().unwrap();
        assert_eq!(txns.len(), 1);
    }

    #[test]
    fn wrong_password_returns_invalid_password() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let vault = LdgrVault::new(path.clone()).unwrap();
        vault
            .create_vault("correct".to_string(), "Test".to_string())
            .unwrap();
        vault.close();

        let vault2 = LdgrVault::new(path).unwrap();
        let err = vault2.open("wrong".to_string()).unwrap_err();
        assert!(matches!(err, LdgrError::InvalidPassword));
    }

    #[test]
    fn operations_on_locked_vault_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let vault = LdgrVault::new(path).unwrap();
        assert!(vault.list_accounts().is_err());
        assert!(vault.vault_name().is_err());
    }

    #[test]
    fn export_session_key_requires_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let vault = LdgrVault::new(path).unwrap();
        let err = vault.export_session_key().unwrap_err();
        assert!(matches!(err, LdgrError::VaultLocked));
    }

    #[test]
    fn session_key_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let vault = LdgrVault::new(path.clone()).unwrap();
        vault
            .create_vault("test-pw".to_string(), "Session Test".to_string())
            .unwrap();

        // Export session key while unlocked
        let key = vault.export_session_key().unwrap();
        assert_eq!(key.len(), 32);

        // Add an account so we can verify data survives
        vault
            .add_account(
                "Assets:Cash".to_string(),
                "asset".to_string(),
                Some("USD".to_string()),
            )
            .unwrap();

        // Close and reopen with session key
        vault.close();
        assert!(!vault.is_unlocked());

        vault.open_with_session_key(key).unwrap();
        assert!(vault.is_unlocked());
        assert_eq!(vault.vault_name().unwrap(), "Session Test");

        // Data persisted
        let accts = vault.list_accounts().unwrap();
        assert_eq!(accts.len(), 1);
        assert_eq!(accts[0].name, "Assets:Cash");
    }

    #[test]
    fn session_key_wrong_length_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let vault = LdgrVault::new(path).unwrap();
        vault
            .create_vault("pw".to_string(), "Test".to_string())
            .unwrap();
        vault.close();

        let err = vault.open_with_session_key(vec![0u8; 16]).unwrap_err();
        assert!(matches!(err, LdgrError::InvalidInput(_)));
    }

    #[test]
    fn session_key_wrong_key_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let vault = LdgrVault::new(path).unwrap();
        vault
            .create_vault("pw".to_string(), "Test".to_string())
            .unwrap();
        vault.close();

        let err = vault.open_with_session_key(vec![0xAB; 32]).unwrap_err();
        // Wrong key produces either CryptoError (decryption failed) or
        // InvalidPassword (unwrap failed) depending on how the core surfaces it.
        assert!(
            matches!(err, LdgrError::CryptoError(_) | LdgrError::InvalidPassword),
            "expected CryptoError or InvalidPassword, got: {err:?}"
        );
    }

    /// Helper: build a vault, add two accounts + one transaction, and return
    /// the handle plus the directory (kept alive by the caller).
    fn vault_with_one_txn(dir: &std::path::Path) -> LdgrVault {
        let path = dir.to_string_lossy().to_string();
        let vault = LdgrVault::new(path).unwrap();
        vault
            .create_vault("test-pw".to_string(), "Device A".to_string())
            .unwrap();

        let checking = vault
            .add_account(
                "Assets:Checking".to_string(),
                "asset".to_string(),
                Some("USD".to_string()),
            )
            .unwrap();
        let food = vault
            .add_account(
                "Expenses:Food".to_string(),
                "expense".to_string(),
                Some("USD".to_string()),
            )
            .unwrap();
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
            .unwrap();
        vault
    }

    #[test]
    fn export_batch_then_ingest_into_second_device() {
        // Device A: a fully-populated vault with pending sync events.
        let dir_a = tempfile::tempdir().unwrap();
        let vault_a = vault_with_one_txn(dir_a.path());

        // Device B: a *second* device of the SAME vault. It shares A's vault
        // key (same `vault.ldgr`) but has its own fresh, empty database — so it
        // gets a distinct device id and starts with no data, exactly like a
        // newly-enrolled device that has not yet synced.
        let session_key = vault_a.export_session_key().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        std::fs::copy(
            dir_a.path().join("vault.ldgr"),
            dir_b.path().join("vault.ldgr"),
        )
        .unwrap();
        let vault_b = LdgrVault::new(dir_b.path().to_string_lossy().to_string()).unwrap();
        vault_b.open_with_session_key(session_key).unwrap();

        // Sanity: B starts empty and the two devices differ.
        assert!(vault_b.list_transactions().unwrap().is_empty());
        assert!(vault_b.list_accounts().unwrap().is_empty());
        assert_ne!(
            vault_a.sync_status().unwrap().device_id,
            vault_b.sync_status().unwrap().device_id,
            "the two devices must have distinct device ids"
        );

        // A composes its pending events into an encrypted blob.
        let batch = vault_a
            .export_pending_batch()
            .unwrap()
            .expect("device A has pending events to export");
        assert!(!batch.ciphertext.is_empty());
        assert!(!batch.event_ids.is_empty());
        assert_eq!(batch.device_id, vault_a.sync_status().unwrap().device_id);

        // B ingests the blob and applies the events.
        let outcome = vault_b.ingest_batch(batch.ciphertext.clone()).unwrap();
        assert!(outcome.applied > 0, "expected applied > 0, got {outcome:?}");
        assert_eq!(outcome.conflicts, 0);

        // The transaction and accounts now exist on device B.
        let txns = vault_b.list_transactions().unwrap();
        assert_eq!(txns.len(), 1, "transaction should be present on device B");
        assert_eq!(txns[0].description, "Grocery store");
        let accts = vault_b.list_accounts().unwrap();
        assert_eq!(
            accts.len(),
            2,
            "both accounts should be present on device B"
        );

        // Re-ingesting the same blob is a no-op (idempotent).
        let again = vault_b.ingest_batch(batch.ciphertext).unwrap();
        assert_eq!(again.applied, 0);
        assert_eq!(again.conflicts, 0);
        assert_eq!(vault_b.list_transactions().unwrap().len(), 1);
    }

    #[test]
    fn export_pending_batch_empty_when_no_pending_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let vault = LdgrVault::new(path).unwrap();
        vault
            .create_vault("pw".to_string(), "Empty".to_string())
            .unwrap();

        // Mark everything synced so the outbox is empty.
        let pending = vault.pending_sync_events().unwrap();
        let ids: Vec<String> = pending.into_iter().map(|e| e.id).collect();
        if !ids.is_empty() {
            vault.mark_events_synced(ids).unwrap();
        }

        assert!(vault.export_pending_batch().unwrap().is_none());
    }

    #[test]
    fn export_and_ingest_require_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let vault = LdgrVault::new(path).unwrap();

        assert!(matches!(
            vault.export_pending_batch().unwrap_err(),
            LdgrError::VaultLocked
        ));
        assert!(matches!(
            vault.ingest_batch(vec![1, 2, 3]).unwrap_err(),
            LdgrError::VaultLocked
        ));
    }
}
