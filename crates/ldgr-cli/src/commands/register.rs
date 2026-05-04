//! `ldgr register` — chronological transaction register with running balance.

use std::path::Path;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};

use ldgr_core::accounting::reports::compute_register;
use ldgr_core::storage::accounts::ListOptions;
use ldgr_core::storage::transactions::list_transactions;

use crate::convert;
use crate::db;

/// Run the `register` command.
pub fn run(
    vault_path: &Path,
    account_filter: Option<&str>,
    begin: Option<&str>,
    end: Option<&str>,
    output: &str,
) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let store_txns = list_transactions(&conn, &ListOptions::default())?;
    let transactions = convert::to_accounting_txns(&store_txns);

    let report = compute_register(&transactions, account_filter, begin, end);

    if report.entries.is_empty() {
        eprintln!("No matching transactions found.");
        return Ok(());
    }

    match output {
        "json" => print_json(&report),
        "csv" => print_csv(&report),
        _ => print_table(&report),
    }

    Ok(())
}

fn print_table(report: &ldgr_core::accounting::RegisterReport) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec!["Date", "Description", "Account", "Amount", "Balance"]);

    let mut prev_date = String::new();
    let mut prev_desc = String::new();

    for entry in &report.entries {
        // Avoid repeating date and description for same transaction
        let date = if entry.date == prev_date {
            String::new()
        } else {
            prev_date.clone_from(&entry.date);
            prev_desc.clear();
            entry.date.clone()
        };

        let desc = if entry.description == prev_desc {
            String::new()
        } else {
            prev_desc.clone_from(&entry.description);
            entry.description.clone()
        };

        let amount = format_amount(entry.amount, &entry.commodity);
        let balance = format_amount(entry.running_balance, &entry.commodity);

        table.add_row(vec![&date, &desc, &entry.account, &amount, &balance]);
    }

    println!("{table}");
}

fn print_json(report: &ldgr_core::accounting::RegisterReport) {
    let entries: Vec<serde_json::Value> = report
        .entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "date": e.date,
                "description": e.description,
                "account": e.account,
                "amount": e.amount.to_string(),
                "commodity": e.commodity,
                "running_balance": e.running_balance.to_string(),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&entries).unwrap_or_default()
    );
}

fn print_csv(report: &ldgr_core::accounting::RegisterReport) {
    println!("date,description,account,amount,commodity,running_balance");
    for e in &report.entries {
        println!(
            "{},{},{},{},{},{}",
            e.date, e.description, e.account, e.amount, e.commodity, e.running_balance
        );
    }
}

fn format_amount(qty: rust_decimal::Decimal, commodity: &str) -> String {
    if commodity.is_empty() {
        qty.to_string()
    } else {
        format!("{qty} {commodity}")
    }
}
