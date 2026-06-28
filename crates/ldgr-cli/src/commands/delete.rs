//! `ldgr delete` — soft-delete a transaction.

use std::path::Path;

use anyhow::{Result, bail};

use ldgr_core::storage::accounts::ListOptions;
use ldgr_core::storage::transactions::{get_transaction, soft_delete_transaction_with_sync};

use crate::db;
use crate::sync::bridge::cli_sync_context;

/// Run the `delete` command.
pub fn run(vault_path: &Path, id: &str, force: bool) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;

    let txn = get_transaction(&conn, id, &ListOptions::default())?
        .ok_or_else(|| anyhow::anyhow!("Transaction '{id}' not found"))?;

    if !force {
        eprintln!(
            "Delete transaction: {} {} ({} postings)?",
            txn.date,
            txn.description,
            txn.postings.len()
        );
        eprint!("Type 'yes' to confirm: ");
        std::io::Write::flush(&mut std::io::stderr())?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim() != "yes" {
            bail!("Cancelled.");
        }
    }

    soft_delete_transaction_with_sync(&conn, id, &cli_sync_context(&conn)?)?;
    eprintln!("✓ Deleted transaction: {} {}", txn.date, txn.description);
    Ok(())
}
