//! Convert storage types to accounting types for report computation.

use ldgr_core::accounting::types as acct;
use ldgr_core::storage::transactions as store;

/// Convert a storage transaction to an accounting transaction.
pub fn to_accounting_txn(txn: &store::Transaction) -> acct::Transaction {
    acct::Transaction {
        date: txn.date.clone(),
        status: match txn.status {
            store::TransactionStatus::Unmarked => acct::Status::Unmarked,
            store::TransactionStatus::Pending => acct::Status::Pending,
            store::TransactionStatus::Cleared => acct::Status::Cleared,
        },
        code: txn.code.clone(),
        description: txn.description.clone(),
        comment: txn.comment.clone(),
        tags: std::collections::HashMap::new(),
        source_line: 0,
        postings: txn.postings.iter().map(to_accounting_posting).collect(),
    }
}

fn to_accounting_posting(p: &store::Posting) -> acct::Posting {
    let amount = match (&p.amount_quantity, &p.amount_commodity) {
        (Some(qty), commodity) => qty.parse().ok().map(|q| acct::Amount {
            quantity: q,
            commodity: commodity.as_deref().unwrap_or("").to_string(),
        }),
        _ => None,
    };

    let balance_assertion = match (
        &p.balance_assertion_quantity,
        &p.balance_assertion_commodity,
    ) {
        (Some(qty), commodity) => qty.parse().ok().map(|q| acct::Amount {
            quantity: q,
            commodity: commodity.as_deref().unwrap_or("").to_string(),
        }),
        _ => None,
    };

    acct::Posting {
        account: p.account_id.clone(),
        amount,
        balance_assertion,
        status: acct::Status::Unmarked,
        comment: None,
        tags: std::collections::HashMap::new(),
    }
}

/// Convert a list of storage transactions to accounting transactions.
pub fn to_accounting_txns(txns: &[store::Transaction]) -> Vec<acct::Transaction> {
    txns.iter().map(to_accounting_txn).collect()
}
