//! `ldgr export` — export transactions to CSV, JSON, or hledger journal.

use std::path::Path;

use anyhow::Result;

use ldgr_core::accounting::query::Query;
use ldgr_core::export::{csv, hledger, json};
use ldgr_core::storage::accounts::ListOptions;
use ldgr_core::storage::transactions::list_transactions;

use crate::convert;
use crate::db;

/// Run the `export` command.
pub fn run(vault_path: &Path, format: &str, query_terms: &[String]) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let store_txns = list_transactions(&conn, &ListOptions::default())?;
    let all_txns = convert::to_accounting_txns(&store_txns);

    let query = Query::parse(query_terms);
    let filtered: Vec<_> = all_txns
        .into_iter()
        .filter(|t| query.matches_transaction(t))
        .collect();

    if filtered.is_empty() {
        eprintln!("No matching transactions to export.");
        return Ok(());
    }

    let output = match format {
        "hledger" | "journal" => hledger::to_hledger(&filtered),
        "csv" => csv::to_csv(&filtered),
        "json" => json::to_json(&filtered),
        _ => {
            eprintln!("Unknown format: '{format}'. Use: hledger, csv, json");
            return Ok(());
        }
    };

    print!("{output}");

    eprintln!("Exported {} transaction(s) as {format}.", filtered.len());

    Ok(())
}
