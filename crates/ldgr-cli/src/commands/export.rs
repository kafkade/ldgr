//! `ldgr export` — export transactions to CSV, JSON, hledger journal, or a PDF report.

use std::path::Path;

use anyhow::{Result, bail};

use ldgr_core::accounting::query::Query;
use ldgr_core::accounting::report_document::{
    ReportDocument, balance_sheet_document, income_statement_document, net_worth_document,
};
use ldgr_core::accounting::reports::{
    compute_balance_sheet, compute_income_statement, compute_net_worth,
};
use ldgr_core::export::{csv, hledger, json};
use ldgr_core::storage::accounts::ListOptions;
use ldgr_core::storage::transactions::list_transactions;

use crate::convert;
use crate::db;
use crate::render::pdf;

/// Run the `export` command.
pub fn run(
    vault_path: &Path,
    format: &str,
    report: Option<&str>,
    output: Option<&str>,
    query_terms: &[String],
) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let store_txns = list_transactions(&conn, &ListOptions::default())?;
    let all_txns = convert::to_accounting_txns(&store_txns);

    if format == "pdf" {
        return run_pdf(&all_txns, report, output, query_terms);
    }

    let query = Query::parse(query_terms);
    let filtered: Vec<_> = all_txns
        .into_iter()
        .filter(|t| query.matches_transaction(t))
        .collect();

    if filtered.is_empty() {
        eprintln!("No matching transactions to export.");
        return Ok(());
    }

    let text = match format {
        "hledger" | "journal" => hledger::to_hledger(&filtered),
        "csv" => csv::to_csv(&filtered),
        "json" => json::to_json(&filtered),
        _ => {
            eprintln!("Unknown format: '{format}'. Use: hledger, csv, json, pdf");
            return Ok(());
        }
    };

    if let Some(path) = output {
        std::fs::write(path, text.as_bytes())?;
        eprintln!(
            "Exported {} transaction(s) as {format} to {path}.",
            filtered.len()
        );
    } else {
        print!("{text}");
        eprintln!("Exported {} transaction(s) as {format}.", filtered.len());
    }

    Ok(())
}

/// Build the requested report and write it as a PDF file.
fn run_pdf(
    transactions: &[ldgr_core::accounting::Transaction],
    report: Option<&str>,
    output: Option<&str>,
    query_terms: &[String],
) -> Result<()> {
    let Some(report) = report else {
        bail!("--format pdf requires --report <balancesheet|incomestatement|networth>");
    };
    let Some(output) = output else {
        bail!("--format pdf requires --output <file.pdf>");
    };

    let query = Query::parse(query_terms);
    let period = period_label(query_terms);

    let document: ReportDocument = match report.to_lowercase().as_str() {
        "balancesheet" | "balance-sheet" | "bs" => {
            let r = compute_balance_sheet(transactions, &query);
            balance_sheet_document(&r, period)
        }
        "incomestatement" | "income-statement" | "is" => {
            let r = compute_income_statement(transactions, &query);
            income_statement_document(&r, period)
        }
        "networth" | "net-worth" | "nw" => {
            let r = compute_net_worth(transactions, &query);
            net_worth_document(&r, period)
        }
        other => {
            bail!("Unknown report '{other}'. Use: balancesheet, incomestatement, networth");
        }
    };

    let bytes = pdf::render_report(&document)?;
    std::fs::write(output, &bytes)?;
    eprintln!(
        "Wrote {} report ({} bytes) to {output}.",
        document.title,
        bytes.len()
    );

    Ok(())
}

/// Derive a human-readable period subtitle from `date:`/`begin:`/`end:` query terms.
fn period_label(query_terms: &[String]) -> Option<String> {
    let mut begin: Option<&str> = None;
    let mut end: Option<&str> = None;
    let mut date: Option<&str> = None;
    for term in query_terms {
        if let Some(v) = term.strip_prefix("begin:") {
            begin = Some(v);
        } else if let Some(v) = term.strip_prefix("end:") {
            end = Some(v);
        } else if let Some(v) = term.strip_prefix("date:") {
            date = Some(v);
        }
    }
    match (begin, end, date) {
        (Some(b), Some(e), _) => Some(format!("{b} to {e}")),
        (Some(b), None, _) => Some(format!("From {b}")),
        (None, Some(e), _) => Some(format!("Through {e}")),
        (None, None, Some(d)) => Some(d.to_string()),
        (None, None, None) => None,
    }
}
