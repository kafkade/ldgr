//! `ldgr lock` — lock the vault by clearing the session.

use std::path::Path;

use anyhow::Result;

use crate::session;

/// Run the `lock` command.
pub fn run(vault_path: &Path) -> Result<()> {
    let vault_dir = session::resolve_vault_dir(Some(vault_path));

    if session::delete_session(&vault_dir)? {
        eprintln!("✓ Vault locked. Session cleared.");
    } else {
        eprintln!("Vault is already locked (no active session).");
    }

    Ok(())
}
