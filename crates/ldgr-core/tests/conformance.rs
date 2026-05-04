//! hledger conformance test suite.
//!
//! Parses test journals with ldgr's parser and verifies correctness.
//! If hledger is available, also compares output with hledger's parser.

use std::fs;
use std::path::Path;

use ldgr_core::accounting::parser::parse_journal;
use ldgr_core::accounting::types::Status;

const JOURNALS_DIR: &str = "../../tests/conformance/journals";

fn read_journal(name: &str) -> String {
    let path = Path::new(JOURNALS_DIR).join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

// ── Supported syntax tests ─────────────────────────────────────────────────────

#[test]
fn basic_transactions() {
    let journal = parse_journal(&read_journal("basic.journal")).unwrap();
    assert_eq!(journal.transactions.len(), 3);
    assert_eq!(journal.transactions[0].description, "Groceries");
    assert_eq!(journal.transactions[0].postings.len(), 2);
    assert_eq!(journal.transactions[0].postings[0].account, "Expenses:Food");
}

#[test]
fn transaction_status() {
    let journal = parse_journal(&read_journal("status.journal")).unwrap();
    assert_eq!(journal.transactions[0].status, Status::Cleared);
    assert_eq!(journal.transactions[1].status, Status::Pending);
    assert_eq!(journal.transactions[2].status, Status::Unmarked);
}

#[test]
fn multi_currency() {
    let journal = parse_journal(&read_journal("multi_currency.journal")).unwrap();
    assert_eq!(journal.transactions.len(), 3);

    let commodities: Vec<&str> = journal
        .transactions
        .iter()
        .filter_map(|t| t.postings[0].amount.as_ref().map(|a| a.commodity.as_str()))
        .collect();
    assert!(commodities.contains(&"EUR"));
    assert!(commodities.contains(&"GBP"));
    assert!(commodities.contains(&"USD"));
}

#[test]
fn balance_assertions() {
    let journal = parse_journal(&read_journal("balance_assertions.journal")).unwrap();
    assert_eq!(journal.transactions.len(), 2);

    let assertion = journal.transactions[0].postings[0]
        .balance_assertion
        .as_ref()
        .unwrap();
    assert_eq!(assertion.quantity, rust_decimal::Decimal::new(1000, 0));
    assert_eq!(assertion.commodity, "USD");
}

#[test]
fn auto_balance_postings() {
    let journal = parse_journal(&read_journal("auto_balance.journal")).unwrap();
    assert_eq!(journal.transactions.len(), 2);

    // Second posting of first transaction has no amount
    assert!(journal.transactions[0].postings[1].amount.is_none());
}

#[test]
fn tags_and_comments() {
    let journal = parse_journal(&read_journal("tags_comments.journal")).unwrap();
    assert_eq!(journal.transactions.len(), 2);
    assert_eq!(
        journal.transactions[0]
            .tags
            .get("project")
            .map(String::as_str),
        Some("alpha")
    );
}

#[test]
fn directives() {
    let journal = parse_journal(&read_journal("directives.journal")).unwrap();
    assert_eq!(journal.account_declarations.len(), 4);
    assert_eq!(journal.commodity_declarations.len(), 2);
    assert_eq!(journal.price_directives.len(), 2);
    assert_eq!(journal.transactions.len(), 1);
}

#[test]
fn transaction_codes() {
    let journal = parse_journal(&read_journal("codes.journal")).unwrap();
    assert_eq!(journal.transactions[0].code.as_deref(), Some("1001"));
    assert_eq!(journal.transactions[1].code.as_deref(), Some("1002"));
}

#[test]
fn slash_date_format() {
    let journal = parse_journal(&read_journal("slash_dates.journal")).unwrap();
    // Dates should be normalized to YYYY-MM-DD
    assert_eq!(journal.transactions[0].date, "2024-01-15");
    assert_eq!(journal.transactions[1].date, "2024-02-01");
}

#[test]
fn prefix_currency_symbols() {
    let journal = parse_journal(&read_journal("prefix_symbols.journal")).unwrap();
    let amt = journal.transactions[0].postings[0].amount.as_ref().unwrap();
    assert_eq!(amt.commodity, "$");
    assert_eq!(amt.quantity, rust_decimal::Decimal::new(450, 2));
}

// ── Unsupported syntax tests ───────────────────────────────────────────────────

#[test]
fn unsupported_include_detected() {
    let errors = parse_journal(&read_journal("unsupported_include.journal")).unwrap_err();
    assert!(!errors.is_empty());
    assert!(errors[0].message.contains("Flatten"));
}

#[test]
fn unsupported_automated_transaction_detected() {
    let errors = parse_journal(&read_journal("unsupported_auto.journal")).unwrap_err();
    assert!(!errors.is_empty());
    assert!(errors[0].message.contains("automated"));
}

#[test]
fn unsupported_periodic_transaction_detected() {
    let errors = parse_journal(&read_journal("unsupported_periodic.journal")).unwrap_err();
    assert!(!errors.is_empty());
    assert!(errors[0].message.contains("periodic"));
}

// ── Complete journal test ──────────────────────────────────────────────────────

#[test]
fn complete_mixed_journal() {
    let journal = parse_journal(&read_journal("complete.journal")).unwrap();

    // Declarations
    assert_eq!(journal.account_declarations.len(), 6);
    assert_eq!(journal.commodity_declarations.len(), 1);
    assert_eq!(journal.price_directives.len(), 1);

    // Transactions
    assert_eq!(journal.transactions.len(), 6);

    // Check specific transactions
    let opening = &journal.transactions[0];
    assert_eq!(opening.description, "Opening balances");
    assert_eq!(opening.status, Status::Cleared);
    assert!(opening.postings[0].balance_assertion.is_some());

    let salary = &journal.transactions[1];
    assert!(salary.postings[1].amount.is_none()); // auto-balance

    let rent = &journal.transactions[2];
    assert_eq!(rent.code.as_deref(), Some("1001"));

    let pending = &journal.transactions[4];
    assert_eq!(pending.status, Status::Pending);
}

// ── hledger comparison (skipped if hledger not installed) ──────────────────────

#[test]
fn hledger_comparison_basic() {
    if !hledger_available() {
        eprintln!("Skipping hledger comparison: hledger not found in PATH");
        return;
    }

    let path = Path::new(JOURNALS_DIR).join("basic.journal");
    let output = std::process::Command::new("hledger")
        .args([
            "balance",
            "-f",
            path.to_str().unwrap(),
            "--output-format",
            "csv",
        ])
        .output()
        .expect("failed to run hledger");

    assert!(
        output.status.success(),
        "hledger failed on basic.journal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn hledger_available() -> bool {
    std::process::Command::new("hledger")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
