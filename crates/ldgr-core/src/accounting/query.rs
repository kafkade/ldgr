//! Query language for filtering transactions.
//!
//! Syntax (subset of hledger):
//! - `acct:Expenses` — account name substring (case-insensitive)
//! - `desc:grocery` — description substring (case-insensitive)
//! - `date:2024` — year, `date:2024-01` — month, `date:2024-01-01..2024-01-31` — range
//! - `amt:>100` / `amt:<50` — amount filters
//! - `tag:project` / `tag:project=alpha` — tag filters
//! - `not:acct:Expenses` — negation
//! - Multiple terms are AND'd

use rust_decimal::Decimal;

use super::types::{Posting, Transaction};

/// A parsed query containing zero or more filters (AND'd together).
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub filters: Vec<Filter>,
}

/// A single filter condition.
#[derive(Debug, Clone)]
pub enum Filter {
    Account(String),
    Description(String),
    DateYear(String),
    DateMonth(String),
    DateRange {
        begin: Option<String>,
        end: Option<String>,
    },
    AmountGt(Decimal),
    AmountLt(Decimal),
    Tag(String, Option<String>),
    Not(Box<Filter>),
}

impl Query {
    /// Parse a query from a list of search terms.
    pub fn parse(terms: &[String]) -> Self {
        let mut filters = Vec::new();
        for term in terms {
            if let Some(f) = parse_filter(term) {
                filters.push(f);
            }
        }
        Query { filters }
    }

    /// Check if a transaction matches all filters.
    pub fn matches_transaction(&self, txn: &Transaction) -> bool {
        self.filters.iter().all(|f| filter_matches_txn(f, txn))
    }

    /// Check if a posting matches all posting-level filters.
    pub fn matches_posting(&self, posting: &Posting, txn: &Transaction) -> bool {
        self.filters
            .iter()
            .all(|f| filter_matches_posting(f, posting, txn))
    }

    /// Whether this query has any filters.
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

fn parse_filter(term: &str) -> Option<Filter> {
    if let Some(rest) = term.strip_prefix("not:") {
        return parse_filter(rest).map(|f| Filter::Not(Box::new(f)));
    }
    if let Some(rest) = term.strip_prefix("acct:") {
        return Some(Filter::Account(rest.to_string()));
    }
    if let Some(rest) = term.strip_prefix("desc:") {
        return Some(Filter::Description(rest.to_string()));
    }
    if let Some(rest) = term.strip_prefix("date:") {
        return Some(parse_date_filter(rest));
    }
    if let Some(rest) = term.strip_prefix("amt:>")
        && let Ok(d) = rest.parse::<Decimal>()
    {
        return Some(Filter::AmountGt(d));
    }
    if let Some(rest) = term.strip_prefix("amt:<")
        && let Ok(d) = rest.parse::<Decimal>()
    {
        return Some(Filter::AmountLt(d));
    }
    if let Some(rest) = term.strip_prefix("tag:") {
        if let Some((key, value)) = rest.split_once('=') {
            return Some(Filter::Tag(key.to_string(), Some(value.to_string())));
        }
        return Some(Filter::Tag(rest.to_string(), None));
    }
    // Bare term = account filter
    Some(Filter::Account(term.to_string()))
}

fn parse_date_filter(s: &str) -> Filter {
    if let Some((begin, end)) = s.split_once("..") {
        return Filter::DateRange {
            begin: if begin.is_empty() {
                None
            } else {
                Some(begin.to_string())
            },
            end: if end.is_empty() {
                None
            } else {
                Some(end.to_string())
            },
        };
    }
    match s.len() {
        7 => Filter::DateMonth(s.to_string()),
        10 => Filter::DateRange {
            begin: Some(s.to_string()),
            end: Some(s.to_string()),
        },
        _ => Filter::DateYear(s.to_string()),
    }
}

fn filter_matches_txn(filter: &Filter, txn: &Transaction) -> bool {
    match filter {
        Filter::Account(pat) => txn
            .postings
            .iter()
            .any(|p| p.account.to_lowercase().contains(&pat.to_lowercase())),
        Filter::Description(pat) => txn.description.to_lowercase().contains(&pat.to_lowercase()),
        Filter::DateYear(y) => txn.date.starts_with(y.as_str()),
        Filter::DateMonth(m) => txn.date.starts_with(m.as_str()),
        Filter::DateRange { begin, end } => {
            if let Some(b) = begin
                && txn.date.as_str() < b.as_str()
            {
                return false;
            }
            if let Some(e) = end
                && txn.date.as_str() > e.as_str()
            {
                return false;
            }
            true
        }
        Filter::AmountGt(threshold) => txn.postings.iter().any(|p| {
            p.amount
                .as_ref()
                .is_some_and(|a| a.quantity.abs() > *threshold)
        }),
        Filter::AmountLt(threshold) => txn.postings.iter().any(|p| {
            p.amount
                .as_ref()
                .is_some_and(|a| a.quantity.abs() < *threshold)
        }),
        Filter::Tag(key, value) => {
            // Check transaction tags
            match value {
                Some(v) => txn.tags.get(key).is_some_and(|tv| tv == v),
                None => txn.tags.contains_key(key),
            }
        }
        Filter::Not(inner) => !filter_matches_txn(inner, txn),
    }
}

fn filter_matches_posting(filter: &Filter, posting: &Posting, txn: &Transaction) -> bool {
    match filter {
        Filter::Account(pat) => posting.account.to_lowercase().contains(&pat.to_lowercase()),
        Filter::Not(inner) => !filter_matches_posting(inner, posting, txn),
        _ => filter_matches_txn(filter, txn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::types::{Amount, Status, Transaction};
    use std::collections::HashMap;

    fn make_txn(date: &str, desc: &str, account: &str, amount: &str) -> Transaction {
        Transaction {
            date: date.into(),
            status: Status::Cleared,
            code: None,
            description: desc.into(),
            comment: None,
            tags: HashMap::new(),
            source_line: 0,
            postings: vec![Posting {
                account: account.into(),
                amount: Some(Amount {
                    quantity: amount.parse().unwrap(),
                    commodity: "USD".into(),
                }),
                balance_assertion: None,
                status: Status::Unmarked,
                comment: None,
                tags: HashMap::new(),
            }],
        }
    }

    #[test]
    fn account_filter() {
        let q = Query::parse(&["acct:Expenses".into()]);
        assert!(q.matches_transaction(&make_txn("2024-01-15", "Test", "Expenses:Food", "42")));
        assert!(!q.matches_transaction(&make_txn("2024-01-15", "Test", "Assets:Cash", "42")));
    }

    #[test]
    fn description_filter() {
        let q = Query::parse(&["desc:grocery".into()]);
        assert!(q.matches_transaction(&make_txn(
            "2024-01-15",
            "Weekly Grocery",
            "Expenses:Food",
            "42"
        )));
        assert!(!q.matches_transaction(&make_txn(
            "2024-01-15",
            "Gas Station",
            "Expenses:Gas",
            "42"
        )));
    }

    #[test]
    fn date_year_filter() {
        let q = Query::parse(&["date:2024".into()]);
        assert!(q.matches_transaction(&make_txn("2024-06-15", "Test", "A", "1")));
        assert!(!q.matches_transaction(&make_txn("2023-12-31", "Test", "A", "1")));
    }

    #[test]
    fn date_range_filter() {
        let q = Query::parse(&["date:2024-01-10..2024-01-20".into()]);
        assert!(q.matches_transaction(&make_txn("2024-01-15", "Test", "A", "1")));
        assert!(!q.matches_transaction(&make_txn("2024-01-05", "Test", "A", "1")));
    }

    #[test]
    fn amount_gt_filter() {
        let q = Query::parse(&["amt:>100".into()]);
        assert!(q.matches_transaction(&make_txn("2024-01-15", "Test", "A", "150")));
        assert!(!q.matches_transaction(&make_txn("2024-01-15", "Test", "A", "50")));
    }

    #[test]
    fn not_filter() {
        let q = Query::parse(&["not:acct:Expenses".into()]);
        assert!(!q.matches_transaction(&make_txn("2024-01-15", "Test", "Expenses:Food", "42")));
        assert!(q.matches_transaction(&make_txn("2024-01-15", "Test", "Assets:Cash", "42")));
    }

    #[test]
    fn multiple_filters_and() {
        let q = Query::parse(&["acct:Expenses".into(), "date:2024".into()]);
        assert!(q.matches_transaction(&make_txn("2024-01-15", "Test", "Expenses:Food", "42")));
        assert!(!q.matches_transaction(&make_txn("2023-01-15", "Test", "Expenses:Food", "42")));
    }

    #[test]
    fn bare_term_is_account_filter() {
        let q = Query::parse(&["Expenses".into()]);
        assert!(q.matches_transaction(&make_txn("2024-01-15", "Test", "Expenses:Food", "42")));
    }

    #[test]
    fn tag_filter() {
        let mut txn = make_txn("2024-01-15", "Test", "A", "1");
        txn.tags.insert("project".into(), "alpha".into());
        let q = Query::parse(&["tag:project=alpha".into()]);
        assert!(q.matches_transaction(&txn));
        let q2 = Query::parse(&["tag:project=beta".into()]);
        assert!(!q2.matches_transaction(&txn));
    }

    #[test]
    fn empty_query_matches_all() {
        let q = Query::default();
        assert!(q.matches_transaction(&make_txn("2024-01-15", "Test", "A", "1")));
    }
}
