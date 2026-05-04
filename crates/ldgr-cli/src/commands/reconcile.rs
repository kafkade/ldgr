//! `ldgr reconcile` — interactive account reconciliation.

use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Result, bail};
use rust_decimal::Decimal;

use ldgr_core::storage::accounts::get_account_by_name;
use ldgr_core::storage::transactions::{
    TransactionStatus, list_transactions_for_account, set_transaction_status,
};

use crate::db;

/// Run the `reconcile` command.
#[allow(clippy::too_many_lines)]
pub fn run(vault_path: &Path, account_name: &str) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;

    let account = get_account_by_name(&conn, account_name)?
        .ok_or_else(|| anyhow::anyhow!("Account '{account_name}' not found"))?;

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    // Get statement details
    let stmt_date = prompt("Statement date (YYYY-MM-DD): ", &mut lines)?;
    let stmt_balance_str = prompt("Statement ending balance: ", &mut lines)?;
    let stmt_balance: Decimal = stmt_balance_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid balance: {e}"))?;

    // Get unreconciled transactions for this account
    let unreconciled = list_transactions_for_account(
        &conn,
        &account.id,
        Some(TransactionStatus::Unmarked),
        Some(&stmt_date),
    )?;

    let pending = list_transactions_for_account(
        &conn,
        &account.id,
        Some(TransactionStatus::Pending),
        Some(&stmt_date),
    )?;

    let all_unreconciled: Vec<_> = unreconciled.into_iter().chain(pending).collect();

    if all_unreconciled.is_empty() {
        eprintln!("No unreconciled transactions found for {account_name}.");
        return Ok(());
    }

    // Calculate already-cleared balance
    let cleared = list_transactions_for_account(
        &conn,
        &account.id,
        Some(TransactionStatus::Cleared),
        Some(&stmt_date),
    )?;

    let mut cleared_balance = Decimal::ZERO;
    for txn in &cleared {
        for posting in &txn.postings {
            if posting.account_id == account.id {
                if let Some(qty) = &posting.amount_quantity {
                    if let Ok(d) = qty.parse::<Decimal>() {
                        cleared_balance += d;
                    }
                }
            }
        }
    }

    eprintln!();
    eprintln!("Reconciling: {account_name}");
    eprintln!("Statement date: {stmt_date}");
    eprintln!("Statement balance: {stmt_balance}");
    eprintln!("Already cleared: {cleared_balance}");
    eprintln!("Difference: {}", stmt_balance - cleared_balance);
    eprintln!();
    eprintln!(
        "{} unreconciled transaction(s). Mark each as [c]leared, [s]kip, or [q]uit:",
        all_unreconciled.len()
    );
    eprintln!();

    let mut running_balance = cleared_balance;
    let mut cleared_count = 0;
    let mut skipped_count = 0;

    for txn in &all_unreconciled {
        // Find the posting amount for this account
        let mut posting_amount = Decimal::ZERO;
        let mut posting_commodity = String::new();
        for posting in &txn.postings {
            if posting.account_id == account.id {
                if let Some(qty) = &posting.amount_quantity {
                    if let Ok(d) = qty.parse::<Decimal>() {
                        posting_amount = d;
                    }
                }
                if let Some(c) = &posting.amount_commodity {
                    posting_commodity.clone_from(c);
                }
            }
        }

        let amount_display = if posting_commodity.is_empty() {
            posting_amount.to_string()
        } else {
            format!("{posting_amount} {posting_commodity}")
        };

        eprintln!(
            "  {} | {:<30} | {:>15} | running: {}",
            txn.date,
            truncate(&txn.description, 30),
            amount_display,
            running_balance + posting_amount,
        );

        let choice = prompt("  [c]lear / [s]kip / [q]uit? ", &mut lines)?;

        match choice.to_lowercase().as_str() {
            "c" | "clear" | "y" | "yes" => {
                set_transaction_status(&conn, &txn.id, TransactionStatus::Cleared)?;
                running_balance += posting_amount;
                cleared_count += 1;

                if running_balance == stmt_balance {
                    eprintln!();
                    eprintln!("✓ Balanced! Running balance matches statement ({stmt_balance}).");
                    eprintln!(
                        "  {cleared_count} cleared, {skipped_count} skipped, {} remaining.",
                        all_unreconciled.len() - cleared_count - skipped_count
                    );
                    return Ok(());
                }
            }
            "s" | "skip" | "n" | "no" => {
                skipped_count += 1;
            }
            "q" | "quit" => {
                eprintln!();
                eprintln!(
                    "Reconciliation paused. {cleared_count} cleared so far. Resume with `ldgr reconcile {account_name}`."
                );
                return Ok(());
            }
            _ => {
                eprintln!("  (unknown input, skipping)");
                skipped_count += 1;
            }
        }
    }

    eprintln!();
    if running_balance == stmt_balance {
        eprintln!("✓ Reconciliation complete! Balance matches statement.");
    } else {
        let diff = stmt_balance - running_balance;
        eprintln!(
            "⚠ Reconciliation incomplete. Difference: {diff} (expected {stmt_balance}, got {running_balance})."
        );
        eprintln!("  {cleared_count} cleared, {skipped_count} skipped.");
    }

    Ok(())
}

fn prompt(message: &str, lines: &mut io::Lines<io::StdinLock<'_>>) -> Result<String> {
    eprint!("{message}");
    io::stderr().flush()?;
    match lines.next() {
        Some(Ok(line)) => Ok(line.trim().to_string()),
        Some(Err(e)) => bail!("Failed to read input: {e}"),
        None => bail!("Unexpected end of input"),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
