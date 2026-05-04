//! `ldgr add` — add a new transaction.

use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Result, bail};
use rust_decimal::Decimal;

use ldgr_core::storage::accounts::get_account_by_name;
use ldgr_core::storage::transactions::{
    NewPosting, NewTransaction, TransactionStatus, create_transaction,
};

use crate::db;

/// Run the `add` command in non-interactive mode.
pub fn run_noninteractive(
    vault_path: &Path,
    date: &str,
    description: &str,
    status: &str,
    postings_raw: &[String],
) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let txn_status = parse_status(status)?;

    if postings_raw.is_empty() {
        bail!("At least one posting is required. Use --posting 'Account  Amount Commodity'");
    }

    let mut postings = Vec::new();
    for raw in postings_raw {
        postings.push(parse_posting_arg(raw)?);
    }

    validate_balance(&postings)?;

    // Verify all accounts exist
    for p in &postings {
        if get_account_by_name(&conn, &p.account_id)?.is_none() {
            bail!(
                "Account '{}' not found. Create it with `ldgr accounts add {}`.",
                p.account_id,
                p.account_id
            );
        }
    }

    let txn = create_transaction(
        &conn,
        &NewTransaction {
            date: date.to_string(),
            status: txn_status,
            code: None,
            description: description.to_string(),
            comment: None,
            postings,
        },
    )?;

    eprintln!(
        "✓ Transaction added: {} {} ({} postings)",
        txn.date,
        txn.description,
        txn.postings.len()
    );
    Ok(())
}

/// Run the `add` command in interactive mode.
pub fn run_interactive(vault_path: &Path) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    let date = prompt("Date (YYYY-MM-DD): ", &mut lines)?;
    let description = prompt("Description: ", &mut lines)?;

    let status_str = prompt(
        "Status [unmarked/pending/cleared] (default: unmarked): ",
        &mut lines,
    )?;
    let status = if status_str.is_empty() {
        TransactionStatus::Unmarked
    } else {
        parse_status(&status_str)?
    };

    eprintln!("Enter postings (blank account to finish):");
    let mut postings = Vec::new();
    let mut posting_num = 1;

    loop {
        let account = prompt(&format!("  Posting {posting_num} account: "), &mut lines)?;
        if account.is_empty() {
            break;
        }

        let amount_str = prompt(
            &format!("  Posting {posting_num} amount (blank for auto-balance): "),
            &mut lines,
        )?;

        let (quantity, commodity) = if amount_str.is_empty() {
            (None, None)
        } else {
            let parts: Vec<&str> = amount_str.split_whitespace().collect();
            match parts.len() {
                1 => {
                    let q: Decimal = parts[0]
                        .parse()
                        .map_err(|e| anyhow::anyhow!("Invalid amount: {e}"))?;
                    (Some(q.to_string()), None)
                }
                2 => {
                    let q: Decimal = parts[0]
                        .parse()
                        .map_err(|e| anyhow::anyhow!("Invalid amount: {e}"))?;
                    (Some(q.to_string()), Some(parts[1].to_string()))
                }
                _ => bail!("Amount should be 'QUANTITY' or 'QUANTITY COMMODITY'"),
            }
        };

        // Verify account exists
        if get_account_by_name(&conn, &account)?.is_none() {
            bail!("Account '{account}' not found. Create it with `ldgr accounts add {account}`.");
        }

        postings.push(NewPosting {
            account_id: account,
            amount_quantity: quantity,
            amount_commodity: commodity,
            balance_assertion_quantity: None,
            balance_assertion_commodity: None,
        });
        posting_num += 1;
    }

    if postings.is_empty() {
        bail!("Transaction must have at least one posting.");
    }

    validate_balance(&postings)?;

    let txn = create_transaction(
        &conn,
        &NewTransaction {
            date,
            status,
            code: None,
            description: description.clone(),
            comment: None,
            postings,
        },
    )?;

    eprintln!(
        "✓ Transaction added: {} {} ({} postings)",
        txn.date,
        description,
        txn.postings.len()
    );
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn parse_status(s: &str) -> Result<TransactionStatus> {
    match s.to_lowercase().as_str() {
        "unmarked" | "" | "u" => Ok(TransactionStatus::Unmarked),
        "pending" | "p" | "!" => Ok(TransactionStatus::Pending),
        "cleared" | "c" | "*" => Ok(TransactionStatus::Cleared),
        _ => bail!("Unknown status: '{s}'. Use: unmarked, pending, cleared"),
    }
}

/// Parse a posting argument: `"Account  Amount Commodity"` or `"Account"` (auto-balance).
///
/// Account and amount are separated by 2+ spaces (same as hledger syntax).
fn parse_posting_arg(raw: &str) -> Result<NewPosting> {
    // Split at 2+ spaces
    let bytes = raw.as_bytes();
    let mut i = 0;
    let mut split_pos = None;

    while i < bytes.len() {
        if bytes[i] == b' ' {
            let start = i;
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
            if i - start >= 2 {
                split_pos = Some((start, i));
                break;
            }
        } else {
            i += 1;
        }
    }

    let (account, amount_part) = match split_pos {
        Some((start, end)) => (raw[..start].trim(), raw[end..].trim()),
        None => (raw.trim(), ""),
    };

    if account.is_empty() {
        bail!("Posting missing account name");
    }

    if amount_part.is_empty() {
        return Ok(NewPosting {
            account_id: account.to_string(),
            amount_quantity: None,
            amount_commodity: None,
            balance_assertion_quantity: None,
            balance_assertion_commodity: None,
        });
    }

    let parts: Vec<&str> = amount_part.split_whitespace().collect();
    let (quantity, commodity) = match parts.len() {
        1 => {
            let _q: Decimal = parts[0]
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid amount '{amount_part}': {e}"))?;
            (parts[0].to_string(), None)
        }
        2 => {
            let _q: Decimal = parts[0]
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid amount '{amount_part}': {e}"))?;
            (parts[0].to_string(), Some(parts[1].to_string()))
        }
        _ => bail!("Invalid posting amount: '{amount_part}'"),
    };

    Ok(NewPosting {
        account_id: account.to_string(),
        amount_quantity: Some(quantity),
        amount_commodity: commodity,
        balance_assertion_quantity: None,
        balance_assertion_commodity: None,
    })
}

/// Validate that postings balance (sum to zero).
///
/// At most one posting may omit an amount (auto-balance).
fn validate_balance(postings: &[NewPosting]) -> Result<()> {
    let mut auto_balance_count = 0;
    let mut sum = Decimal::ZERO;

    for p in postings {
        match &p.amount_quantity {
            Some(q) => {
                let qty: Decimal = q
                    .parse()
                    .map_err(|e| anyhow::anyhow!("Invalid amount '{q}': {e}"))?;
                sum += qty;
            }
            None => auto_balance_count += 1,
        }
    }

    if auto_balance_count > 1 {
        bail!("At most one posting can omit the amount (auto-balance).");
    }

    if auto_balance_count == 0 && sum != Decimal::ZERO {
        bail!(
            "Transaction does not balance: postings sum to {sum}.\n\
             All postings must sum to zero, or leave one amount blank for auto-balance."
        );
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
