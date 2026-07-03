//! Apply a parsed hledger journal to the vault.
//!
//! Parsing lives in [`crate::accounting::parser`] and is pure (no storage). This
//! module takes an already-parsed [`Journal`] and materializes it into the
//! `SQLite` vault: it creates any accounts referenced by the journal (inferring
//! their type from the top-level name segment), then inserts each transaction
//! with its postings. Every write records a sync-outbox event, so an imported
//! journal syncs to other devices like any other change.
//!
//! Postings reference accounts by **id** (matching the rest of the storage
//! layer), so account names are resolved to ids as they are created.

use std::collections::HashMap;

use crate::accounting::types::{Amount, Journal, Status as JournalStatus};

use super::accounts::{self, AccountType, ListOptions, NewAccount};
use super::error::StorageError;
use super::sync::{self, SyncContext};
use super::transactions::{self, NewPosting, NewTransaction, TransactionStatus};

/// Counts describing what an import produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Accounts newly created because the journal referenced them.
    pub accounts_created: u32,
    /// Transactions inserted into the vault.
    pub transactions_imported: u32,
}

/// Apply a parsed [`Journal`] to the vault behind `conn`.
///
/// Missing accounts (referenced by postings or declared via `account`
/// directives) are created with a type inferred from their top-level name
/// segment. Transactions are inserted in journal order.
///
/// # Errors
///
/// Returns [`StorageError`] if an account or transaction fails to persist. The
/// import is applied incrementally, so a mid-stream failure may leave earlier
/// entries committed; callers that need all-or-nothing semantics should validate
/// the journal (e.g. via the parser) before calling.
pub fn import_journal(
    conn: &rusqlite::Connection,
    journal: &Journal,
) -> Result<ImportSummary, StorageError> {
    let mut summary = ImportSummary::default();

    // name -> id, seeded with the accounts already in the vault so we reuse them
    // instead of hitting the unique-name constraint.
    let mut ids: HashMap<String, String> = HashMap::new();
    for acct in accounts::list_accounts(conn, &ListOptions::default())? {
        ids.insert(acct.name, acct.id);
    }

    // Materialize declared accounts up front so declared-but-unused accounts
    // still appear after import.
    for decl in &journal.account_declarations {
        ensure_account(conn, &mut ids, &mut summary, &decl.name)?;
    }

    for txn in &journal.transactions {
        let mut postings = Vec::with_capacity(txn.postings.len());
        for p in &txn.postings {
            let account_id = ensure_account(conn, &mut ids, &mut summary, &p.account)?;
            let (amount_quantity, amount_commodity) = split_amount(p.amount.as_ref());
            let (balance_assertion_quantity, balance_assertion_commodity) =
                split_amount(p.balance_assertion.as_ref());
            postings.push(NewPosting {
                account_id,
                amount_quantity,
                amount_commodity,
                balance_assertion_quantity,
                balance_assertion_commodity,
            });
        }

        let input = NewTransaction {
            date: txn.date.clone(),
            status: to_storage_status(txn.status),
            code: txn.code.clone(),
            description: txn.description.clone(),
            comment: txn.comment.clone(),
            postings,
        };
        let ctx = next_context(conn)?;
        transactions::create_transaction_with_sync(conn, &input, &ctx)?;
        summary.transactions_imported += 1;
    }

    Ok(summary)
}

/// Resolve an account name to its id, creating the account if it does not exist.
fn ensure_account(
    conn: &rusqlite::Connection,
    ids: &mut HashMap<String, String>,
    summary: &mut ImportSummary,
    name: &str,
) -> Result<String, StorageError> {
    if let Some(id) = ids.get(name) {
        return Ok(id.clone());
    }

    let ctx = next_context(conn)?;
    let account = accounts::create_account_with_sync(
        conn,
        &NewAccount {
            name: name.to_string(),
            account_type: infer_account_type(name),
            commodity: None,
            parent_id: None,
            note: None,
        },
        &ctx,
    )?;

    ids.insert(name.to_string(), account.id.clone());
    summary.accounts_created += 1;
    Ok(account.id)
}

/// Build a [`SyncContext`] for the next write: the vault device id plus a freshly
/// ticked Lamport clock, mirroring the FFI layer's per-mutation context.
fn next_context(conn: &rusqlite::Connection) -> Result<SyncContext, StorageError> {
    Ok(SyncContext {
        device_id: sync::device_id(conn)?,
        lamport_clock: sync::tick_lamport(conn)?,
    })
}

/// Split an optional [`Amount`] into the `(quantity, commodity)` string pair the
/// storage layer expects. An empty commodity is stored as `None`.
fn split_amount(amount: Option<&Amount>) -> (Option<String>, Option<String>) {
    match amount {
        Some(a) => {
            let commodity = if a.commodity.is_empty() {
                None
            } else {
                Some(a.commodity.clone())
            };
            (Some(a.quantity.to_string()), commodity)
        }
        None => (None, None),
    }
}

fn to_storage_status(status: JournalStatus) -> TransactionStatus {
    match status {
        JournalStatus::Unmarked => TransactionStatus::Unmarked,
        JournalStatus::Pending => TransactionStatus::Pending,
        JournalStatus::Cleared => TransactionStatus::Cleared,
    }
}

/// Infer an [`AccountType`] from the top-level segment of an hledger account
/// name (e.g. `Assets:Checking` -> [`AccountType::Asset`]). Unknown roots
/// default to [`AccountType::Asset`].
fn infer_account_type(name: &str) -> AccountType {
    let root = name.split(':').next().unwrap_or(name).trim();
    match root.to_lowercase().as_str() {
        "liabilities" | "liability" => AccountType::Liability,
        "income" | "revenue" | "revenues" => AccountType::Income,
        "expenses" | "expense" => AccountType::Expense,
        "equity" => AccountType::Equity,
        // "assets"/"asset" and anything unrecognized fall back to Asset.
        _ => AccountType::Asset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::parser::parse_journal;
    use crate::storage::schema;

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::initialize(&conn).unwrap();
        conn
    }

    #[test]
    fn infers_account_types_from_root() {
        assert_eq!(
            infer_account_type("Assets:Bank:Checking"),
            AccountType::Asset
        );
        assert_eq!(
            infer_account_type("Liabilities:Card"),
            AccountType::Liability
        );
        assert_eq!(infer_account_type("Income:Salary"), AccountType::Income);
        assert_eq!(infer_account_type("Expenses:Food"), AccountType::Expense);
        assert_eq!(infer_account_type("Equity:Opening"), AccountType::Equity);
        assert_eq!(infer_account_type("Weird:Root"), AccountType::Asset);
    }

    #[test]
    fn imports_transactions_and_creates_accounts() {
        let conn = test_conn();
        let journal = parse_journal(
            "2024-01-01 Opening\n    Assets:Checking   $100.00\n    Equity:Opening\n\n\
             2024-01-02 * Coffee\n    Expenses:Food    $4.50\n    Assets:Checking\n",
        )
        .unwrap();

        let summary = import_journal(&conn, &journal).unwrap();
        assert_eq!(summary.transactions_imported, 2);
        // Assets:Checking, Equity:Opening, Expenses:Food = 3 unique accounts.
        assert_eq!(summary.accounts_created, 3);

        let accts = accounts::list_accounts(&conn, &ListOptions::default()).unwrap();
        assert_eq!(accts.len(), 3);

        let txns = transactions::list_transactions(&conn, &ListOptions::default()).unwrap();
        assert_eq!(txns.len(), 2);

        // Postings reference account ids, not names.
        let checking = accts.iter().find(|a| a.name == "Assets:Checking").unwrap();
        let opening = txns.iter().find(|t| t.description == "Opening").unwrap();
        assert!(opening.postings.iter().any(|p| p.account_id == checking.id));
    }

    #[test]
    fn reuses_existing_accounts() {
        let conn = test_conn();
        // Pre-create one of the accounts the journal references.
        let ctx = next_context(&conn).unwrap();
        accounts::create_account_with_sync(
            &conn,
            &NewAccount {
                name: "Assets:Checking".to_string(),
                account_type: AccountType::Asset,
                commodity: None,
                parent_id: None,
                note: None,
            },
            &ctx,
        )
        .unwrap();

        let journal =
            parse_journal("2024-01-02 Coffee\n    Expenses:Food    $4.50\n    Assets:Checking\n")
                .unwrap();

        let summary = import_journal(&conn, &journal).unwrap();
        // Only Expenses:Food is new; Assets:Checking is reused.
        assert_eq!(summary.accounts_created, 1);
        assert_eq!(summary.transactions_imported, 1);
        assert_eq!(
            accounts::list_accounts(&conn, &ListOptions::default())
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn creates_declared_but_unused_accounts() {
        let conn = test_conn();
        let journal = parse_journal("account Assets:Savings\n").unwrap();
        let summary = import_journal(&conn, &journal).unwrap();
        assert_eq!(summary.accounts_created, 1);
        assert_eq!(summary.transactions_imported, 0);
    }
}
