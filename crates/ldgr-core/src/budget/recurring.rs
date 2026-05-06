//! Recurring transaction detection.
//!
//! Groups transactions by normalized payee, detects frequency patterns,
//! and classifies as subscription, variable recurring, or income.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::accounting::types::Transaction;

/// Detected frequency of a recurring transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frequency {
    Weekly,
    Biweekly,
    Monthly,
    Quarterly,
    Annual,
    Irregular,
}

/// Classification of a recurring pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecurringType {
    /// Fixed amount (subscriptions, rent).
    Subscription,
    /// Variable amount (utilities, groceries at same store).
    Variable,
    /// Income (salary, freelance).
    Income,
}

/// A detected recurring transaction pattern.
#[derive(Debug, Clone)]
pub struct RecurringPattern {
    pub payee: String,
    pub frequency: Frequency,
    pub recurring_type: RecurringType,
    pub average_amount: Decimal,
    pub occurrence_count: usize,
    pub last_date: String,
    pub account: String,
}

/// Detect recurring patterns from a list of transactions.
///
/// Groups by normalized payee, requires at least `min_occurrences` to
/// consider a pattern (default: 3).
pub fn detect_recurring(
    transactions: &[Transaction],
    min_occurrences: usize,
) -> Vec<RecurringPattern> {
    // Group by normalized payee
    let mut groups: BTreeMap<String, Vec<&Transaction>> = BTreeMap::new();
    for txn in transactions {
        let key = normalize_payee(&txn.description);
        if !key.is_empty() {
            groups.entry(key).or_default().push(txn);
        }
    }

    let mut patterns = Vec::new();

    for (payee, txns) in &groups {
        if txns.len() < min_occurrences {
            continue;
        }

        // Sort by date
        let mut sorted: Vec<&&Transaction> = txns.iter().collect();
        sorted.sort_by_key(|t| &t.date);

        // Compute intervals between consecutive transactions (in days)
        let intervals = compute_intervals(&sorted);
        let frequency = classify_frequency(&intervals);

        // Skip if truly irregular (no pattern)
        if frequency == Frequency::Irregular && intervals.len() > 3 {
            let cv = coefficient_of_variation(&intervals);
            if cv > 0.5 {
                continue;
            }
        }

        // Compute average amount (from first posting)
        let amounts: Vec<Decimal> = sorted
            .iter()
            .filter_map(|t| {
                t.postings
                    .first()?
                    .amount
                    .as_ref()
                    .map(|a| a.quantity.abs())
            })
            .collect();

        let avg = if amounts.is_empty() {
            Decimal::ZERO
        } else {
            amounts.iter().copied().sum::<Decimal>() / Decimal::from(amounts.len())
        };

        // Classify type
        let is_income = sorted.iter().any(|t| {
            t.postings.iter().any(|p| {
                let lower = p.account.to_lowercase();
                lower.starts_with("income") || lower.starts_with("revenue")
            })
        });

        let amount_variance = if amounts.len() > 1 {
            let mean = avg;
            let var: Decimal = amounts
                .iter()
                .map(|a| (*a - mean) * (*a - mean))
                .sum::<Decimal>()
                / Decimal::from(amounts.len());
            var
        } else {
            Decimal::ZERO
        };

        let recurring_type = if is_income {
            RecurringType::Income
        } else if amount_variance < Decimal::new(1, 0) {
            RecurringType::Subscription
        } else {
            RecurringType::Variable
        };

        let account = sorted
            .last()
            .and_then(|t| t.postings.first().map(|p| p.account.clone()))
            .unwrap_or_default();

        patterns.push(RecurringPattern {
            payee: payee.clone(),
            frequency,
            recurring_type,
            average_amount: avg,
            occurrence_count: sorted.len(),
            last_date: sorted.last().map_or(String::new(), |t| t.date.clone()),
            account,
        });
    }

    patterns
}

/// Check if a recurring pattern is missing its expected next occurrence.
pub fn check_missing(pattern: &RecurringPattern, current_date: &str) -> bool {
    let Ok(last) = chrono::NaiveDate::parse_from_str(&pattern.last_date, "%Y-%m-%d") else {
        return false;
    };
    let Ok(now) = chrono::NaiveDate::parse_from_str(current_date, "%Y-%m-%d") else {
        return false;
    };

    let days_since = (now - last).num_days();
    let expected_interval = match pattern.frequency {
        Frequency::Weekly => 7,
        Frequency::Biweekly => 14,
        Frequency::Monthly => 30,
        Frequency::Quarterly => 90,
        Frequency::Annual => 365,
        Frequency::Irregular => return false,
    };

    // Consider missing if 50% overdue
    days_since > expected_interval + expected_interval / 2
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn normalize_payee(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn compute_intervals(sorted: &[&&Transaction]) -> Vec<i64> {
    sorted
        .windows(2)
        .filter_map(|w| {
            let d1 = chrono::NaiveDate::parse_from_str(&w[0].date, "%Y-%m-%d").ok()?;
            let d2 = chrono::NaiveDate::parse_from_str(&w[1].date, "%Y-%m-%d").ok()?;
            Some((d2 - d1).num_days())
        })
        .collect()
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn classify_frequency(intervals: &[i64]) -> Frequency {
    if intervals.is_empty() {
        return Frequency::Irregular;
    }
    let avg = intervals.iter().sum::<i64>() as f64 / intervals.len() as f64;
    match avg as i64 {
        0..=10 => Frequency::Weekly,
        11..=21 => Frequency::Biweekly,
        22..=45 => Frequency::Monthly,
        46..=120 => Frequency::Quarterly,
        121..=500 => Frequency::Annual,
        _ => Frequency::Irregular,
    }
}

#[allow(clippy::cast_precision_loss)]
fn coefficient_of_variation(intervals: &[i64]) -> f64 {
    if intervals.is_empty() {
        return 0.0;
    }
    let mean = intervals.iter().sum::<i64>() as f64 / intervals.len() as f64;
    if mean == 0.0 {
        return 0.0;
    }
    let variance = intervals
        .iter()
        .map(|&i| (i as f64 - mean).powi(2))
        .sum::<f64>()
        / intervals.len() as f64;
    variance.sqrt() / mean
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::types::{Amount, Posting, Status};
    use std::collections::HashMap;

    fn make_txn(date: &str, desc: &str, account: &str, amount: &str) -> Transaction {
        Transaction {
            date: date.into(),
            status: Status::Cleared,
            code: None,
            description: desc.into(),
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
            tags: HashMap::new(),
            comment: None,
            source_line: 0,
        }
    }

    #[test]
    fn detect_monthly_subscription() {
        let txns = vec![
            make_txn("2024-01-15", "Netflix", "Expenses:Subscriptions", "15.99"),
            make_txn("2024-02-15", "Netflix", "Expenses:Subscriptions", "15.99"),
            make_txn("2024-03-15", "Netflix", "Expenses:Subscriptions", "15.99"),
            make_txn("2024-04-15", "Netflix", "Expenses:Subscriptions", "15.99"),
        ];
        let patterns = detect_recurring(&txns, 3);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].frequency, Frequency::Monthly);
        assert_eq!(patterns[0].recurring_type, RecurringType::Subscription);
    }

    #[test]
    fn detect_income() {
        let txns = vec![
            make_txn("2024-01-01", "Employer Inc", "Income:Salary", "-5000"),
            make_txn("2024-02-01", "Employer Inc", "Income:Salary", "-5000"),
            make_txn("2024-03-01", "Employer Inc", "Income:Salary", "-5000"),
        ];
        let patterns = detect_recurring(&txns, 3);
        assert_eq!(patterns[0].recurring_type, RecurringType::Income);
    }

    #[test]
    fn skip_infrequent() {
        let txns = vec![
            make_txn("2024-01-15", "OneTime Store", "Expenses:Other", "50"),
            make_txn("2024-06-15", "OneTime Store", "Expenses:Other", "50"),
        ];
        let patterns = detect_recurring(&txns, 3);
        assert!(patterns.is_empty());
    }

    #[test]
    fn missing_detection() {
        let pattern = RecurringPattern {
            payee: "netflix".into(),
            frequency: Frequency::Monthly,
            recurring_type: RecurringType::Subscription,
            average_amount: Decimal::new(1599, 2),
            occurrence_count: 4,
            last_date: "2024-04-15".into(),
            account: "Expenses:Subscriptions".into(),
        };
        // 60 days after last = missing (>45 day threshold for monthly)
        assert!(check_missing(&pattern, "2024-06-15"));
        // 20 days after = not missing
        assert!(!check_missing(&pattern, "2024-05-05"));
    }
}
