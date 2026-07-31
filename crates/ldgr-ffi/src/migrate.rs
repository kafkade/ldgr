//! Migration of a legacy plaintext `vault.db` to the encrypted (`SQLCipher`)
//! working store on the iOS/FFI path (issue #315).
//!
//! Prior versions of the FFI opened the working store with plain
//! `Connection::open`, writing an unencrypted `SQLite` database to the device
//! filesystem. This module detects such a file and re-materialises it as a
//! `SQLCipher`-encrypted database keyed by the session vault key, mirroring the
//! CLI's `ldgr migrate` semantics (`crates/ldgr-cli/src/migrate.rs`): the
//! caller triggers migration **explicitly** (see [`crate::LdgrVault::migrate`]),
//! it is never performed silently on unlock. The original is only removed once
//! the encrypted copy is verified to preserve the schema version and every
//! table's row count; the plaintext file is retained as a `.plaintext.bak`
//! backup for backout.
//!
//! The file I/O deliberately lives here in the FFI crate (not `ldgr-core`, which
//! is I/O-free) exactly as the equivalent logic lives in the CLI binary crate.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use ldgr_core::crypto::derive_db_key;
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::{LdgrError, open_encrypted_db};

/// The 16-byte header every unencrypted `SQLite` database starts with.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

fn storage_err(msg: impl Into<String>) -> LdgrError {
    LdgrError::StorageError(msg.into())
}

/// Returns `true` if the file is an unencrypted `SQLite` database (i.e. it still
/// needs migrating). A `SQLCipher`-encrypted file has an encrypted header and
/// will not match this magic.
pub fn is_plaintext_sqlite(path: &Path) -> Result<bool, LdgrError> {
    if !path.exists() {
        return Ok(false);
    }
    let mut file = fs::File::open(path)?;
    let mut header = [0u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(&header == SQLITE_MAGIC),
        // Too small to be a plaintext database (e.g. empty/encrypted-only).
        Err(_) => Ok(false),
    }
}

/// Migrate the store at `db_path` to encrypted form if it is currently
/// plaintext. Returns `true` if a migration was performed.
pub fn migrate_if_plaintext(db_path: &Path, session_key: &[u8; 32]) -> Result<bool, LdgrError> {
    if !is_plaintext_sqlite(db_path)? {
        return Ok(false);
    }
    migrate_plaintext_to_encrypted(db_path, session_key)?;
    Ok(true)
}

fn migrate_plaintext_to_encrypted(db_path: &Path, session_key: &[u8; 32]) -> Result<(), LdgrError> {
    let dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_enc: PathBuf = dir.join("vault.db.migrating");
    let backup: PathBuf = dir.join("vault.db.plaintext.bak");

    if tmp_enc.exists() {
        fs::remove_file(&tmp_enc)?;
    }

    // Snapshot the plaintext source for later verification.
    let src = Connection::open(db_path)?;
    let src_version = ldgr_core::storage::schema::current_version(&src)?;
    let src_counts = table_counts(&src)?;

    // Export the plaintext main database into a freshly keyed SQLCipher file.
    let db_key = derive_db_key(session_key)?;
    let pragma = db_key.to_pragma_hex();
    let attach = Zeroizing::new(format!(
        "ATTACH DATABASE '{}' AS encrypted KEY \"{}\";",
        sql_escape_single_quotes(&tmp_enc.to_string_lossy()),
        pragma.as_str()
    ));
    src.execute_batch(&attach)?;
    src.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))?;
    src.execute_batch("DETACH DATABASE encrypted;")?;
    drop(src);

    // Verify the encrypted copy is complete before touching the original.
    let verify = || -> Result<(), LdgrError> {
        let enc = open_encrypted_db(&tmp_enc, session_key)?;
        let enc_version = ldgr_core::storage::schema::current_version(&enc)?;
        let enc_counts = table_counts(&enc)?;
        if enc_version != src_version || enc_counts != src_counts {
            return Err(storage_err(format!(
                "migration verification failed (schema {src_version}->{enc_version}); \
                 original database left untouched"
            )));
        }
        Ok(())
    };
    if let Err(e) = verify() {
        let _ = fs::remove_file(&tmp_enc);
        return Err(e);
    }

    // Atomic swap: keep the plaintext original as a backup, move encrypted in.
    fs::rename(db_path, &backup)?;
    if let Err(e) = fs::rename(&tmp_enc, db_path) {
        // Attempt to restore the original on failure.
        let _ = fs::rename(&backup, db_path);
        return Err(e.into());
    }

    Ok(())
}

/// Row counts for every user table, used to verify a migration preserved data.
fn table_counts(conn: &Connection) -> Result<BTreeMap<String, i64>, LdgrError> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master \
         WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<_, _>>()?;

    let mut counts = BTreeMap::new();
    for name in names {
        let count: i64 = conn.query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| {
            r.get(0)
        })?;
        counts.insert(name, count);
    }
    Ok(counts)
}

fn sql_escape_single_quotes(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    fn byte_contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// Create a plaintext `SQLite` database at `path` with the real schema plus a
    /// small custom table carrying known row counts.
    fn seed_plaintext_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        ldgr_core::storage::schema::initialize(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE t_probe (id INTEGER); INSERT INTO t_probe VALUES (1),(2),(3);",
        )
        .unwrap();
    }

    #[test]
    fn detects_plaintext_and_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.db");
        seed_plaintext_db(&plain);
        assert!(is_plaintext_sqlite(&plain).unwrap());

        // A fresh encrypted database must NOT be detected as plaintext.
        let enc = dir.path().join("enc.db");
        let conn = open_encrypted_db(&enc, &KEY).unwrap();
        ldgr_core::storage::schema::initialize(&conn).unwrap();
        drop(conn);
        assert!(!is_plaintext_sqlite(&enc).unwrap());

        // A non-existent file is not "plaintext to migrate".
        assert!(!is_plaintext_sqlite(&dir.path().join("nope.db")).unwrap());
    }

    #[test]
    fn migration_round_trip_preserves_data_and_encrypts() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vault.db");
        seed_plaintext_db(&db_path);

        // Sanity: on-disk header is the plaintext SQLite magic before migration.
        let before = std::fs::read(&db_path).unwrap();
        assert_eq!(&before[..16], SQLITE_MAGIC);

        assert!(migrate_if_plaintext(&db_path, &KEY).unwrap());
        // Running again is a no-op (already encrypted).
        assert!(!migrate_if_plaintext(&db_path, &KEY).unwrap());

        // The on-disk file is no longer a plaintext SQLite database, and neither
        // the schema table names nor the seeded probe data survive in the clear.
        let after = std::fs::read(&db_path).unwrap();
        assert_ne!(
            &after[..16],
            SQLITE_MAGIC,
            "store must be encrypted at rest"
        );
        assert!(!is_plaintext_sqlite(&db_path).unwrap());
        assert!(!byte_contains(&after, b"t_probe"));
        assert!(!byte_contains(&after, b"sqlite_master"));

        // The plaintext backup is retained; the temp migration file is cleaned up.
        assert!(dir.path().join("vault.db.plaintext.bak").exists());
        assert!(!dir.path().join("vault.db.migrating").exists());

        // Data is preserved and readable with the correct key.
        let conn = open_encrypted_db(&db_path, &KEY).unwrap();
        let probe: i64 = conn
            .query_row("SELECT count(*) FROM t_probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(probe, 3);
    }
}
