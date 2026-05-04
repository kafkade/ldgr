//! `ldgr balance` — hierarchical account balance report.

use std::path::Path;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};

use ldgr_core::accounting::reports::compute_balance;
use ldgr_core::storage::accounts::ListOptions;
use ldgr_core::storage::transactions::list_transactions;

use crate::convert;
use crate::db;

/// Run the `balance` command.
pub fn run(
    vault_path: &Path,
    account_filter: Option<&str>,
    begin: Option<&str>,
    end: Option<&str>,
    flat: bool,
    output: &str,
) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let store_txns = list_transactions(&conn, &ListOptions::default())?;
    let transactions = convert::to_accounting_txns(&store_txns);

    let report = compute_balance(&transactions, account_filter, begin, end);

    if report.accounts.is_empty() {
        eprintln!("No matching transactions found.");
        return Ok(());
    }

    match output {
        "json" => print_json(&report),
        "csv" => print_csv(&report),
        _ => print_table(&report, flat),
    }

    Ok(())
}

fn print_table(report: &ldgr_core::accounting::BalanceReport, flat: bool) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec!["Account", "Balance"]);

    for ab in &report.accounts {
        let name = if flat {
            ab.account.clone()
        } else {
            format!("{}{}", "  ".repeat(ab.depth), short_name(&ab.account))
        };

        let balance_str = format_balances(&ab.balances);
        table.add_row(vec![&name, &balance_str]);
    }

    // Totals row
    if !report.totals.is_empty() {
        let totals_str = format_balances(&report.totals);
        table.add_row(vec!["────────", "────────"]);
        table.add_row(vec!["Total", &totals_str]);
    }

    println!("{table}");
}

fn print_json(report: &ldgr_core::accounting::BalanceReport) {
    let entries: Vec<serde_json::Value> = report
        .accounts
        .iter()
        .map(|ab| {
            let balances: serde_json::Map<String, serde_json::Value> = ab
                .balances
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.to_string())))
                .collect();
            serde_json::json!({
                "account": ab.account,
                "balances": balances,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&entries).unwrap_or_default()
    );
}

fn print_csv(report: &ldgr_core::accounting::BalanceReport) {
    println!("account,commodity,balance");
    for ab in &report.accounts {
        for (commodity, qty) in &ab.balances {
            println!("{},{},{}", ab.account, commodity, qty);
        }
    }
}

fn short_name(account: &str) -> &str {
    account.rsplit(':').next().unwrap_or(account)
}

fn format_balances(balances: &std::collections::BTreeMap<String, rust_decimal::Decimal>) -> String {
    balances
        .iter()
        .map(|(commodity, qty)| {
            if commodity.is_empty() {
                qty.to_string()
            } else {
                format!("{qty} {commodity}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
