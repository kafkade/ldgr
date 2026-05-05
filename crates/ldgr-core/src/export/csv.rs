//! Export transactions to CSV format.

use std::fmt::Write;

use crate::accounting::types::Transaction;

/// Export transactions as CSV with header row.
pub fn to_csv(transactions: &[Transaction]) -> String {
    let mut out = String::from("date,status,code,description,account,amount,commodity\n");

    for txn in transactions {
        let status = match txn.status {
            crate::accounting::types::Status::Cleared => "cleared",
            crate::accounting::types::Status::Pending => "pending",
            crate::accounting::types::Status::Unmarked => "unmarked",
        };
        let code = txn.code.as_deref().unwrap_or("");

        for posting in &txn.postings {
            let (amount, commodity) = match &posting.amount {
                Some(a) => (a.quantity.to_string(), a.commodity.as_str()),
                None => (String::new(), ""),
            };
            let _ = writeln!(
                out,
                "{},{},{},{},{},{},{}",
                txn.date,
                status,
                esc(code),
                esc(&txn.description),
                esc(&posting.account),
                amount,
                commodity,
            );
        }
    }
    out
}

fn esc(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::types::{Amount, Posting, Status};
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    #[test]
    fn csv_header() {
        let out = to_csv(&[]);
        assert!(out.starts_with("date,status,code,description,account,amount,commodity\n"));
    }

    #[test]
    fn csv_row() {
        let txn = Transaction {
            date: "2024-01-15".into(),
            status: Status::Cleared,
            code: None,
            description: "Groceries".into(),
            postings: vec![Posting {
                account: "Expenses:Food".into(),
                amount: Some(Amount {
                    quantity: Decimal::new(4250, 2),
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
        };
        let out = to_csv(&[txn]);
        assert!(out.contains("2024-01-15,cleared,,Groceries,Expenses:Food,42.50,USD"));
    }

    #[test]
    fn csv_escapes() {
        assert_eq!(esc("hello, world"), "\"hello, world\"");
        assert_eq!(esc("plain"), "plain");
    }
}
