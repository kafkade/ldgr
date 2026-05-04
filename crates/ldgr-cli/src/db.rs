//! Database connection helper for CLI commands.
//!
//! Provides a shared pattern: check the vault is unlocked, then open
//! the `SQLite` database for operations.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::Connection;

use crate::session;

/// Open the `SQLite` database, requiring the vault to be unlocked.
///
/// Returns the database connection. Fails if no active session exists.
pub fn require_unlocked_db(vault_path: &Path) -> Result<Connection> {
    let vault_dir = session::resolve_vault_dir(Some(vault_path));

    if session::load_session(&vault_dir)?.is_none() {
        bail!("Vault is locked. Run `ldgr unlock` first.");
    }

    let db_path = vault_dir.join("vault.db");
    if !db_path.exists() {
        bail!(
            "Database not found at {}.\nRun `ldgr init` to create a vault.",
            db_path.display()
        );
    }

    Connection::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))
}
