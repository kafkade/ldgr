//! Session file management for vault unlock persistence.
//!
//! When a vault is unlocked, the vault key is cached in a session file
//! so subsequent CLI commands can access the vault without re-entering
//! the password. The session expires after a configurable timeout.
//!
//! **Security**: the session file contains raw vault key material.
//! File permissions are restricted to owner-only on Unix.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Default session timeout in minutes.
pub const DEFAULT_TIMEOUT_MINUTES: i64 = 15;

/// Session file name within the vault directory.
const SESSION_FILE: &str = "session.json";

#[derive(Serialize, Deserialize)]
struct SessionData {
    vault_path: String,
    vault_key_hex: String,
    created_at: String,
    expires_at: String,
}

/// Create a new session file after a successful vault unlock.
pub fn create_session(
    vault_dir: &Path,
    vault_path: &Path,
    vault_key: &[u8; 32],
    timeout_minutes: i64,
) -> Result<()> {
    let now = Utc::now();
    let expires = now + Duration::minutes(timeout_minutes);

    let data = SessionData {
        vault_path: vault_path.to_string_lossy().to_string(),
        vault_key_hex: hex_encode(vault_key),
        created_at: now.to_rfc3339(),
        expires_at: expires.to_rfc3339(),
    };

    let json = serde_json::to_string_pretty(&data).context("failed to serialize session")?;
    let session_path = vault_dir.join(SESSION_FILE);

    fs::write(&session_path, &json).context("failed to write session file")?;

    // Restrict file permissions on Unix
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
/// Returns `None` if no session exists or if it has expired (auto-deleted).
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
        // Session expired — clean up
        let _ = fs::remove_file(&session_path);
        return Ok(None);
    }

    let key_bytes = hex_decode(&data.vault_key_hex).context("invalid vault key in session")?;

    let info = SessionInfo {
        vault_path: data.vault_path,
        expires_at,
    };

    Ok(Some((key_bytes, info)))
}

/// Delete the session file (lock the vault).
pub fn delete_session(vault_dir: &Path) -> Result<bool> {
    let session_path = vault_dir.join(SESSION_FILE);
    if session_path.exists() {
        fs::remove_file(&session_path).context("failed to delete session file")?;
        Ok(true)
    } else {
        Ok(false)
    }
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
