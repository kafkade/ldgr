//! Transaction and posting CRUD operations.
//!
//! Transactions are the atomic unit of double-entry accounting. Each transaction
//! contains two or more postings that must balance (enforced by the accounting
//! module, not at the storage layer).
//!
//! Creating or updating a transaction is atomic: the transaction row and all its
//! postings are written in a single `SQLite` transaction.

use rusqlite::Connection;
use uuid::Uuid;

use super::accounts::ListOptions;
use super::error::StorageError;
use super::sync::SyncContext;

// ── Types ──────────────────────────────────────────────────────────────────────

/// Transaction status (cleared, pending, or unmarked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStatus {
    Unmarked,
    Pending,
    Cleared,
}

impl TransactionStatus {
    /// Convert to the string stored in `SQLite`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unmarked => "unmarked",
            Self::Pending => "pending",
            Self::Cleared => "cleared",
        }
    }

    fn from_str(s: &str) -> Result<Self, StorageError> {
        match s {
            "unmarked" => Ok(Self::Unmarked),
            "pending" => Ok(Self::Pending),
            "cleared" => Ok(Self::Cleared),
            _ => Err(StorageError::InvalidInput(format!(
                "unknown transaction status: {s}"
            ))),
        }
    }
}

/// A persisted transaction with its postings.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub id: String,
    pub date: String,
    pub status: TransactionStatus,
    pub code: Option<String>,
    pub description: String,
    pub comment: Option<String>,
    pub created_at: String,
    pub modified_at: String,
    pub version: i64,
    pub deleted: bool,
    pub postings: Vec<Posting>,
}

/// A persisted posting row.
#[derive(Debug, Clone)]
pub struct Posting {
    pub id: String,
    pub transaction_id: String,
    pub account_id: String,
    pub amount_quantity: Option<String>,
    pub amount_commodity: Option<String>,
    pub balance_assertion_quantity: Option<String>,
    pub balance_assertion_commodity: Option<String>,
    pub posting_order: i64,
    pub created_at: String,
    pub version: i64,
}

/// Input for creating a new transaction.
#[derive(Debug, Clone)]
pub struct NewTransaction {
    pub date: String,
    pub status: TransactionStatus,
    pub code: Option<String>,
    pub description: String,
    pub comment: Option<String>,
    pub postings: Vec<NewPosting>,
}

/// Input for a new posting within a transaction.
#[derive(Debug, Clone)]
pub struct NewPosting {
    pub account_id: String,
    pub amount_quantity: Option<String>,
    pub amount_commodity: Option<String>,
    pub balance_assertion_quantity: Option<String>,
    pub balance_assertion_commodity: Option<String>,
}

/// Input for updating an existing transaction (full replacement).
#[derive(Debug, Clone)]
pub struct TransactionUpdate {
    pub date: String,
    pub status: TransactionStatus,
    pub code: Option<String>,
    pub description: String,
    pub comment: Option<String>,
    pub postings: Vec<NewPosting>,
    /// Must match the current version or the update is rejected.
    pub expected_version: i64,
}

// ── CRUD operations ────────────────────────────────────────────────────────────

/// Create a new transaction with its postings atomically.
///
/// # Errors
///
/// Returns [`StorageError::InvalidInput`] if the postings list is empty.
pub fn create_transaction(
    conn: &Connection,
    input: &NewTransaction,
) -> Result<Transaction, StorageError> {
    if input.postings.is_empty() {
        return Err(StorageError::InvalidInput(
            "transaction must have at least one posting".into(),
        ));
    }

    let txn_id = Uuid::now_v7().to_string();
    let now = now_iso8601();

    let db_tx = conn.unchecked_transaction()?;

    db_tx.execute(
        "INSERT INTO transactions (id, date, status, code, description, comment, created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            txn_id,
            input.date,
            input.status.as_str(),
            input.code,
            input.description,
            input.comment,
            now,
            now,
        ],
    )?;

    let postings = insert_postings(&db_tx, &txn_id, &input.postings, &now)?;

    db_tx.commit()?;

    Ok(Transaction {
        id: txn_id,
        date: input.date.clone(),
        status: input.status,
        code: input.code.clone(),
        description: input.description.clone(),
        comment: input.comment.clone(),
        created_at: now.clone(),
        modified_at: now,
        version: 1,
        deleted: false,
        postings,
    })
}

/// Get a transaction by ID, including its postings.
///
/// Postings reference accounts that may be soft-deleted (historical data).
pub fn get_transaction(
    conn: &Connection,
    id: &str,
    opts: &ListOptions,
) -> Result<Option<Transaction>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, date, status, code, description, comment, created_at, modified_at, version, deleted
         FROM transactions WHERE id = ?1"
    } else {
        "SELECT id, date, status, code, description, comment, created_at, modified_at, version, deleted
         FROM transactions WHERE id = ?1 AND deleted = 0"
    };

    let mut stmt = conn.prepare(sql)?;
    let txn = match stmt.query_row([id], row_to_transaction_header) {
        Ok(t) => t,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let postings = fetch_postings(conn, &txn.id)?;
    Ok(Some(Transaction { postings, ..txn }))
}

/// List all transactions with their postings.
pub fn list_transactions(
    conn: &Connection,
    opts: &ListOptions,
) -> Result<Vec<Transaction>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, date, status, code, description, comment, created_at, modified_at, version, deleted
         FROM transactions ORDER BY date DESC, created_at DESC"
    } else {
        "SELECT id, date, status, code, description, comment, created_at, modified_at, version, deleted
         FROM transactions WHERE deleted = 0 ORDER BY date DESC, created_at DESC"
    };

    let mut stmt = conn.prepare(sql)?;
    let headers: Vec<Transaction> = stmt
        .query_map([], row_to_transaction_header)?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(headers.len());
    for txn in headers {
        let postings = fetch_postings(conn, &txn.id)?;
        result.push(Transaction { postings, ..txn });
    }

    Ok(result)
}

/// Update a transaction and replace all its postings atomically.
///
/// Uses optimistic concurrency via `expected_version`.
///
/// # Errors
///
/// Returns [`StorageError::Conflict`] on version mismatch,
/// [`StorageError::InvalidInput`] if postings list is empty.
pub fn update_transaction(
    conn: &Connection,
    id: &str,
    update: &TransactionUpdate,
) -> Result<Transaction, StorageError> {
    if update.postings.is_empty() {
        return Err(StorageError::InvalidInput(
            "transaction must have at least one posting".into(),
        ));
    }

    let now = now_iso8601();
    let new_version = update.expected_version + 1;

    let db_tx = conn.unchecked_transaction()?;

    let rows = db_tx.execute(
        "UPDATE transactions
         SET date = ?1, status = ?2, code = ?3, description = ?4, comment = ?5,
             modified_at = ?6, version = ?7
         WHERE id = ?8 AND version = ?9 AND deleted = 0",
        rusqlite::params![
            update.date,
            update.status.as_str(),
            update.code,
            update.description,
            update.comment,
            now,
            new_version,
            id,
            update.expected_version,
        ],
    )?;

    if rows == 0 {
        db_tx.rollback()?;
        return Err(StorageError::Conflict(format!(
            "transaction '{id}' was modified or deleted (expected version {})",
            update.expected_version
        )));
    }

    // Replace postings: delete old, insert new
    db_tx.execute("DELETE FROM postings WHERE transaction_id = ?1", [id])?;
    let postings = insert_postings(&db_tx, id, &update.postings, &now)?;

    db_tx.commit()?;

    Ok(Transaction {
        id: id.to_string(),
        date: update.date.clone(),
        status: update.status,
        code: update.code.clone(),
        description: update.description.clone(),
        comment: update.comment.clone(),
        created_at: String::new(),
        modified_at: now,
        version: new_version,
        deleted: false,
        postings,
    })
}

/// Soft-delete a transaction by setting `deleted = 1`.
///
/// Postings are preserved (they reference the soft-deleted transaction).
pub fn soft_delete_transaction(conn: &Connection, id: &str) -> Result<(), StorageError> {
    let rows = conn.execute(
        "UPDATE transactions SET deleted = 1, version = version + 1, modified_at = ?1
         WHERE id = ?2 AND deleted = 0",
        rusqlite::params![now_iso8601(), id],
    )?;

    if rows == 0 {
        return Err(StorageError::NotFound(format!("transaction '{id}'")));
    }
    Ok(())
}

// ── Sync-aware variants ────────────────────────────────────────────────────────

/// Create a new transaction with postings and atomic sync event recording.
pub fn create_transaction_with_sync(
    conn: &Connection,
    input: &NewTransaction,
    ctx: &SyncContext,
) -> Result<Transaction, StorageError> {
    if input.postings.is_empty() {
        return Err(StorageError::InvalidInput(
            "transaction must have at least one posting".into(),
        ));
    }

    let txn_id = Uuid::now_v7().to_string();
    let now = now_iso8601();

    let db_tx = conn.unchecked_transaction()?;

    db_tx.execute(
        "INSERT INTO transactions (id, date, status, code, description, comment, created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            txn_id,
            input.date,
            input.status.as_str(),
            input.code,
            input.description,
            input.comment,
            now,
            now,
        ],
    )?;

    let postings = insert_postings(&db_tx, &txn_id, &input.postings, &now)?;

    let payload = serde_json::json!({
        "id": txn_id,
        "date": input.date,
        "description": input.description,
        "status": input.status.as_str(),
        "postings": input.postings.iter().map(|p| serde_json::json!({
            "account_id": p.account_id,
            "amount_quantity": p.amount_quantity,
            "amount_commodity": p.amount_commodity,
        })).collect::<Vec<_>>(),
    })
    .to_string()
    .into_bytes();

    super::sync::record_event(
        &db_tx,
        &ctx.device_id,
        "transaction",
        &txn_id,
        "create",
        &payload,
        ctx.lamport_clock,
        1,
    )?;

    db_tx.commit()?;

    Ok(Transaction {
        id: txn_id,
        date: input.date.clone(),
        status: input.status,
        code: input.code.clone(),
        description: input.description.clone(),
        comment: input.comment.clone(),
        created_at: now.clone(),
        modified_at: now,
        version: 1,
        deleted: false,
        postings,
    })
}

/// Soft-delete a transaction with atomic sync event recording.
pub fn soft_delete_transaction_with_sync(
    conn: &Connection,
    id: &str,
    ctx: &SyncContext,
) -> Result<(), StorageError> {
    let db_tx = conn.unchecked_transaction()?;

    let rows = db_tx.execute(
        "UPDATE transactions SET deleted = 1, version = version + 1, modified_at = ?1
         WHERE id = ?2 AND deleted = 0",
        rusqlite::params![now_iso8601(), id],
    )?;

    if rows == 0 {
        return Err(StorageError::NotFound(format!("transaction '{id}'")));
    }

    let payload = serde_json::json!({ "id": id }).to_string().into_bytes();

    super::sync::record_event(
        &db_tx,
        &ctx.device_id,
        "transaction",
        id,
        "delete",
        &payload,
        ctx.lamport_clock,
        1,
    )?;

    db_tx.commit()?;
    Ok(())
}

/// Update just the status of a transaction (e.g., mark as cleared during reconciliation).
pub fn set_transaction_status(
    conn: &Connection,
    id: &str,
    status: TransactionStatus,
) -> Result<(), StorageError> {
    let rows = conn.execute(
        "UPDATE transactions SET status = ?1, version = version + 1, modified_at = ?2
         WHERE id = ?3 AND deleted = 0",
        rusqlite::params![status.as_str(), now_iso8601(), id],
    )?;

    if rows == 0 {
        return Err(StorageError::NotFound(format!("transaction '{id}'")));
    }
    Ok(())
}

/// List transactions that have postings referencing a specific account.
///
/// Ordered by date ascending. Filters by status if provided.
pub fn list_transactions_for_account(
    conn: &Connection,
    account_id: &str,
    status_filter: Option<TransactionStatus>,
    before_date: Option<&str>,
) -> Result<Vec<Transaction>, StorageError> {
    let mut sql = String::from(
        "SELECT DISTINCT t.id, t.date, t.status, t.code, t.description, t.comment,
                t.created_at, t.modified_at, t.version, t.deleted
         FROM transactions t
         JOIN postings p ON t.id = p.transaction_id
         WHERE t.deleted = 0 AND p.account_id = ?1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(account_id.to_string())];

    if let Some(status) = status_filter {
        sql.push_str(" AND t.status = ?2");
        params.push(Box::new(status.as_str().to_string()));

        if let Some(date) = before_date {
            sql.push_str(" AND t.date <= ?3");
            params.push(Box::new(date.to_string()));
        }
    } else if let Some(date) = before_date {
        sql.push_str(" AND t.date <= ?2");
        params.push(Box::new(date.to_string()));
    }

    sql.push_str(" ORDER BY t.date ASC, t.created_at ASC");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(std::convert::AsRef::as_ref).collect();
    let mut stmt = conn.prepare(&sql)?;
    let headers: Vec<Transaction> = stmt
        .query_map(param_refs.as_slice(), row_to_transaction_header)?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(headers.len());
    for txn in headers {
        let postings = fetch_postings(conn, &txn.id)?;
        result.push(Transaction { postings, ..txn });
    }

    Ok(result)
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Insert postings for a transaction, assigning `posting_order` by list position.
fn insert_postings(
    conn: &Connection,
    transaction_id: &str,
    postings: &[NewPosting],
    now: &str,
) -> Result<Vec<Posting>, StorageError> {
    let mut result = Vec::with_capacity(postings.len());

    for (i, p) in postings.iter().enumerate() {
        let posting_id = Uuid::now_v7().to_string();
        let order =
            i64::try_from(i).map_err(|_| StorageError::InvalidInput("too many postings".into()))?;

        conn.execute(
            "INSERT INTO postings (id, transaction_id, account_id, amount_quantity, amount_commodity,
             balance_assertion_quantity, balance_assertion_commodity, posting_order, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                posting_id,
                transaction_id,
                p.account_id,
                p.amount_quantity,
                p.amount_commodity,
                p.balance_assertion_quantity,
                p.balance_assertion_commodity,
                order,
                now,
            ],
        )?;

        result.push(Posting {
            id: posting_id,
            transaction_id: transaction_id.to_string(),
            account_id: p.account_id.clone(),
            amount_quantity: p.amount_quantity.clone(),
            amount_commodity: p.amount_commodity.clone(),
            balance_assertion_quantity: p.balance_assertion_quantity.clone(),
            balance_assertion_commodity: p.balance_assertion_commodity.clone(),
            posting_order: order,
            created_at: now.to_string(),
            version: 1,
        });
    }

    Ok(result)
}

/// Fetch all postings for a transaction, ordered by `posting_order`.
fn fetch_postings(conn: &Connection, transaction_id: &str) -> Result<Vec<Posting>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, transaction_id, account_id, amount_quantity, amount_commodity,
                balance_assertion_quantity, balance_assertion_commodity,
                posting_order, created_at, version
         FROM postings
         WHERE transaction_id = ?1
         ORDER BY posting_order",
    )?;

    let rows = stmt
        .query_map([transaction_id], |row| {
            Ok(Posting {
                id: row.get(0)?,
                transaction_id: row.get(1)?,
                account_id: row.get(2)?,
                amount_quantity: row.get(3)?,
                amount_commodity: row.get(4)?,
                balance_assertion_quantity: row.get(5)?,
                balance_assertion_commodity: row.get(6)?,
                posting_order: row.get(7)?,
                created_at: row.get(8)?,
                version: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

fn row_to_transaction_header(row: &rusqlite::Row<'_>) -> rusqlite::Result<Transaction> {
    let status_str: String = row.get(2)?;
    let status =
        TransactionStatus::from_str(&status_str).map_err(|_e| rusqlite::Error::InvalidQuery)?;

    Ok(Transaction {
        id: row.get(0)?,
        date: row.get(1)?,
        status,
        code: row.get(3)?,
        description: row.get(4)?,
        comment: row.get(5)?,
        created_at: row.get(6)?,
        modified_at: row.get(7)?,
        version: row.get(8)?,
        deleted: row.get::<_, i64>(9)? != 0,
        postings: Vec::new(), // filled in by caller
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::accounts::{self, AccountType, NewAccount};
    use crate::storage::schema::initialize;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        conn
    }

    fn create_test_accounts(conn: &Connection) -> (String, String) {
        let a1 = accounts::create_account(
            conn,
            &NewAccount {
                name: "Assets:Checking".into(),
                account_type: AccountType::Asset,
                commodity: None,
                parent_id: None,
                note: None,
            },
        )
        .unwrap();
        let a2 = accounts::create_account(
            conn,
            &NewAccount {
                name: "Expenses:Food".into(),
                account_type: AccountType::Expense,
                commodity: None,
                parent_id: None,
                note: None,
            },
        )
        .unwrap();
        (a1.id, a2.id)
    }

    fn sample_transaction(checking_id: &str, food_id: &str) -> NewTransaction {
        NewTransaction {
            date: "2024-01-15".into(),
            status: TransactionStatus::Cleared,
            code: Some("1001".into()),
            description: "Whole Foods".into(),
            comment: Some("Weekly groceries".into()),
            postings: vec![
                NewPosting {
                    account_id: food_id.into(),
                    amount_quantity: Some("42.50".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                },
                NewPosting {
                    account_id: checking_id.into(),
                    amount_quantity: Some("-42.50".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                },
            ],
        }
    }

    // --- Create ---

    #[test]
    fn create_transaction_with_postings() {
        let conn = setup();
        let (checking, food) = create_test_accounts(&conn);

        let txn = create_transaction(&conn, &sample_transaction(&checking, &food)).unwrap();

        assert!(!txn.id.is_empty());
        assert_eq!(txn.date, "2024-01-15");
        assert_eq!(txn.status, TransactionStatus::Cleared);
        assert_eq!(txn.description, "Whole Foods");
        assert_eq!(txn.postings.len(), 2);
        assert_eq!(txn.postings[0].posting_order, 0);
        assert_eq!(txn.postings[1].posting_order, 1);
        assert_eq!(txn.version, 1);
    }

    #[test]
    fn create_transaction_rejects_empty_postings() {
        let conn = setup();
        let result = create_transaction(
            &conn,
            &NewTransaction {
                date: "2024-01-15".into(),
                status: TransactionStatus::Unmarked,
                code: None,
                description: "No postings".into(),
                comment: None,
                postings: vec![],
            },
        );
        assert!(matches!(result, Err(StorageError::InvalidInput(_))));
    }

    // --- Get ---

    #[test]
    fn get_transaction_with_postings() {
        let conn = setup();
        let (checking, food) = create_test_accounts(&conn);
        let created = create_transaction(&conn, &sample_transaction(&checking, &food)).unwrap();

        let fetched = get_transaction(&conn, &created.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.postings.len(), 2);
        assert_eq!(
            fetched.postings[0].amount_quantity.as_deref(),
            Some("42.50")
        );
    }

    #[test]
    fn get_nonexistent_transaction_returns_none() {
        let conn = setup();
        assert!(
            get_transaction(&conn, "no-such-id", &ListOptions::default())
                .unwrap()
                .is_none()
        );
    }

    // --- List ---

    #[test]
    fn list_transactions_ordered_by_date_desc() {
        let conn = setup();
        let (checking, food) = create_test_accounts(&conn);

        create_transaction(
            &conn,
            &NewTransaction {
                date: "2024-01-10".into(),
                status: TransactionStatus::Cleared,
                code: None,
                description: "Earlier".into(),
                comment: None,
                postings: vec![NewPosting {
                    account_id: checking.clone(),
                    amount_quantity: Some("10".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                }],
            },
        )
        .unwrap();

        create_transaction(
            &conn,
            &NewTransaction {
                date: "2024-01-20".into(),
                status: TransactionStatus::Cleared,
                code: None,
                description: "Later".into(),
                comment: None,
                postings: vec![NewPosting {
                    account_id: food.clone(),
                    amount_quantity: Some("20".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                }],
            },
        )
        .unwrap();

        let txns = list_transactions(&conn, &ListOptions::default()).unwrap();
        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].description, "Later"); // newer first
        assert_eq!(txns[1].description, "Earlier");
    }

    // --- Update ---

    #[test]
    fn update_transaction_replaces_postings() {
        let conn = setup();
        let (checking, food) = create_test_accounts(&conn);
        let created = create_transaction(&conn, &sample_transaction(&checking, &food)).unwrap();

        let updated = update_transaction(
            &conn,
            &created.id,
            &TransactionUpdate {
                date: "2024-01-16".into(),
                status: TransactionStatus::Pending,
                code: None,
                description: "Updated Description".into(),
                comment: None,
                postings: vec![NewPosting {
                    account_id: checking.clone(),
                    amount_quantity: Some("100.00".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                }],
                expected_version: 1,
            },
        )
        .unwrap();

        assert_eq!(updated.date, "2024-01-16");
        assert_eq!(updated.description, "Updated Description");
        assert_eq!(updated.status, TransactionStatus::Pending);
        assert_eq!(updated.version, 2);
        assert_eq!(updated.postings.len(), 1);
        assert_eq!(
            updated.postings[0].amount_quantity.as_deref(),
            Some("100.00")
        );
    }

    #[test]
    fn update_with_wrong_version_fails() {
        let conn = setup();
        let (checking, food) = create_test_accounts(&conn);
        let created = create_transaction(&conn, &sample_transaction(&checking, &food)).unwrap();

        let result = update_transaction(
            &conn,
            &created.id,
            &TransactionUpdate {
                date: "2024-01-16".into(),
                status: TransactionStatus::Unmarked,
                code: None,
                description: "Should fail".into(),
                comment: None,
                postings: vec![NewPosting {
                    account_id: checking,
                    amount_quantity: Some("1".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                }],
                expected_version: 99,
            },
        );
        assert!(matches!(result, Err(StorageError::Conflict(_))));
    }

    #[test]
    fn update_rejects_empty_postings() {
        let conn = setup();
        let (checking, food) = create_test_accounts(&conn);
        let created = create_transaction(&conn, &sample_transaction(&checking, &food)).unwrap();

        let result = update_transaction(
            &conn,
            &created.id,
            &TransactionUpdate {
                date: "2024-01-16".into(),
                status: TransactionStatus::Unmarked,
                code: None,
                description: "No postings".into(),
                comment: None,
                postings: vec![],
                expected_version: 1,
            },
        );
        assert!(matches!(result, Err(StorageError::InvalidInput(_))));
    }

    // --- Soft delete ---

    #[test]
    fn soft_delete_preserves_transaction() {
        let conn = setup();
        let (checking, food) = create_test_accounts(&conn);
        let txn = create_transaction(&conn, &sample_transaction(&checking, &food)).unwrap();

        soft_delete_transaction(&conn, &txn.id).unwrap();

        // Default query excludes deleted
        assert!(
            get_transaction(&conn, &txn.id, &ListOptions::default())
                .unwrap()
                .is_none()
        );

        // include_deleted finds it
        let opts = ListOptions {
            include_deleted: true,
        };
        let found = get_transaction(&conn, &txn.id, &opts).unwrap().unwrap();
        assert!(found.deleted);
        assert_eq!(found.postings.len(), 2); // postings preserved
    }

    #[test]
    fn soft_delete_nonexistent_fails() {
        let conn = setup();
        assert!(matches!(
            soft_delete_transaction(&conn, "no-such-id"),
            Err(StorageError::NotFound(_))
        ));
    }

    // --- All statuses ---

    #[test]
    fn all_transaction_statuses_round_trip() {
        let conn = setup();
        let (checking, _food) = create_test_accounts(&conn);

        for status in [
            TransactionStatus::Unmarked,
            TransactionStatus::Pending,
            TransactionStatus::Cleared,
        ] {
            let txn = create_transaction(
                &conn,
                &NewTransaction {
                    date: "2024-06-01".into(),
                    status,
                    code: None,
                    description: format!("{status:?} test"),
                    comment: None,
                    postings: vec![NewPosting {
                        account_id: checking.clone(),
                        amount_quantity: Some("1".into()),
                        amount_commodity: Some("USD".into()),
                        balance_assertion_quantity: None,
                        balance_assertion_commodity: None,
                    }],
                },
            )
            .unwrap();

            let fetched = get_transaction(&conn, &txn.id, &ListOptions::default())
                .unwrap()
                .unwrap();
            assert_eq!(fetched.status, status);
        }
    }

    // --- Balance assertion ---

    #[test]
    fn posting_with_balance_assertion() {
        let conn = setup();
        let (checking, _food) = create_test_accounts(&conn);

        let txn = create_transaction(
            &conn,
            &NewTransaction {
                date: "2024-03-01".into(),
                status: TransactionStatus::Cleared,
                code: None,
                description: "Balance check".into(),
                comment: None,
                postings: vec![NewPosting {
                    account_id: checking,
                    amount_quantity: Some("500.00".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: Some("1500.00".into()),
                    balance_assertion_commodity: Some("USD".into()),
                }],
            },
        )
        .unwrap();

        let fetched = get_transaction(&conn, &txn.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            fetched.postings[0].balance_assertion_quantity.as_deref(),
            Some("1500.00")
        );
    }
}
