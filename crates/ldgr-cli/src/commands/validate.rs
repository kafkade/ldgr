//! `ldgr validate` — check if a journal file is importable.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use ldgr_core::accounting::parser::parse_journal;

/// Run the `validate` command.
pub fn run(file: &str) -> Result<()> {
    let path = Path::new(file);
    if !path.exists() {
        eprintln!("✗ File not found: {file}");
        std::process::exit(1);
    }

    let content = fs::read_to_string(path).with_context(|| format!("failed to read '{file}'"))?;

    match parse_journal(&content) {
        Ok(journal) => {
            eprintln!("✓ Journal is valid and importable.");
            eprintln!();
            print_statistics(&journal);
        }
        Err(errors) => {
            eprintln!("✗ Journal has {} error(s):", errors.len());
            eprintln!();
            for err in &errors {
                eprintln!("  {err}");
                eprintln!();
            }
            std::process::exit(1);
        }
    }

    Ok(())
}

fn print_statistics(journal: &ldgr_core::accounting::Journal) {
    eprintln!("Statistics:");
    eprintln!("  Transactions:  {}", journal.transactions.len());

    // Count unique accounts
    let mut accounts: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut commodities: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut min_date: Option<&str> = None;
    let mut max_date: Option<&str> = None;

    for txn in &journal.transactions {
        for posting in &txn.postings {
            accounts.insert(&posting.account);
            if let Some(amt) = &posting.amount
                && !amt.commodity.is_empty()
            {
                commodities.insert(&amt.commodity);
            }
        }
        let d = txn.date.as_str();
        if min_date.is_none_or(|m| d < m) {
            min_date = Some(d);
        }
        if max_date.is_none_or(|m| d > m) {
            max_date = Some(d);
        }
    }

    eprintln!("  Accounts:      {}", accounts.len());
    eprintln!("  Commodities:   {}", commodities.len());

    if let (Some(min), Some(max)) = (min_date, max_date) {
        eprintln!("  Date range:    {min} to {max}");
    }

    eprintln!(
        "  Declarations:  {} account, {} commodity, {} price",
        journal.account_declarations.len(),
        journal.commodity_declarations.len(),
        journal.price_directives.len(),
    );
}
