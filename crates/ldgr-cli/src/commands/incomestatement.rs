//! `ldgr incomestatement` — income statement report.

use std::path::Path;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};

use ldgr_core::accounting::query::Query;
use ldgr_core::accounting::reports::compute_income_statement;
use ldgr_core::storage::accounts::ListOptions;
use ldgr_core::storage::transactions::list_transactions;

use crate::convert;
use crate::db;

/// Run the `incomestatement` command.
pub fn run(vault_path: &Path, query_terms: &[String], output: &str) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let store_txns = list_transactions(&conn, &ListOptions::default())?;
    let transactions = convert::to_accounting_txns(&store_txns);
    let query = Query::parse(query_terms);
    let report = compute_income_statement(&transactions, &query);

    match output {
        "json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "income": format_section_json(&report.income),
                    "expenses": format_section_json(&report.expenses),
                    "net_income": format_totals_json(&report.net_income),
                }))
                .unwrap_or_default()
            );
        }
        "csv" => {
            println!("section,account,commodity,balance");
            for ab in &report.income {
                for (c, q) in &ab.balances {
                    println!("income,{},{},{}", ab.account, c, q);
                }
            }
            for ab in &report.expenses {
                for (c, q) in &ab.balances {
                    println!("expenses,{},{},{}", ab.account, c, q);
                }
            }
        }
        _ => {
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(vec!["Account", "Balance"]);

            if !report.income.is_empty() {
                table.add_row(vec!["── Income ──", ""]);
                for ab in &report.income {
                    table.add_row(vec![
                        &format!("  {}", short_name(&ab.account)),
                        &format_balances(&ab.balances),
                    ]);
                }
                table.add_row(vec![
                    "  Total Income",
                    &format_balances(&report.income_totals),
                ]);
            }

            if !report.expenses.is_empty() {
                table.add_row(vec!["── Expenses ──", ""]);
                for ab in &report.expenses {
                    table.add_row(vec![
                        &format!("  {}", short_name(&ab.account)),
                        &format_balances(&ab.balances),
                    ]);
                }
                table.add_row(vec![
                    "  Total Expenses",
                    &format_balances(&report.expense_totals),
                ]);
            }

            table.add_row(vec!["────────", "────────"]);
            table.add_row(vec!["Net Income", &format_balances(&report.net_income)]);

            if report.income.is_empty() && report.expenses.is_empty() {
                eprintln!("No income or expense transactions found.");
            } else {
                println!("{table}");
            }
        }
    }

    Ok(())
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

fn format_section_json(
    accounts: &[ldgr_core::accounting::AccountBalance],
) -> Vec<serde_json::Value> {
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
}

fn format_totals_json(
    totals: &std::collections::BTreeMap<String, rust_decimal::Decimal>,
) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = totals
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.to_string())))
        .collect();
    serde_json::Value::Object(map)
}
