//! `ldgr migrate` — migrate a legacy plaintext working store to encrypted form.

use std::path::Path;

use anyhow::{Context, Result};

use crate::{migrate, session};

/// Run the `migrate` command.
///
/// Requires an unlocked vault (the session key is needed to encrypt the store).
pub fn run(vault_path: &Path) -> Result<()> {
    let vault_dir = session::resolve_vault_dir(Some(vault_path));

    let (key, _info) = session::load_session(&vault_dir)?
        .ok_or_else(|| anyhow::anyhow!("Vault is locked. Run `ldgr unlock` first."))?;

    let db_path = vault_dir.join("vault.db");
    if !db_path.exists() {
        eprintln!(
            "No working store found at {}. Nothing to migrate.",
            db_path.display()
        );
        return Ok(());
    }

    if !migrate::is_plaintext_sqlite(&db_path).context("failed to inspect the working store")? {
        eprintln!("✓ Working store is already encrypted at rest. Nothing to do.");
        return Ok(());
    }

    eprintln!("Migrating the local store to encrypted-at-rest format...");
    eprintln!("  This may take a while for large ledgers. Do not interrupt.");

    let migrated = migrate::migrate_if_plaintext(&db_path, &key)
        .context("failed to migrate the working store to the encrypted format")?;

    if migrated {
        eprintln!("✓ Migrated the local store to encrypted-at-rest format.");
        eprintln!("  A backup of the previous plaintext store was kept as vault.db.plaintext.bak.");
        eprintln!("  Once you have verified your data, you may delete that backup.");
    } else {
        eprintln!("✓ Working store is already encrypted at rest. Nothing to do.");
    }

    Ok(())
}
