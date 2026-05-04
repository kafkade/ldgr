//! Balance and register report computation.
//!
//! Pure functions that take transactions and produce structured report data.
//! No I/O — the CLI handles database queries and output formatting.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::storage::transactions::{Posting, Transaction};

// ── Balance Report ─────────────────────────────────────────────────────────────

/// A single account's balance across one or more commodities.
#[derive(Debug, Clone)]
pub struct AccountBalance {
    pub account: String,
    /// Balance per commodity (e.g., {"USD": 1500.00, "EUR": 200.00}).
    pub balances: BTreeMap<String, Decimal>,
    /// Nesting depth for display indentation (0 = top-level).
    pub depth: usize,
}

/// Computed balance report: account balances with totals.
#[derive(Debug, Clone)]
pub struct BalanceReport {
    pub accounts: Vec<AccountBalance>,
    pub totals: BTreeMap<String, Decimal>,
}

/// Compute account balances from a list of transactions.
///
/// Optionally filters by account name substring and/or date range.
pub fn compute_balance(
    transactions: &[Transaction],
    account_filter: Option<&str>,
    begin: Option<&str>,
    end: Option<&str>,
) -> BalanceReport {
    let mut balances: BTreeMap<String, BTreeMap<String, Decimal>> = BTreeMap::new();

    for txn in transactions {
        if !date_in_range(&txn.date, begin, end) {
            continue;
        }

        for posting in &txn.postings {
            if let Some(filter) = account_filter {
                if !posting
                    .account_id
                    .to_lowercase()
                    .contains(&filter.to_lowercase())
                {
                    continue;
                }
            }

            let (qty, commodity) = parse_posting_amount(posting);
            *balances
                .entry(posting.account_id.clone())
                .or_default()
                .entry(commodity)
                .or_insert(Decimal::ZERO) += qty;
        }
    }

    // Build sorted account list with depth
    let accounts: Vec<AccountBalance> = balances
        .into_iter()
        .map(|(account, bals)| {
            let depth = account.matches(':').count();
            AccountBalance {
                account,
                balances: bals,
                depth,
            }
        })
        .collect();

    // Compute totals
    let mut totals: BTreeMap<String, Decimal> = BTreeMap::new();
    for ab in &accounts {
        for (commodity, qty) in &ab.balances {
            *totals.entry(commodity.clone()).or_insert(Decimal::ZERO) += qty;
        }
    }

    BalanceReport { accounts, totals }
}

// ── Register Report ────────────────────────────────────────────────────────────

/// A single entry in the register report.
#[derive(Debug, Clone)]
pub struct RegisterEntry {
    pub date: String,
    pub description: String,
    pub account: String,
    pub amount: Decimal,
    pub commodity: String,
    pub running_balance: Decimal,
}

/// Computed register report: chronological posting list with running balance.
#[derive(Debug, Clone)]
pub struct RegisterReport {
    pub entries: Vec<RegisterEntry>,
}

/// Compute the register report from a list of transactions.
///
/// Transactions are sorted by date. Each posting becomes a register entry
/// with a running balance (per commodity — the running balance tracks
/// the primary commodity, defaulting to the first one seen).
pub fn compute_register(
    transactions: &[Transaction],
    account_filter: Option<&str>,
    begin: Option<&str>,
    end: Option<&str>,
) -> RegisterReport {
    // Sort by date ascending
    let mut sorted: Vec<&Transaction> = transactions.iter().collect();
    sorted.sort_by(|a, b| a.date.cmp(&b.date));

    let mut entries = Vec::new();
    let mut running: BTreeMap<String, Decimal> = BTreeMap::new();

    for txn in &sorted {
        if !date_in_range(&txn.date, begin, end) {
            continue;
        }

        for posting in &txn.postings {
            if let Some(filter) = account_filter {
                if !posting
                    .account_id
                    .to_lowercase()
                    .contains(&filter.to_lowercase())
                {
                    continue;
                }
            }

            let (qty, commodity) = parse_posting_amount(posting);
            let balance = running.entry(commodity.clone()).or_insert(Decimal::ZERO);
            *balance += qty;

            entries.push(RegisterEntry {
                date: txn.date.clone(),
                description: txn.description.clone(),
                account: posting.account_id.clone(),
                amount: qty,
                commodity,
                running_balance: *balance,
            });
        }
    }

    RegisterReport { entries }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn parse_posting_amount(posting: &Posting) -> (Decimal, String) {
    let qty = posting
        .amount_quantity
        .as_deref()
        .and_then(|s| s.parse::<Decimal>().ok())
        .unwrap_or(Decimal::ZERO);

    let commodity = posting
        .amount_commodity
        .as_deref()
        .unwrap_or("")
        .to_string();

    (qty, commodity)
}

fn date_in_range(date: &str, begin: Option<&str>, end: Option<&str>) -> bool {
    if let Some(b) = begin {
        if date < b {
            return false;
        }
    }
    if let Some(e) = end {
        if date > e {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::transactions::{Posting, Transaction, TransactionStatus};

    fn make_txn(date: &str, desc: &str, postings: Vec<(&str, &str, &str)>) -> Transaction {
        Transaction {
            id: String::new(),
            date: date.into(),
            status: TransactionStatus::Cleared,
            code: None,
            description: desc.into(),
            comment: None,
            created_at: String::new(),
            modified_at: String::new(),
            version: 1,
            deleted: false,
            postings: postings
                .into_iter()
                .enumerate()
                .map(|(i, (acct, qty, comm))| Posting {
                    id: String::new(),
                    transaction_id: String::new(),
                    account_id: acct.into(),
                    amount_quantity: Some(qty.into()),
                    amount_commodity: Some(comm.into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                    #[allow(clippy::cast_possible_wrap)]
                    posting_order: i as i64,
                    created_at: String::new(),
                    version: 1,
                })
                .collect(),
        }
    }

    // --- Balance report ---

    #[test]
    fn balance_sums_across_transactions() {
        let txns = vec![
            make_txn(
                "2024-01-15",
                "Groceries",
                vec![
                    ("Expenses:Food", "42.50", "USD"),
                    ("Assets:Checking", "-42.50", "USD"),
                ],
            ),
            make_txn(
                "2024-01-16",
                "Gas",
                vec![
                    ("Expenses:Transport", "35.00", "USD"),
                    ("Assets:Checking", "-35.00", "USD"),
                ],
            ),
        ];

        let report = compute_balance(&txns, None, None, None);

        let checking = report
            .accounts
            .iter()
            .find(|a| a.account == "Assets:Checking")
            .unwrap();
        assert_eq!(checking.balances["USD"], Decimal::new(-7750, 2));

        let food = report
            .accounts
            .iter()
            .find(|a| a.account == "Expenses:Food")
            .unwrap();
        assert_eq!(food.balances["USD"], Decimal::new(4250, 2));
    }

    #[test]
    fn balance_filters_by_account() {
        let txns = vec![make_txn(
            "2024-01-15",
            "Test",
            vec![
                ("Expenses:Food", "42.50", "USD"),
                ("Assets:Checking", "-42.50", "USD"),
            ],
        )];

        let report = compute_balance(&txns, Some("Expenses"), None, None);
        assert_eq!(report.accounts.len(), 1);
        assert_eq!(report.accounts[0].account, "Expenses:Food");
    }

    #[test]
    fn balance_filters_by_date_range() {
        let txns = vec![
            make_txn(
                "2024-01-10",
                "Early",
                vec![
                    ("Expenses:Food", "10", "USD"),
                    ("Assets:Cash", "-10", "USD"),
                ],
            ),
            make_txn(
                "2024-01-20",
                "Mid",
                vec![
                    ("Expenses:Food", "20", "USD"),
                    ("Assets:Cash", "-20", "USD"),
                ],
            ),
            make_txn(
                "2024-01-30",
                "Late",
                vec![
                    ("Expenses:Food", "30", "USD"),
                    ("Assets:Cash", "-30", "USD"),
                ],
            ),
        ];

        let report = compute_balance(
            &txns,
            Some("Expenses"),
            Some("2024-01-15"),
            Some("2024-01-25"),
        );
        assert_eq!(report.accounts.len(), 1);
        assert_eq!(report.accounts[0].balances["USD"], Decimal::new(20, 0));
    }

    #[test]
    fn balance_multi_commodity() {
        let txns = vec![
            make_txn(
                "2024-01-15",
                "USD Buy",
                vec![
                    ("Expenses:Food", "42.50", "USD"),
                    ("Assets:Checking", "-42.50", "USD"),
                ],
            ),
            make_txn(
                "2024-01-16",
                "EUR Buy",
                vec![
                    ("Expenses:Food", "38.00", "EUR"),
                    ("Assets:Checking", "-38.00", "EUR"),
                ],
            ),
        ];

        let report = compute_balance(&txns, Some("Expenses:Food"), None, None);
        let food = &report.accounts[0];
        assert_eq!(food.balances["USD"], Decimal::new(4250, 2));
        assert_eq!(food.balances["EUR"], Decimal::new(38, 0));
    }

    #[test]
    fn balance_totals_correct() {
        let txns = vec![make_txn(
            "2024-01-15",
            "Test",
            vec![
                ("Expenses:Food", "42.50", "USD"),
                ("Assets:Checking", "-42.50", "USD"),
            ],
        )];

        let report = compute_balance(&txns, None, None, None);
        // Total should be zero (balanced transaction)
        assert_eq!(report.totals["USD"], Decimal::ZERO);
    }

    #[test]
    fn balance_depth_correct() {
        let txns = vec![make_txn(
            "2024-01-15",
            "Test",
            vec![
                ("Expenses:Food:Groceries", "10", "USD"),
                ("Assets:Checking", "-10", "USD"),
            ],
        )];

        let report = compute_balance(&txns, None, None, None);
        let groceries = report
            .accounts
            .iter()
            .find(|a| a.account == "Expenses:Food:Groceries")
            .unwrap();
        assert_eq!(groceries.depth, 2);

        let checking = report
            .accounts
            .iter()
            .find(|a| a.account == "Assets:Checking")
            .unwrap();
        assert_eq!(checking.depth, 1);
    }

    // --- Register report ---

    #[test]
    fn register_chronological_order() {
        let txns = vec![
            make_txn(
                "2024-01-20",
                "Later",
                vec![
                    ("Expenses:Food", "20", "USD"),
                    ("Assets:Cash", "-20", "USD"),
                ],
            ),
            make_txn(
                "2024-01-10",
                "Earlier",
                vec![
                    ("Expenses:Food", "10", "USD"),
                    ("Assets:Cash", "-10", "USD"),
                ],
            ),
        ];

        let report = compute_register(&txns, Some("Expenses"), None, None);
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.entries[0].date, "2024-01-10");
        assert_eq!(report.entries[1].date, "2024-01-20");
    }

    #[test]
    fn register_running_balance() {
        let txns = vec![
            make_txn(
                "2024-01-10",
                "First",
                vec![
                    ("Expenses:Food", "10", "USD"),
                    ("Assets:Cash", "-10", "USD"),
                ],
            ),
            make_txn(
                "2024-01-20",
                "Second",
                vec![
                    ("Expenses:Food", "25", "USD"),
                    ("Assets:Cash", "-25", "USD"),
                ],
            ),
        ];

        let report = compute_register(&txns, Some("Expenses"), None, None);
        assert_eq!(report.entries[0].running_balance, Decimal::new(10, 0));
        assert_eq!(report.entries[1].running_balance, Decimal::new(35, 0));
    }

    #[test]
    fn register_filters_by_account() {
        let txns = vec![make_txn(
            "2024-01-15",
            "Test",
            vec![
                ("Expenses:Food", "42.50", "USD"),
                ("Assets:Checking", "-42.50", "USD"),
            ],
        )];

        let report = compute_register(&txns, Some("Assets"), None, None);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].account, "Assets:Checking");
    }

    #[test]
    fn register_empty_transactions() {
        let report = compute_register(&[], None, None, None);
        assert!(report.entries.is_empty());
    }
}
