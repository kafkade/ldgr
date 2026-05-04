//! Account CRUD operations.
//!
//! Accounts use a hierarchical naming convention (e.g. `"Assets:Checking:Chase"`).
//! Names are unique among active (non-deleted) accounts. Soft-deleted accounts
//! free their name for reuse.

use rusqlite::Connection;
use uuid::Uuid;

use super::error::StorageError;

// ── Types ──────────────────────────────────────────────────────────────────────

/// Account type in the double-entry system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountType {
    Asset,
    Liability,
    Income,
    Expense,
    Equity,
}

impl AccountType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Liability => "liability",
            Self::Income => "income",
            Self::Expense => "expense",
            Self::Equity => "equity",
        }
    }

    fn from_str(s: &str) -> Result<Self, StorageError> {
        match s {
            "asset" => Ok(Self::Asset),
            "liability" => Ok(Self::Liability),
            "income" => Ok(Self::Income),
            "expense" => Ok(Self::Expense),
            "equity" => Ok(Self::Equity),
            _ => Err(StorageError::InvalidInput(format!(
                "unknown account type: {s}"
            ))),
        }
    }
}

/// A persisted account row.
#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub account_type: AccountType,
    pub commodity: Option<String>,
    pub parent_id: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    pub modified_at: String,
    pub version: i64,
    pub deleted: bool,
}

/// Input for creating a new account.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub name: String,
    pub account_type: AccountType,
    pub commodity: Option<String>,
    pub parent_id: Option<String>,
    pub note: Option<String>,
}

/// Input for updating an existing account (full replacement of mutable fields).
#[derive(Debug, Clone)]
pub struct AccountUpdate {
    pub name: String,
    pub account_type: AccountType,
    pub commodity: Option<String>,
    pub parent_id: Option<String>,
    pub note: Option<String>,
    /// Must match the current version or the update is rejected.
    pub expected_version: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub include_deleted: bool,
}

// ── CRUD operations ────────────────────────────────────────────────────────────

/// Create a new account.
///
/// Generates a `UUIDv7` for the account ID and sets timestamps to now.
///
/// # Errors
///
/// Returns [`StorageError::ConstraintViolation`] if the name already exists
/// among active accounts.
pub fn create_account(conn: &Connection, input: &NewAccount) -> Result<Account, StorageError> {
    let id = Uuid::now_v7().to_string();
    let now = now_iso8601();

    conn.execute(
        "INSERT INTO accounts (id, name, type, commodity, parent_id, note, created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            input.name,
            input.account_type.as_str(),
            input.commodity,
            input.parent_id,
            input.note,
            now,
            now,
        ],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(err, _)
            if err.code == rusqlite::ffi::ErrorCode::ConstraintViolation =>
        {
            StorageError::ConstraintViolation(format!(
                "account name '{}' already exists",
                input.name
            ))
        }
        other => StorageError::Database(other),
    })?;

    Ok(Account {
        id,
        name: input.name.clone(),
        account_type: input.account_type,
        commodity: input.commodity.clone(),
        parent_id: input.parent_id.clone(),
        note: input.note.clone(),
        created_at: now.clone(),
        modified_at: now,
        version: 1,
        deleted: false,
    })
}

/// Get an account by ID.
///
/// Returns `None` if not found. Respects `include_deleted` in options.
pub fn get_account(
    conn: &Connection,
    id: &str,
    opts: &ListOptions,
) -> Result<Option<Account>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, name, type, commodity, parent_id, note, created_at, modified_at, version, deleted
         FROM accounts WHERE id = ?1"
    } else {
        "SELECT id, name, type, commodity, parent_id, note, created_at, modified_at, version, deleted
         FROM accounts WHERE id = ?1 AND deleted = 0"
    };

    let mut stmt = conn.prepare(sql)?;
    let result = stmt.query_row([id], row_to_account).optional()?;
    Ok(result)
}

/// Get an account by its hierarchical name (e.g. `"Assets:Checking:Chase"`).
///
/// Only searches active (non-deleted) accounts.
pub fn get_account_by_name(conn: &Connection, name: &str) -> Result<Option<Account>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, type, commodity, parent_id, note, created_at, modified_at, version, deleted
         FROM accounts WHERE name = ?1 AND deleted = 0",
    )?;
    let result = stmt.query_row([name], row_to_account).optional()?;
    Ok(result)
}

/// List all accounts.
pub fn list_accounts(conn: &Connection, opts: &ListOptions) -> Result<Vec<Account>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, name, type, commodity, parent_id, note, created_at, modified_at, version, deleted
         FROM accounts ORDER BY name"
    } else {
        "SELECT id, name, type, commodity, parent_id, note, created_at, modified_at, version, deleted
         FROM accounts WHERE deleted = 0 ORDER BY name"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], row_to_account)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Update an account's mutable fields.
///
/// Uses optimistic concurrency: the update only succeeds if the current
/// version matches `expected_version`. The version is bumped and
/// `modified_at` is set to now.
///
/// # Errors
///
/// Returns [`StorageError::Conflict`] if the version doesn't match
/// (concurrent modification or account was deleted).
pub fn update_account(
    conn: &Connection,
    id: &str,
    update: &AccountUpdate,
) -> Result<Account, StorageError> {
    let now = now_iso8601();
    let new_version = update.expected_version + 1;

    let rows = conn
        .execute(
            "UPDATE accounts
         SET name = ?1, type = ?2, commodity = ?3, parent_id = ?4, note = ?5,
             modified_at = ?6, version = ?7
         WHERE id = ?8 AND version = ?9 AND deleted = 0",
            rusqlite::params![
                update.name,
                update.account_type.as_str(),
                update.commodity,
                update.parent_id,
                update.note,
                now,
                new_version,
                id,
                update.expected_version,
            ],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ffi::ErrorCode::ConstraintViolation =>
            {
                StorageError::ConstraintViolation(format!(
                    "account name '{}' already exists",
                    update.name
                ))
            }
            other => StorageError::Database(other),
        })?;

    if rows == 0 {
        return Err(StorageError::Conflict(format!(
            "account '{id}' was modified or deleted (expected version {})",
            update.expected_version
        )));
    }

    Ok(Account {
        id: id.to_string(),
        name: update.name.clone(),
        account_type: update.account_type,
        commodity: update.commodity.clone(),
        parent_id: update.parent_id.clone(),
        note: update.note.clone(),
        created_at: String::new(), // caller should use the returned value from get
        modified_at: now,
        version: new_version,
        deleted: false,
    })
}

/// Soft-delete an account by setting `deleted = 1`.
///
/// The row is preserved for audit history. The account name becomes
/// available for reuse by new accounts.
///
/// # Errors
///
/// Returns [`StorageError::NotFound`] if the account doesn't exist or
/// is already deleted.
pub fn soft_delete_account(conn: &Connection, id: &str) -> Result<(), StorageError> {
    let rows = conn.execute(
        "UPDATE accounts SET deleted = 1, version = version + 1, modified_at = ?1
         WHERE id = ?2 AND deleted = 0",
        rusqlite::params![now_iso8601(), id],
    )?;

    if rows == 0 {
        return Err(StorageError::NotFound(format!("account '{id}'")));
    }
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let type_str: String = row.get(2)?;
    let account_type =
        AccountType::from_str(&type_str).map_err(|_| rusqlite::Error::InvalidQuery)?;

    Ok(Account {
        id: row.get(0)?,
        name: row.get(1)?,
        account_type,
        commodity: row.get(3)?,
        parent_id: row.get(4)?,
        note: row.get(5)?,
        created_at: row.get(6)?,
        modified_at: row.get(7)?,
        version: row.get(8)?,
        deleted: row.get::<_, i64>(9)? != 0,
    })
}

/// Extension trait for optional query results.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema::initialize;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        conn
    }

    fn sample_account() -> NewAccount {
        NewAccount {
            name: "Assets:Checking:Chase".into(),
            account_type: AccountType::Asset,
            commodity: Some("USD".into()),
            parent_id: None,
            note: Some("Primary checking".into()),
        }
    }

    // --- Create ---

    #[test]
    fn create_account_succeeds() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();

        assert!(!acct.id.is_empty());
        assert_eq!(acct.name, "Assets:Checking:Chase");
        assert_eq!(acct.account_type, AccountType::Asset);
        assert_eq!(acct.commodity.as_deref(), Some("USD"));
        assert_eq!(acct.version, 1);
        assert!(!acct.deleted);
    }

    #[test]
    fn create_duplicate_name_fails() {
        let conn = setup();
        create_account(&conn, &sample_account()).unwrap();

        let result = create_account(&conn, &sample_account());
        assert!(matches!(result, Err(StorageError::ConstraintViolation(_))));
    }

    #[test]
    fn create_after_soft_delete_reuses_name() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();
        soft_delete_account(&conn, &acct.id).unwrap();

        // Same name should now be available
        let acct2 = create_account(&conn, &sample_account()).unwrap();
        assert_ne!(acct.id, acct2.id);
        assert_eq!(acct2.name, "Assets:Checking:Chase");
    }

    // --- Get ---

    #[test]
    fn get_account_by_id() {
        let conn = setup();
        let created = create_account(&conn, &sample_account()).unwrap();

        let fetched = get_account(&conn, &created.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, created.name);
    }

    #[test]
    fn get_deleted_account_excluded_by_default() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();
        soft_delete_account(&conn, &acct.id).unwrap();

        assert!(
            get_account(&conn, &acct.id, &ListOptions::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn get_deleted_account_with_include_deleted() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();
        soft_delete_account(&conn, &acct.id).unwrap();

        let opts = ListOptions {
            include_deleted: true,
        };
        let fetched = get_account(&conn, &acct.id, &opts).unwrap().unwrap();
        assert!(fetched.deleted);
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let conn = setup();
        assert!(
            get_account(&conn, "no-such-id", &ListOptions::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn get_by_name() {
        let conn = setup();
        let created = create_account(&conn, &sample_account()).unwrap();

        let fetched = get_account_by_name(&conn, "Assets:Checking:Chase")
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, created.id);
    }

    // --- List ---

    #[test]
    fn list_accounts_sorted_by_name() {
        let conn = setup();
        create_account(
            &conn,
            &NewAccount {
                name: "Expenses:Food".into(),
                account_type: AccountType::Expense,
                commodity: None,
                parent_id: None,
                note: None,
            },
        )
        .unwrap();
        create_account(
            &conn,
            &NewAccount {
                name: "Assets:Cash".into(),
                account_type: AccountType::Asset,
                commodity: None,
                parent_id: None,
                note: None,
            },
        )
        .unwrap();

        let accounts = list_accounts(&conn, &ListOptions::default()).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].name, "Assets:Cash");
        assert_eq!(accounts[1].name, "Expenses:Food");
    }

    #[test]
    fn list_excludes_deleted_by_default() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();
        soft_delete_account(&conn, &acct.id).unwrap();

        assert!(
            list_accounts(&conn, &ListOptions::default())
                .unwrap()
                .is_empty()
        );
    }

    // --- Update ---

    #[test]
    fn update_account_succeeds() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();

        let updated = update_account(
            &conn,
            &acct.id,
            &AccountUpdate {
                name: "Assets:Checking:BofA".into(),
                account_type: AccountType::Asset,
                commodity: Some("USD".into()),
                parent_id: None,
                note: Some("Switched banks".into()),
                expected_version: 1,
            },
        )
        .unwrap();

        assert_eq!(updated.name, "Assets:Checking:BofA");
        assert_eq!(updated.version, 2);
    }

    #[test]
    fn update_with_wrong_version_fails() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();

        let result = update_account(
            &conn,
            &acct.id,
            &AccountUpdate {
                name: "New Name".into(),
                account_type: AccountType::Asset,
                commodity: None,
                parent_id: None,
                note: None,
                expected_version: 99,
            },
        );
        assert!(matches!(result, Err(StorageError::Conflict(_))));
    }

    // --- Soft delete ---

    #[test]
    fn soft_delete_preserves_row() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();
        soft_delete_account(&conn, &acct.id).unwrap();

        let opts = ListOptions {
            include_deleted: true,
        };
        let all = list_accounts(&conn, &opts).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].deleted);
        assert_eq!(all[0].version, 2); // bumped from 1 to 2
    }

    #[test]
    fn soft_delete_nonexistent_fails() {
        let conn = setup();
        assert!(matches!(
            soft_delete_account(&conn, "no-such-id"),
            Err(StorageError::NotFound(_))
        ));
    }

    #[test]
    fn double_soft_delete_fails() {
        let conn = setup();
        let acct = create_account(&conn, &sample_account()).unwrap();
        soft_delete_account(&conn, &acct.id).unwrap();

        assert!(matches!(
            soft_delete_account(&conn, &acct.id),
            Err(StorageError::NotFound(_))
        ));
    }

    // --- All account types ---

    #[test]
    fn all_account_types_round_trip() {
        let conn = setup();
        for (i, at) in [
            AccountType::Asset,
            AccountType::Liability,
            AccountType::Income,
            AccountType::Expense,
            AccountType::Equity,
        ]
        .iter()
        .enumerate()
        {
            let acct = create_account(
                &conn,
                &NewAccount {
                    name: format!("Type{i}:Test"),
                    account_type: *at,
                    commodity: None,
                    parent_id: None,
                    note: None,
                },
            )
            .unwrap();

            let fetched = get_account(&conn, &acct.id, &ListOptions::default())
                .unwrap()
                .unwrap();
            assert_eq!(fetched.account_type, *at);
        }
    }
}
