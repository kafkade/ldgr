//! `ldgr unlock` — unlock the vault with the master password.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use ldgr_core::crypto::open_vault;

use crate::session;

/// Run the `unlock` command.
pub fn run(vault_path: &Path, timeout_minutes: i64) -> Result<()> {
    if !vault_path.exists() {
        bail!(
            "No vault found at {}\nRun `ldgr init` to create one.",
            vault_path.display()
        );
    }

    let vault_dir = session::resolve_vault_dir(Some(vault_path));

    // Check if already unlocked
    if let Some((_key, info)) = session::load_session(&vault_dir)? {
        let remaining = info.expires_at - chrono::Utc::now();
        let mins = remaining.num_minutes();
        eprintln!("Vault is already unlocked ({} min remaining).", mins.max(0));
        return Ok(());
    }

    let password =
        rpassword::prompt_password("Master password: ").context("failed to read password")?;

    let data =
        fs::read(vault_path).with_context(|| format!("failed to read {}", vault_path.display()))?;

    eprintln!("Deriving keys...");

    let vault = open_vault(&data, password.as_bytes()).map_err(|e| match e {
        ldgr_core::crypto::CryptoError::UnwrapFailed => {
            anyhow::anyhow!(
                "Wrong password. Try again or use `ldgr recover` with your recovery key."
            )
        }
        other => anyhow::anyhow!("Failed to unlock vault: {other}"),
    })?;

    // Cache session
    let session_key = vault.export_session_key();
    session::create_session(&vault_dir, vault_path, &session_key, timeout_minutes)?;

    eprintln!("✓ Vault unlocked (session expires in {timeout_minutes} min).");
    eprintln!("  Vault: {}", vault.metadata().name);
    eprintln!("  Items: {}", vault.item_count());

    Ok(())
}
