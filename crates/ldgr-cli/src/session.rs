//! Session management for vault unlock persistence.
//!
//! When a vault is unlocked, the 32-byte vault key is cached so subsequent CLI
//! commands can access the vault without re-entering the password. The session
//! expires after a configurable timeout.
//!
//! **Security (issue #295)**: the key material is stored in the operating
//! system keystore (macOS Keychain / Windows Credential Manager / Linux kernel
//! keyutils) — **never** written to disk in plaintext. Only non-secret session
//! metadata (vault path, timestamps) is persisted to `session.json`, so a
//! filesystem reader cannot recover the key needed to decrypt the working store.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Default session timeout in minutes.
pub const DEFAULT_TIMEOUT_MINUTES: i64 = 15;

/// Session metadata file name within the vault directory. Contains **no**
/// key material — only the vault path and expiry timestamps.
const SESSION_FILE: &str = "session.json";

/// Service name used for the OS keystore entry holding the session key.
const KEYRING_SERVICE: &str = "ldgr-session";

/// Non-secret session metadata persisted to disk.
#[derive(Serialize, Deserialize)]
struct SessionData {
    vault_path: String,
    created_at: String,
    expires_at: String,
}

/// Build the OS-keystore entry for a given vault directory.
///
/// The account is the vault directory path, keeping sessions for distinct
/// vaults isolated from one another.
fn keyring_entry(vault_dir: &Path) -> Result<keyring::Entry> {
    let account = vault_dir.to_string_lossy();
    keyring::Entry::new(KEYRING_SERVICE, &account)
        .context("failed to access OS keystore for session")
}

/// Create a new session after a successful vault unlock.
///
/// Stores the vault key in the OS keystore and writes non-secret metadata to
/// `session.json` (owner-only permissions on Unix).
pub fn create_session(
    vault_dir: &Path,
    vault_path: &Path,
    vault_key: &[u8; 32],
    timeout_minutes: i64,
) -> Result<()> {
    let now = Utc::now();
    let expires = now + Duration::minutes(timeout_minutes);

    // Store the key in the OS keystore (never on disk).
    let hex = Zeroizing::new(hex_encode(vault_key));
    keyring_entry(vault_dir)?
        .set_password(&hex)
        .context("failed to store session key in OS keystore")?;

    let data = SessionData {
        vault_path: vault_path.to_string_lossy().to_string(),
        created_at: now.to_rfc3339(),
        expires_at: expires.to_rfc3339(),
    };

    let json = serde_json::to_string_pretty(&data).context("failed to serialize session")?;
    let session_path = vault_dir.join(SESSION_FILE);
    fs::write(&session_path, &json).context("failed to write session file")?;

    // Restrict file permissions on Unix (metadata only, but keep tidy).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&session_path, perms).context("failed to set session permissions")?;
    }

    Ok(())
}

/// Load a valid (non-expired) session, returning the vault key bytes.
///
/// Returns `None` if no session exists or if it has expired (auto-cleaned).
pub fn load_session(vault_dir: &Path) -> Result<Option<([u8; 32], SessionInfo)>> {
    let session_path = vault_dir.join(SESSION_FILE);

    if !session_path.exists() {
        return Ok(None);
    }

    let json = fs::read_to_string(&session_path).context("failed to read session file")?;
    let data: SessionData = serde_json::from_str(&json).context("failed to parse session file")?;

    let expires_at = DateTime::parse_from_rfc3339(&data.expires_at)
        .context("invalid expires_at in session")?
        .with_timezone(&Utc);

    if Utc::now() >= expires_at {
        // Session expired — scrub key and metadata.
        clear_key(vault_dir);
        let _ = fs::remove_file(&session_path);
        return Ok(None);
    }

    // Fetch the key from the OS keystore.
    let Some(hex) = fetch_key_hex(vault_dir)? else {
        // Metadata present but key gone (e.g. reboot cleared keyutils). Treat
        // as locked and clean up the stale metadata.
        let _ = fs::remove_file(&session_path);
        return Ok(None);
    };

    let key_bytes = hex_decode(&hex).context("invalid vault key in keystore")?;

    let info = SessionInfo {
        vault_path: data.vault_path,
        expires_at,
    };

    Ok(Some((key_bytes, info)))
}

/// Delete the session (lock the vault): remove the keystore key and metadata.
///
/// Returns `true` if any session state existed.
pub fn delete_session(vault_dir: &Path) -> Result<bool> {
    let key_existed = clear_key(vault_dir);

    let session_path = vault_dir.join(SESSION_FILE);
    let file_existed = session_path.exists();
    if file_existed {
        fs::remove_file(&session_path).context("failed to delete session file")?;
    }

    Ok(key_existed || file_existed)
}

/// Fetch the stored session key as a hex string, if present.
fn fetch_key_hex(vault_dir: &Path) -> Result<Option<Zeroizing<String>>> {
    match keyring_entry(vault_dir)?.get_password() {
        Ok(hex) => Ok(Some(Zeroizing::new(hex))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow::anyhow!(
            "failed to read session key from OS keystore: {e}"
        )),
    }
}

/// Remove the session key from the OS keystore. Returns `true` if one existed.
fn clear_key(vault_dir: &Path) -> bool {
    let Ok(entry) = keyring_entry(vault_dir) else {
        return false;
    };
    matches!(entry.delete_credential(), Ok(()))
}

/// Information about the current session (no key material).
pub struct SessionInfo {
    #[allow(dead_code)]
    pub vault_path: String,
    pub expires_at: DateTime<Utc>,
}

/// Resolve the vault directory from an optional `--vault` path.
///
/// If a path is provided, uses its parent directory. Otherwise, uses
/// `~/.ldgr/`.
pub fn resolve_vault_dir(vault_flag: Option<&Path>) -> PathBuf {
    if let Some(p) = vault_flag {
        p.parent().unwrap_or(p).to_path_buf()
    } else {
        default_vault_dir()
    }
}

/// Resolve the full vault file path from an optional `--vault` flag.
pub fn resolve_vault_path(vault_flag: Option<&Path>) -> PathBuf {
    vault_flag.map_or_else(|| default_vault_dir().join("vault.ldgr"), PathBuf::from)
}

/// Default vault directory: `~/.ldgr/`
pub fn default_vault_dir() -> PathBuf {
    home_dir().join(".ldgr")
}

fn home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        write!(s, "{b:02x}").expect("writing to String never fails");
        s
    })
}

fn hex_decode(hex: &str) -> Result<[u8; 32]> {
    if hex.len() != 64 {
        bail!("expected 64 hex characters, got {}", hex.len());
    }
    let mut bytes = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk)?;
        bytes[i] = u8::from_str_radix(s, 16)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    /// Route all keyring access through the in-memory mock so tests never touch
    /// (or depend on) a real OS keystore.
    fn init_mock_keyring() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        });
    }

    #[test]
    fn session_file_persists_no_key_material() {
        init_mock_keyring();
        let dir = tempfile::tempdir().unwrap();
        let vault_dir = dir.path();
        let vault_path = vault_dir.join("vault.ldgr");
        let key = [0xABu8; 32];

        create_session(vault_dir, &vault_path, &key, 15).unwrap();

        let contents = fs::read_to_string(vault_dir.join(SESSION_FILE)).unwrap();
        // The raw key hex must NOT appear anywhere in the on-disk metadata.
        let key_hex = hex_encode(&key);
        assert!(
            !contents.contains(&key_hex),
            "session file must not contain key material"
        );
        assert!(!contents.contains("vault_key"));
        // But it should carry the non-secret metadata we rely on.
        assert!(contents.contains("expires_at"));
        assert!(contents.contains("vault_path"));
    }

    #[test]
    fn expired_session_is_cleaned_up() {
        init_mock_keyring();
        let dir = tempfile::tempdir().unwrap();
        let vault_dir = dir.path();
        let vault_path = vault_dir.join("vault.ldgr");

        // Negative timeout => already expired.
        create_session(vault_dir, &vault_path, &[1u8; 32], -1).unwrap();
        assert!(load_session(vault_dir).unwrap().is_none());
        // Metadata file is removed on expiry.
        assert!(!vault_dir.join(SESSION_FILE).exists());
    }

    #[test]
    fn delete_session_removes_metadata_file() {
        init_mock_keyring();
        let dir = tempfile::tempdir().unwrap();
        let vault_dir = dir.path();
        let vault_path = vault_dir.join("vault.ldgr");

        create_session(vault_dir, &vault_path, &[2u8; 32], 15).unwrap();
        assert!(vault_dir.join(SESSION_FILE).exists());

        assert!(delete_session(vault_dir).unwrap());
        assert!(!vault_dir.join(SESSION_FILE).exists());
        // Second delete reports nothing to remove.
        assert!(!delete_session(vault_dir).unwrap());
    }
}
