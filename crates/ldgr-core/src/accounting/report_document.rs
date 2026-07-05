//! Layout-agnostic report document model.
//!
//! Pure, I/O-free, WASM-safe (per ADR-005). This module converts the computed
//! report structs (`BalanceSheet`, `IncomeStatement`, `NetWorth`, ...) into a
//! serializable, presentation-neutral [`ReportDocument`] describing *what* to
//! render — titles, sections, indented rows, and per-commodity totals — without
//! knowing anything about the output format (PDF, HTML, ...).
//!
//! Byte-level rendering (e.g. PDF) lives in platform crates such as the CLI,
//! which are allowed to pull in heavier dependencies. Keeping this model in
//! core lets every platform share one description of a report.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Serialize;

use super::reports::{AccountBalance, BalanceSheet, IncomeStatement, NetWorth};

/// A single monetary amount for one commodity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Amount {
    /// Commodity/currency code (e.g. `"USD"`). Empty for commodity-less amounts.
    pub commodity: String,
    /// The value. Always a `Decimal` — never a float.
    pub value: Decimal,
}

impl Amount {
    /// Render this amount as a single human-readable string (`"1234.50 USD"`).
    #[must_use]
    pub fn display(&self) -> String {
        if self.commodity.is_empty() {
            self.value.to_string()
        } else {
            format!("{} {}", self.value, self.commodity)
        }
    }
}

/// One line in a report section.
#[derive(Debug, Clone, Serialize)]
pub struct ReportRow {
    /// The row label (usually an account name or a total caption).
    pub label: String,
    /// Nesting depth for indentation (0 = top-level).
    pub depth: usize,
    /// One entry per commodity, grouped so multi-currency rows render cleanly.
    pub amounts: Vec<Amount>,
    /// Whether the row should be visually emphasized (e.g. totals, net figures).
    pub emphasis: bool,
}

impl ReportRow {
    /// Render all amounts as one string, one commodity per line joined by `\n`.
    #[must_use]
    pub fn amounts_display(&self) -> String {
        if self.amounts.is_empty() {
            return String::new();
        }
        self.amounts
            .iter()
            .map(Amount::display)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A titled group of rows, optionally closed by a total row.
#[derive(Debug, Clone, Serialize)]
pub struct ReportSection {
    pub heading: String,
    pub rows: Vec<ReportRow>,
    /// Optional section total (rendered emphasized beneath the rows).
    pub total: Option<ReportRow>,
}

/// A presentation-neutral description of a rendered report.
#[derive(Debug, Clone, Serialize)]
pub struct ReportDocument {
    pub title: String,
    /// Optional period/subtitle line (e.g. `"2024-01-01 to 2024-12-31"`).
    pub period: Option<String>,
    pub sections: Vec<ReportSection>,
    /// Optional footnote (e.g. generation context).
    pub note: Option<String>,
}

// ── Conversion helpers ──────────────────────────────────────────────────────────

fn amounts_from_map(balances: &BTreeMap<String, Decimal>) -> Vec<Amount> {
    balances
        .iter()
        // Drop commodity-less zero entries: these are noise from elided/unpriced
        // postings (`parse_posting_amount` maps a missing amount to `(0, "")`),
        // never a meaningful figure in a currency report.
        .filter(|(commodity, value)| !(commodity.is_empty() && **value == Decimal::ZERO))
        .map(|(commodity, value)| Amount {
            commodity: commodity.clone(),
            value: *value,
        })
        .collect()
}

/// Leaf account name (last `:`-separated segment).
fn short_name(account: &str) -> &str {
    account.rsplit(':').next().unwrap_or(account)
}

fn account_row(ab: &AccountBalance) -> ReportRow {
    ReportRow {
        label: short_name(&ab.account).to_string(),
        depth: ab.depth + 1,
        amounts: amounts_from_map(&ab.balances),
        emphasis: false,
    }
}

fn total_row(label: &str, totals: &BTreeMap<String, Decimal>) -> ReportRow {
    ReportRow {
        label: label.to_string(),
        depth: 1,
        amounts: amounts_from_map(totals),
        emphasis: true,
    }
}

fn section(
    heading: &str,
    accounts: &[AccountBalance],
    total_label: &str,
    totals: &BTreeMap<String, Decimal>,
) -> ReportSection {
    ReportSection {
        heading: heading.to_string(),
        rows: accounts.iter().map(account_row).collect(),
        total: Some(total_row(total_label, totals)),
    }
}

/// Build a [`ReportDocument`] for a balance sheet.
#[must_use]
pub fn balance_sheet_document(sheet: &BalanceSheet, period: Option<String>) -> ReportDocument {
    ReportDocument {
        title: "Balance Sheet".to_string(),
        period,
        sections: vec![
            section("Assets", &sheet.assets, "Total Assets", &sheet.asset_totals),
            section(
                "Liabilities",
                &sheet.liabilities,
                "Total Liabilities",
                &sheet.liability_totals,
            ),
            section(
                "Equity",
                &sheet.equity,
                "Total Equity",
                &sheet.equity_totals,
            ),
        ],
        note: None,
    }
}

/// Build a [`ReportDocument`] for an income statement.
#[must_use]
pub fn income_statement_document(
    statement: &IncomeStatement,
    period: Option<String>,
) -> ReportDocument {
    ReportDocument {
        title: "Income Statement".to_string(),
        period,
        sections: vec![
            section(
                "Income",
                &statement.income,
                "Total Income",
                &statement.income_totals,
            ),
            section(
                "Expenses",
                &statement.expenses,
                "Total Expenses",
                &statement.expense_totals,
            ),
            ReportSection {
                heading: "Net Income".to_string(),
                rows: Vec::new(),
                total: Some(total_row("Net Income", &statement.net_income)),
            },
        ],
        note: None,
    }
}

/// Build a [`ReportDocument`] for a net worth snapshot.
#[must_use]
pub fn net_worth_document(net_worth: &NetWorth, period: Option<String>) -> ReportDocument {
    let breakdown = ReportSection {
        heading: "Breakdown".to_string(),
        rows: vec![
            ReportRow {
                label: "Liquid Assets".to_string(),
                depth: 1,
                amounts: amounts_from_map(&net_worth.liquid),
                emphasis: false,
            },
            ReportRow {
                label: "Investments".to_string(),
                depth: 1,
                amounts: amounts_from_map(&net_worth.investments),
                emphasis: false,
            },
            ReportRow {
                label: "Total Assets".to_string(),
                depth: 1,
                amounts: amounts_from_map(&net_worth.assets),
                emphasis: false,
            },
            ReportRow {
                label: "Total Liabilities".to_string(),
                depth: 1,
                amounts: amounts_from_map(&net_worth.liabilities),
                emphasis: false,
            },
        ],
        total: Some(total_row("Net Worth", &net_worth.total)),
    };

    ReportDocument {
        title: "Net Worth".to_string(),
        period,
        sections: vec![breakdown],
        note: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shorthand for a two-decimal-place `Decimal` (e.g. `dec(1500, 2)` == 15.00).
    fn dec(mantissa: i64, scale: u32) -> Decimal {
        Decimal::new(mantissa, scale)
    }

    fn ab(account: &str, commodity: &str, value: Decimal) -> AccountBalance {
        let mut balances = BTreeMap::new();
        balances.insert(commodity.to_string(), value);
        AccountBalance {
            account: account.to_string(),
            balances,
            depth: account.matches(':').count(),
        }
    }

    #[test]
    fn balance_sheet_document_has_three_sections() {
        let mut asset_totals = BTreeMap::new();
        asset_totals.insert("USD".to_string(), dec(150_000, 2));
        let sheet = BalanceSheet {
            assets: vec![ab("Assets:Checking", "USD", dec(150_000, 2))],
            liabilities: vec![ab("Liabilities:Card", "USD", dec(-20_000, 2))],
            equity: vec![],
            asset_totals: asset_totals.clone(),
            liability_totals: {
                let mut m = BTreeMap::new();
                m.insert("USD".to_string(), dec(-20_000, 2));
                m
            },
            equity_totals: BTreeMap::new(),
        };

        let doc = balance_sheet_document(&sheet, Some("2024".to_string()));
        assert_eq!(doc.title, "Balance Sheet");
        assert_eq!(doc.period.as_deref(), Some("2024"));
        assert_eq!(doc.sections.len(), 3);

        let assets = &doc.sections[0];
        assert_eq!(assets.heading, "Assets");
        assert_eq!(assets.rows[0].label, "Checking");
        assert_eq!(assets.rows[0].amounts[0].value, dec(150_000, 2));
        let total = assets.total.as_ref().unwrap();
        assert!(total.emphasis);
        assert_eq!(total.amounts[0].display(), "1500.00 USD");
    }

    #[test]
    fn income_statement_document_includes_net_income() {
        let mut income_totals = BTreeMap::new();
        income_totals.insert("USD".to_string(), dec(-500_000, 2));
        let mut expense_totals = BTreeMap::new();
        expense_totals.insert("USD".to_string(), dec(300_000, 2));
        let mut net = BTreeMap::new();
        net.insert("USD".to_string(), dec(-200_000, 2));

        let statement = IncomeStatement {
            income: vec![ab("Income:Salary", "USD", dec(-500_000, 2))],
            expenses: vec![ab("Expenses:Rent", "USD", dec(300_000, 2))],
            income_totals,
            expense_totals,
            net_income: net,
        };

        let doc = income_statement_document(&statement, None);
        assert_eq!(doc.sections.len(), 3);
        let net_section = &doc.sections[2];
        assert_eq!(net_section.heading, "Net Income");
        assert!(net_section.rows.is_empty());
        assert_eq!(
            net_section.total.as_ref().unwrap().amounts[0].display(),
            "-2000.00 USD"
        );
    }

    #[test]
    fn net_worth_document_groups_breakdown() {
        let usd = |v: Decimal| {
            let mut m = BTreeMap::new();
            m.insert("USD".to_string(), v);
            m
        };
        let net_worth = NetWorth {
            total: usd(dec(50_000, 0)),
            assets: usd(dec(60_000, 0)),
            liabilities: usd(dec(-10_000, 0)),
            liquid: usd(dec(20_000, 0)),
            investments: usd(dec(40_000, 0)),
        };

        let doc = net_worth_document(&net_worth, None);
        assert_eq!(doc.title, "Net Worth");
        assert_eq!(doc.sections.len(), 1);
        let section = &doc.sections[0];
        assert_eq!(section.rows.len(), 4);
        assert_eq!(section.total.as_ref().unwrap().label, "Net Worth");
        assert_eq!(
            section.total.as_ref().unwrap().amounts[0].value,
            dec(50_000, 0)
        );
    }

    #[test]
    fn multi_currency_row_renders_each_commodity() {
        let mut balances = BTreeMap::new();
        balances.insert("USD".to_string(), dec(100, 0));
        balances.insert("EUR".to_string(), dec(50, 0));
        let row = ReportRow {
            label: "Cash".to_string(),
            depth: 1,
            amounts: amounts_from_map(&balances),
            emphasis: false,
        };
        // BTreeMap orders commodities: EUR before USD.
        assert_eq!(row.amounts_display(), "50 EUR\n100 USD");
    }

    #[test]
    fn commodity_less_zero_is_filtered_out() {
        let mut balances = BTreeMap::new();
        balances.insert(String::new(), dec(0, 0));
        balances.insert("USD".to_string(), dec(300_000, 2));
        let amounts = amounts_from_map(&balances);
        assert_eq!(amounts.len(), 1);
        assert_eq!(amounts[0].commodity, "USD");
    }
}
