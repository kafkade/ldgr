//! Export transactions to JSON format.

use crate::accounting::types::Transaction;

/// Export transactions as a JSON array string.
pub fn to_json(transactions: &[Transaction]) -> String {
    let entries: Vec<serde_json::Value> = transactions
        .iter()
        .map(|txn| {
            let postings: Vec<serde_json::Value> = txn
                .postings
                .iter()
                .map(|p| {
                    let mut posting = serde_json::json!({
                        "account": p.account,
                    });
                    if let Some(amt) = &p.amount {
                        posting["amount"] = serde_json::json!({
                            "quantity": amt.quantity.to_string(),
                            "commodity": amt.commodity,
                        });
                    }
                    if let Some(assertion) = &p.balance_assertion {
                        posting["balance_assertion"] = serde_json::json!({
                            "quantity": assertion.quantity.to_string(),
                            "commodity": assertion.commodity,
                        });
                    }
                    if let Some(comment) = &p.comment {
                        posting["comment"] = serde_json::Value::String(comment.clone());
                    }
                    posting
                })
                .collect();

            let mut entry = serde_json::json!({
                "date": txn.date,
                "status": format!("{:?}", txn.status).to_lowercase(),
                "description": txn.description,
                "postings": postings,
            });

            if let Some(code) = &txn.code {
                entry["code"] = serde_json::Value::String(code.clone());
            }
            if let Some(comment) = &txn.comment {
                entry["comment"] = serde_json::Value::String(comment.clone());
            }
            if !txn.tags.is_empty() {
                let tags: serde_json::Map<String, serde_json::Value> = txn
                    .tags
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                entry["tags"] = serde_json::Value::Object(tags);
            }

            entry
        })
        .collect();

    serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::types::{Amount, Posting, Status};
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    #[test]
    fn json_valid_output() {
        let txn = Transaction {
            date: "2024-01-15".into(),
            status: Status::Cleared,
            code: Some("1001".into()),
            description: "Test".into(),
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

        let output = to_json(&[txn]);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 1);
        assert_eq!(parsed[0]["date"], "2024-01-15");
        assert_eq!(parsed[0]["code"], "1001");
        assert_eq!(parsed[0]["postings"][0]["amount"]["quantity"], "42.50");
    }

    #[test]
    fn json_empty_list() {
        let output = to_json(&[]);
        assert_eq!(output.trim(), "[]");
    }
}
