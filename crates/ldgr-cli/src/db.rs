//! Database connection helper for CLI commands.
//!
//! Provides a shared pattern: check the vault is unlocked, then open the
//! encrypted `SQLite` (`SQLCipher`) database for operations. The working store is
//! encrypted at rest with a key derived from the session vault key (issue #295),
//! so opening it always requires an active session.

use std::path::Path;

use anyhow::{Context, Result, bail};
use ldgr_core::crypto::derive_db_key;
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::session;

/// Apply the `SQLCipher` key (derived from the session vault key) to a connection.
///
/// Uses the raw-key `PRAGMA key = "x'...'"` form so `SQLCipher` uses the derived
/// bytes directly. The formatted `PRAGMA` statement (which contains the key hex)
/// is zeroized after use.
pub fn apply_key(conn: &Connection, session_key: &[u8; 32]) -> Result<()> {
    let db_key = derive_db_key(session_key)
        .map_err(|e| anyhow::anyhow!("failed to derive database key: {e}"))?;
    let pragma = db_key.to_pragma_hex();
    let stmt = Zeroizing::new(format!(
        "PRAGMA cipher_memory_security = ON;\nPRAGMA key = \"{}\";",
        pragma.as_str()
    ));
    conn.execute_batch(&stmt)
        .context("failed to apply database encryption key")?;
    Ok(())
}

/// Open an encrypted database, applying the key and verifying it decrypts.
///
/// Works for both a fresh (empty) file — which becomes encrypted on first write
/// — and an existing `SQLCipher` database. Fails clearly if the key is wrong or
/// the file is an unmigrated plaintext store.
pub fn open_encrypted(path: &Path, session_key: &[u8; 32]) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("failed to open database at {}", path.display()))?;
    apply_key(&conn, session_key)?;
    // Force a page read so an incorrect key (or plaintext store) fails now with
    // a clear message rather than at the first query.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .context(
        "failed to open the encrypted working store — wrong key or an unmigrated plaintext database",
    )?;
    Ok(conn)
}

/// Open the encrypted `SQLite` database, requiring the vault to be unlocked.
///
/// Returns the database connection. Fails if no active session exists.
pub fn require_unlocked_db(vault_path: &Path) -> Result<Connection> {
    let (conn, _key) = require_unlocked_db_with_key(vault_path)?;
    Ok(conn)
}

/// Open the encrypted `SQLite` database **and** return the cached raw vault key.
///
/// Like [`require_unlocked_db`], but also yields the 32-byte session key needed
/// to drive the sync pipeline (`export`/`ingest`). Fails if the vault is locked.
pub fn require_unlocked_db_with_key(vault_path: &Path) -> Result<(Connection, [u8; 32])> {
    let vault_dir = session::resolve_vault_dir(Some(vault_path));

    let (key, _info) = session::load_session(&vault_dir)?
        .ok_or_else(|| anyhow::anyhow!("Vault is locked. Run `ldgr unlock` first."))?;

    let db_path = vault_dir.join("vault.db");
    if !db_path.exists() {
        bail!(
            "Database not found at {}.\nRun `ldgr init` to create a vault.",
            db_path.display()
        );
    }

    let conn = open_encrypted(&db_path, &key)?;
    Ok((conn, key))
}
