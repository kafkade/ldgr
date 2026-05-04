//! `ldgr status` — show vault status.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use ldgr_core::crypto::validate_vault;

use crate::session;

/// Run the `status` command.
pub fn run(vault_path: &Path) -> Result<()> {
    eprintln!("Vault: {}", vault_path.display());

    if !vault_path.exists() {
        eprintln!("Status: No vault found");
        eprintln!();
        eprintln!("Run `ldgr init` to create a new vault.");
        return Ok(());
    }

    // Read and validate the vault file
    let data =
        fs::read(vault_path).with_context(|| format!("failed to read {}", vault_path.display()))?;

    match validate_vault(&data) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Status: Invalid vault file ({e})");
            return Ok(());
        }
    }

    // Check format version from header
    if data.len() >= 6 {
        let version = u16::from_le_bytes([data[4], data[5]]);
        eprintln!("Format: v{version}");
    }

    // File metadata
    if let Ok(meta) = fs::metadata(vault_path) {
        let size = meta.len();
        eprintln!("Size: {}", format_size(size));

        if let Ok(modified) = meta.modified() {
            let dt: chrono::DateTime<chrono::Utc> = modified.into();
            eprintln!("Modified: {}", dt.format("%Y-%m-%d %H:%M:%S UTC"));
        }
    }

    // Session status
    let vault_dir = session::resolve_vault_dir(Some(vault_path));
    match session::load_session(&vault_dir) {
        Ok(Some((_key, info))) => {
            let remaining = info.expires_at - chrono::Utc::now();
            let mins = remaining.num_minutes();
            eprintln!("Status: Unlocked ({} min remaining)", mins.max(0));
        }
        Ok(None) => {
            eprintln!("Status: Locked");
        }
        Err(_) => {
            eprintln!("Status: Locked (session file unreadable)");
        }
    }

    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
