//! Migration of a legacy plaintext `vault.db` to the encrypted (`SQLCipher`)
//! working store (issue #295).
//!
//! Prior versions created an unencrypted `SQLite` database. The `ldgr migrate`
//! command detects such a file and re-materialises it as a `SQLCipher`-encrypted
//! database, keyed by the session vault key. `ldgr unlock` only *detects* a
//! plaintext store and instructs the user to run `ldgr migrate` — it never
//! migrates silently, because a large ledger's migration should be explicit. The
//! original is only removed once the encrypted copy is verified to preserve the
//! schema version and every table's row count; the plaintext file is retained as
//! a `.plaintext.bak` backup for backout.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ldgr_core::crypto::derive_db_key;
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::db;

/// The 16-byte header every unencrypted `SQLite` database starts with.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Returns `true` if the file is an unencrypted `SQLite` database (i.e. it still
/// needs migrating). A `SQLCipher`-encrypted file has an encrypted header and will
/// not match this magic.
pub fn is_plaintext_sqlite(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open {} for inspection", path.display()))?;
    let mut header = [0u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(&header == SQLITE_MAGIC),
        // Too small to be a plaintext database (e.g. empty/encrypted-only).
        Err(_) => Ok(false),
    }
}

/// Migrate the store at `db_path` to encrypted form if it is currently
/// plaintext. Returns `true` if a migration was performed.
pub fn migrate_if_plaintext(db_path: &Path, session_key: &[u8; 32]) -> Result<bool> {
    if !is_plaintext_sqlite(db_path)? {
        return Ok(false);
    }
    migrate_plaintext_to_encrypted(db_path, session_key)?;
    Ok(true)
}

fn migrate_plaintext_to_encrypted(db_path: &Path, session_key: &[u8; 32]) -> Result<()> {
    let dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_enc: PathBuf = dir.join("vault.db.migrating");
    let backup: PathBuf = dir.join("vault.db.plaintext.bak");

    if tmp_enc.exists() {
        fs::remove_file(&tmp_enc)
            .with_context(|| format!("failed to clear stale {}", tmp_enc.display()))?;
    }

    // Snapshot the plaintext source for later verification.
    let src = Connection::open(db_path)
        .with_context(|| format!("failed to open plaintext database at {}", db_path.display()))?;
    let src_version = ldgr_core::storage::schema::current_version(&src)
        .context("failed to read schema version from plaintext database")?;
    let src_counts = table_counts(&src)?;

    // Export the plaintext main database into a freshly keyed SQLCipher file.
    let db_key = derive_db_key(session_key)
        .map_err(|e| anyhow::anyhow!("failed to derive database key: {e}"))?;
    let pragma = db_key.to_pragma_hex();
    let attach = Zeroizing::new(format!(
        "ATTACH DATABASE '{}' AS encrypted KEY \"{}\";",
        sql_escape_single_quotes(&tmp_enc.to_string_lossy()),
        pragma.as_str()
    ));
    src.execute_batch(&attach)
        .context("failed to attach encrypted target for migration")?;
    src.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))
        .context("failed to export data into encrypted database")?;
    src.execute_batch("DETACH DATABASE encrypted;")
        .context("failed to detach encrypted database")?;
    drop(src);

    // Verify the encrypted copy is complete before touching the original.
    let verify = || -> Result<()> {
        let enc = db::open_encrypted(&tmp_enc, session_key)?;
        let enc_version = ldgr_core::storage::schema::current_version(&enc)
            .context("failed to read schema version from encrypted database")?;
        let enc_counts = table_counts(&enc)?;
        if enc_version != src_version || enc_counts != src_counts {
            bail!(
                "migration verification failed (schema {src_version}->{enc_version}); \
                 original database left untouched"
            );
        }
        Ok(())
    };
    if let Err(e) = verify() {
        let _ = fs::remove_file(&tmp_enc);
        return Err(e);
    }

    // Atomic swap: keep the plaintext original as a backup, move encrypted in.
    fs::rename(db_path, &backup).with_context(|| {
        format!(
            "failed to back up plaintext database to {}",
            backup.display()
        )
    })?;
    if let Err(e) = fs::rename(&tmp_enc, db_path) {
        // Attempt to restore the original on failure.
        let _ = fs::rename(&backup, db_path);
        return Err(e).with_context(|| {
            format!(
                "failed to move encrypted database into place at {}",
                db_path.display()
            )
        });
    }

    Ok(())
}

/// Row counts for every user table, used to verify a migration preserved data.
fn table_counts(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master \
             WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .context("failed to enumerate tables")?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<_, _>>()
        .context("failed to read table names")?;

    let mut counts = BTreeMap::new();
    for name in names {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| {
                r.get(0)
            })
            .with_context(|| format!("failed to count rows in {name}"))?;
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
    use rusqlite::Connection;

    const KEY: [u8; 32] = [7u8; 32];
    const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

    /// Returns `true` if `haystack` contains the byte sequence `needle`.
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
        let conn = db::open_encrypted(&enc, &KEY).unwrap();
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
        assert_eq!(&before[..16], SQLITE_HEADER);

        let migrated = migrate_if_plaintext(&db_path, &KEY).unwrap();
        assert!(migrated, "a plaintext store must be migrated");

        // Running again is a no-op (already encrypted).
        assert!(!migrate_if_plaintext(&db_path, &KEY).unwrap());

        // The on-disk file is no longer a plaintext SQLite database.
        let after = std::fs::read(&db_path).unwrap();
        assert_ne!(
            &after[..16],
            SQLITE_HEADER,
            "store must be encrypted at rest"
        );
        assert!(!is_plaintext_sqlite(&db_path).unwrap());

        // The plaintext backup is retained for backout.
        assert!(dir.path().join("vault.db.plaintext.bak").exists());
        // The temporary migration file is cleaned up.
        assert!(!dir.path().join("vault.db.migrating").exists());

        // Data is preserved and readable with the correct key.
        let conn = db::open_encrypted(&db_path, &KEY).unwrap();
        let probe: i64 = conn
            .query_row("SELECT count(*) FROM t_probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(probe, 3);
    }

    #[test]
    fn encrypted_store_is_unreadable_without_key() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vault.db");
        seed_plaintext_db(&db_path);
        assert!(migrate_if_plaintext(&db_path, &KEY).unwrap());

        // The at-rest guarantee, verified deterministically from the on-disk
        // bytes (cheap and identical on every platform): the file is no longer a
        // plaintext SQLite database and none of the schema's table names or the
        // seeded probe data survive in the clear — the payload is opaque
        // ciphertext without the key.
        let bytes = std::fs::read(&db_path).unwrap();
        assert_ne!(
            &bytes[..16],
            SQLITE_HEADER,
            "store must be encrypted at rest"
        );
        assert!(
            !byte_contains(&bytes, b"t_probe"),
            "table names must not be readable in the clear"
        );
        assert!(
            !byte_contains(&bytes, b"sqlite_master"),
            "schema must not be readable in the clear"
        );

        // A keyless open cannot read the schema either. With no key set,
        // SQLCipher treats the file as plain SQLite, sees a non-matching header,
        // and fails fast without attempting any decryption — so this stays cheap
        // and portable (unlike a wrong-key open, which would run the full native
        // AES/HMAC failure path and is SQLCipher's behavior to test, not ours).
        let conn = Connection::open(&db_path).unwrap();
        let keyless = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        });
        assert!(
            keyless.is_err(),
            "keyless read must fail on encrypted store"
        );

        // The data is only recoverable with the correct key.
        let conn = db::open_encrypted(&db_path, &KEY).unwrap();
        let probe: i64 = conn
            .query_row("SELECT count(*) FROM t_probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(probe, 3);
    }
}
