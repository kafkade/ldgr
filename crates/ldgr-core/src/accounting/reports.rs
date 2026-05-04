//! Balance and register report computation.
//!
//! Pure functions that take transactions and produce structured report data.
//! No I/O — the CLI handles database queries and output formatting.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::accounting::types::{Posting, Transaction};

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
                    .account
                    .to_lowercase()
                    .contains(&filter.to_lowercase())
                {
                    continue;
                }
            }

            let (qty, commodity) = parse_posting_amount(posting);
            *balances
                .entry(posting.account.clone())
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
                    .account
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
                account: posting.account.clone(),
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
    match &posting.amount {
        Some(amt) => (amt.quantity, amt.commodity.clone()),
        None => (Decimal::ZERO, String::new()),
    }
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

// ── Income Statement ───────────────────────────────────────────────────────────

/// Income statement: Revenue minus Expenses for a period.
#[derive(Debug, Clone)]
pub struct IncomeStatement {
    pub income: Vec<AccountBalance>,
    pub expenses: Vec<AccountBalance>,
    pub income_totals: BTreeMap<String, Decimal>,
    pub expense_totals: BTreeMap<String, Decimal>,
    pub net_income: BTreeMap<String, Decimal>,
}

/// Compute an income statement for the given transactions.
pub fn compute_income_statement(
    transactions: &[Transaction],
    query: &super::query::Query,
) -> IncomeStatement {
    let income_report = compute_filtered(transactions, query, |acct| {
        let top = acct.split(':').next().unwrap_or("").to_lowercase();
        top == "income" || top == "revenue" || top == "revenues"
    });
    let expense_report = compute_filtered(transactions, query, |acct| {
        let top = acct.split(':').next().unwrap_or("").to_lowercase();
        top == "expenses" || top == "expense"
    });

    let mut net_income = BTreeMap::new();
    for (commodity, qty) in &income_report.totals {
        *net_income.entry(commodity.clone()).or_insert(Decimal::ZERO) += qty;
    }
    for (commodity, qty) in &expense_report.totals {
        *net_income.entry(commodity.clone()).or_insert(Decimal::ZERO) += qty;
    }

    IncomeStatement {
        income: income_report.accounts,
        expenses: expense_report.accounts,
        income_totals: income_report.totals,
        expense_totals: expense_report.totals,
        net_income,
    }
}

// ── Balance Sheet ──────────────────────────────────────────────────────────────

/// Balance sheet: Assets - Liabilities = Equity at a point in time.
#[derive(Debug, Clone)]
pub struct BalanceSheet {
    pub assets: Vec<AccountBalance>,
    pub liabilities: Vec<AccountBalance>,
    pub equity: Vec<AccountBalance>,
    pub asset_totals: BTreeMap<String, Decimal>,
    pub liability_totals: BTreeMap<String, Decimal>,
    pub equity_totals: BTreeMap<String, Decimal>,
}

/// Compute a balance sheet from the given transactions.
pub fn compute_balance_sheet(
    transactions: &[Transaction],
    query: &super::query::Query,
) -> BalanceSheet {
    let assets = compute_filtered(transactions, query, |acct| {
        let top = acct.split(':').next().unwrap_or("").to_lowercase();
        top == "assets" || top == "asset"
    });
    let liabilities = compute_filtered(transactions, query, |acct| {
        let top = acct.split(':').next().unwrap_or("").to_lowercase();
        top == "liabilities" || top == "liability"
    });
    let equity_report = compute_filtered(transactions, query, |acct| {
        let top = acct.split(':').next().unwrap_or("").to_lowercase();
        top == "equity"
    });

    BalanceSheet {
        assets: assets.accounts,
        liabilities: liabilities.accounts,
        equity: equity_report.accounts,
        asset_totals: assets.totals,
        liability_totals: liabilities.totals,
        equity_totals: equity_report.totals,
    }
}

// ── Query-aware helpers ────────────────────────────────────────────────────────

fn compute_filtered(
    transactions: &[Transaction],
    query: &super::query::Query,
    account_predicate: impl Fn(&str) -> bool,
) -> BalanceReport {
    let mut balances: BTreeMap<String, BTreeMap<String, Decimal>> = BTreeMap::new();

    for txn in transactions {
        if !query.matches_transaction(txn) {
            continue;
        }
        for posting in &txn.postings {
            if !account_predicate(&posting.account) {
                continue;
            }
            if !query.matches_posting(posting, txn) {
                continue;
            }
            let (qty, commodity) = parse_posting_amount(posting);
            *balances
                .entry(posting.account.clone())
                .or_default()
                .entry(commodity)
                .or_insert(Decimal::ZERO) += qty;
        }
    }

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

    let mut totals: BTreeMap<String, Decimal> = BTreeMap::new();
    for ab in &accounts {
        for (commodity, qty) in &ab.balances {
            *totals.entry(commodity.clone()).or_insert(Decimal::ZERO) += qty;
        }
    }

    BalanceReport { accounts, totals }
}

/// Compute balances using the query system.
pub fn compute_balance_with_query(
    transactions: &[Transaction],
    query: &super::query::Query,
) -> BalanceReport {
    compute_filtered(transactions, query, |_| true)
}

/// Compute register using the query system.
pub fn compute_register_with_query(
    transactions: &[Transaction],
    query: &super::query::Query,
) -> RegisterReport {
    let mut sorted: Vec<&Transaction> = transactions
        .iter()
        .filter(|txn| query.matches_transaction(txn))
        .collect();
    sorted.sort_by(|a, b| a.date.cmp(&b.date));

    let mut entries = Vec::new();
    let mut running: BTreeMap<String, Decimal> = BTreeMap::new();

    for txn in &sorted {
        for posting in &txn.postings {
            if !query.matches_posting(posting, txn) {
                continue;
            }
            let (qty, commodity) = parse_posting_amount(posting);
            let balance = running.entry(commodity.clone()).or_insert(Decimal::ZERO);
            *balance += qty;
            entries.push(RegisterEntry {
                date: txn.date.clone(),
                description: txn.description.clone(),
                account: posting.account.clone(),
                amount: qty,
                commodity,
                running_balance: *balance,
            });
        }
    }

    RegisterReport { entries }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::types::{Amount, Posting, Status, Transaction};
    use std::collections::HashMap;

    fn make_txn(date: &str, desc: &str, postings: Vec<(&str, &str, &str)>) -> Transaction {
        Transaction {
            date: date.into(),
            status: Status::Cleared,
            code: None,
            description: desc.into(),
            comment: None,
            tags: HashMap::new(),
            source_line: 0,
            postings: postings
                .into_iter()
                .map(|(acct, qty, comm)| Posting {
                    account: acct.into(),
                    amount: Some(Amount {
                        quantity: qty.parse().unwrap(),
                        commodity: comm.into(),
                    }),
                    balance_assertion: None,
                    status: Status::Unmarked,
                    comment: None,
                    tags: HashMap::new(),
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
