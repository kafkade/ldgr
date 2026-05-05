//! Export to hledger journal format.

use std::fmt::Write;

use crate::accounting::types::{Amount, Status, Transaction};

/// Format transactions as an hledger journal string.
pub fn to_hledger(transactions: &[Transaction]) -> String {
    let mut out = String::new();
    out.push_str("; Exported from ldgr\n\n");

    for txn in transactions {
        out.push_str(&txn.date);
        match txn.status {
            Status::Cleared => out.push_str(" *"),
            Status::Pending => out.push_str(" !"),
            Status::Unmarked => {}
        }
        if let Some(code) = &txn.code {
            let _ = write!(out, " ({code})");
        }
        out.push(' ');
        out.push_str(&txn.description);
        if let Some(comment) = &txn.comment {
            let _ = write!(out, "  ; {comment}");
        }
        out.push('\n');

        for posting in &txn.postings {
            out.push_str("    ");
            out.push_str(&posting.account);
            if let Some(amt) = &posting.amount {
                let pad = 40_usize.saturating_sub(posting.account.len()).max(2);
                let _ = write!(out, "{}{}", " ".repeat(pad), fmt_amt(amt));
            }
            if let Some(a) = &posting.balance_assertion {
                let _ = write!(out, " = {}", fmt_amt(a));
            }
            if let Some(c) = &posting.comment {
                let _ = write!(out, "  ; {c}");
            }
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

fn fmt_amt(a: &Amount) -> String {
    if a.commodity.is_empty() {
        a.quantity.to_string()
    } else {
        format!("{} {}", a.quantity, a.commodity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::types::Posting;
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    fn sample() -> Transaction {
        Transaction {
            date: "2024-01-15".into(),
            status: Status::Cleared,
            code: Some("1001".into()),
            description: "Whole Foods".into(),
            postings: vec![
                Posting {
                    account: "Expenses:Food".into(),
                    amount: Some(Amount {
                        quantity: Decimal::new(4250, 2),
                        commodity: "USD".into(),
                    }),
                    balance_assertion: None,
                    status: Status::Unmarked,
                    comment: None,
                    tags: HashMap::new(),
                },
                Posting {
                    account: "Assets:Checking".into(),
                    amount: Some(Amount {
                        quantity: Decimal::new(-4250, 2),
                        commodity: "USD".into(),
                    }),
                    balance_assertion: None,
                    status: Status::Unmarked,
                    comment: None,
                    tags: HashMap::new(),
                },
            ],
            tags: HashMap::new(),
            comment: None,
            source_line: 0,
        }
    }

    #[test]
    fn basic_format() {
        let out = to_hledger(&[sample()]);
        assert!(out.contains("2024-01-15 * (1001) Whole Foods"));
        assert!(out.contains("42.50 USD"));
    }

    #[test]
    fn round_trip_parseable() {
        let out = to_hledger(&[sample()]);
        let parsed = crate::accounting::parser::parse_journal(&out).unwrap();
        assert_eq!(parsed.transactions.len(), 1);
    }
}
