//! `ldgr balancesheet` — balance sheet report.

use std::path::Path;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};

use ldgr_core::accounting::query::Query;
use ldgr_core::accounting::reports::compute_balance_sheet;
use ldgr_core::storage::accounts::ListOptions;
use ldgr_core::storage::transactions::list_transactions;

use crate::convert;
use crate::db;

/// Run the `balancesheet` command.
pub fn run(vault_path: &Path, query_terms: &[String], output: &str) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let store_txns = list_transactions(&conn, &ListOptions::default())?;
    let transactions = convert::to_accounting_txns(&store_txns);
    let query = Query::parse(query_terms);
    let report = compute_balance_sheet(&transactions, &query);

    match output {
        "json" => print_json(&report),
        "csv" => print_csv(&report),
        _ => print_table(&report),
    }

    Ok(())
}

fn print_table(report: &ldgr_core::accounting::BalanceSheet) {
    let has_data =
        !report.assets.is_empty() || !report.liabilities.is_empty() || !report.equity.is_empty();

    if !has_data {
        eprintln!("No asset, liability, or equity transactions found.");
        return;
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec!["Account", "Balance"]);

    if !report.assets.is_empty() {
        table.add_row(vec!["── Assets ──", ""]);
        for ab in &report.assets {
            table.add_row(vec![
                &format!("  {}", short_name(&ab.account)),
                &format_balances(&ab.balances),
            ]);
        }
        table.add_row(vec![
            "  Total Assets",
            &format_balances(&report.asset_totals),
        ]);
    }

    if !report.liabilities.is_empty() {
        table.add_row(vec!["── Liabilities ──", ""]);
        for ab in &report.liabilities {
            table.add_row(vec![
                &format!("  {}", short_name(&ab.account)),
                &format_balances(&ab.balances),
            ]);
        }
        table.add_row(vec![
            "  Total Liabilities",
            &format_balances(&report.liability_totals),
        ]);
    }

    if !report.equity.is_empty() {
        table.add_row(vec!["── Equity ──", ""]);
        for ab in &report.equity {
            table.add_row(vec![
                &format!("  {}", short_name(&ab.account)),
                &format_balances(&ab.balances),
            ]);
        }
        table.add_row(vec![
            "  Total Equity",
            &format_balances(&report.equity_totals),
        ]);
    }

    println!("{table}");
}

fn print_json(report: &ldgr_core::accounting::BalanceSheet) {
    let to_json = |accounts: &[ldgr_core::accounting::AccountBalance]| -> Vec<serde_json::Value> {
        accounts
            .iter()
            .map(|ab| {
                let bals: serde_json::Map<String, serde_json::Value> = ab
                    .balances
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.to_string())))
                    .collect();
                serde_json::json!({"account": ab.account, "balances": bals})
            })
            .collect()
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "assets": to_json(&report.assets),
            "liabilities": to_json(&report.liabilities),
            "equity": to_json(&report.equity),
        }))
        .unwrap_or_default()
    );
}

fn print_csv(report: &ldgr_core::accounting::BalanceSheet) {
    println!("section,account,commodity,balance");
    for ab in &report.assets {
        for (c, q) in &ab.balances {
            println!("assets,{},{},{}", ab.account, c, q);
        }
    }
    for ab in &report.liabilities {
        for (c, q) in &ab.balances {
            println!("liabilities,{},{},{}", ab.account, c, q);
        }
    }
    for ab in &report.equity {
        for (c, q) in &ab.balances {
            println!("equity,{},{},{}", ab.account, c, q);
        }
    }
}

fn short_name(account: &str) -> &str {
    account.rsplit(':').next().unwrap_or(account)
}

fn format_balances(balances: &std::collections::BTreeMap<String, rust_decimal::Decimal>) -> String {
    balances
        .iter()
        .map(|(c, q)| {
            if c.is_empty() {
                q.to_string()
            } else {
                format!("{q} {c}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
