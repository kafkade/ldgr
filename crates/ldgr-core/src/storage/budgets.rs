//! Budget CRUD operations.
//!
//! Persists the *definition* of a budget (see [`crate::budget::Budget`]) as a
//! first-class, versioned vault entity. Computed results
//! ([`crate::budget::BudgetReport`] / [`crate::budget::BudgetVsActual`]) are
//! never stored — they stay pure compute.
//!
//! This is the parent+child persistence layer, mirroring
//! [`crate::storage::transactions`] (transactions + postings). A budget's
//! nested `allocations` collection is stored in a dedicated
//! `budget_allocations` child table — **not** an embedded JSON column — so the
//! caller-supplied order is reproduced deterministically on read via
//! `allocation_order` (this matters for the future sync payload, exactly like
//! `posting_order`).
//!
//! Rows are versioned with optimistic concurrency and soft-deleted
//! (`deleted = 1`) to preserve audit history. Allocation [`Decimal`] amounts are
//! stored as TEXT for precision, and `account` is a soft reference (no foreign
//! key) since the referenced account may not exist on every device.

use std::str::FromStr;

use rusqlite::Connection;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::error::StorageError;
use crate::budget::{Budget, BudgetAllocation, BudgetMethod, BudgetPeriod};

// ── Types ──────────────────────────────────────────────────────────────────────

/// A persisted budget row: the [`Budget`] definition plus storage metadata.
#[derive(Debug, Clone)]
pub struct StoredBudget {
    /// The budget definition (id, name, method, period, allocations).
    pub budget: Budget,
    pub created_at: String,
    pub modified_at: String,
    pub version: i64,
    pub deleted: bool,
}

/// Input for a single allocation within a new or updated budget.
#[derive(Debug, Clone)]
pub struct NewBudgetAllocation {
    pub account: String,
    pub amount: Decimal,
    pub rollover: bool,
}

/// Input for creating a new budget.
#[derive(Debug, Clone)]
pub struct NewBudget {
    pub name: String,
    pub method: BudgetMethod,
    pub period: BudgetPeriod,
    pub allocations: Vec<NewBudgetAllocation>,
}

/// Input for updating an existing budget (full replacement of mutable fields,
/// including the entire allocations collection).
#[derive(Debug, Clone)]
pub struct BudgetUpdate {
    pub name: String,
    pub method: BudgetMethod,
    pub period: BudgetPeriod,
    pub allocations: Vec<NewBudgetAllocation>,
    /// Must match the current version or the update is rejected.
    pub expected_version: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub include_deleted: bool,
}

// ── Canonical enum string mappings ──────────────────────────────────────────────

/// Canonical persistence string for a [`BudgetMethod`].
///
/// These strings are stable — the future sync payload depends on them, so they
/// must not change once written to disk.
#[must_use]
pub fn method_as_str(method: BudgetMethod) -> &'static str {
    match method {
        BudgetMethod::Envelope => "envelope",
        BudgetMethod::ZeroBased => "zero_based",
    }
}

/// Parse a canonical persistence string into a [`BudgetMethod`].
///
/// # Errors
///
/// Returns [`StorageError::InvalidInput`] for an unknown string.
pub fn method_from_str(s: &str) -> Result<BudgetMethod, StorageError> {
    match s {
        "envelope" => Ok(BudgetMethod::Envelope),
        "zero_based" => Ok(BudgetMethod::ZeroBased),
        _ => Err(StorageError::InvalidInput(format!(
            "unknown budget method: {s}"
        ))),
    }
}

/// Canonical persistence string for a [`BudgetPeriod`].
///
/// These strings are stable — the future sync payload depends on them, so they
/// must not change once written to disk.
#[must_use]
pub fn period_as_str(period: BudgetPeriod) -> &'static str {
    match period {
        BudgetPeriod::Monthly => "monthly",
        BudgetPeriod::Weekly => "weekly",
        BudgetPeriod::Quarterly => "quarterly",
        BudgetPeriod::Annual => "annual",
    }
}

/// Parse a canonical persistence string into a [`BudgetPeriod`].
///
/// # Errors
///
/// Returns [`StorageError::InvalidInput`] for an unknown string.
pub fn period_from_str(s: &str) -> Result<BudgetPeriod, StorageError> {
    match s {
        "monthly" => Ok(BudgetPeriod::Monthly),
        "weekly" => Ok(BudgetPeriod::Weekly),
        "quarterly" => Ok(BudgetPeriod::Quarterly),
        "annual" => Ok(BudgetPeriod::Annual),
        _ => Err(StorageError::InvalidInput(format!(
            "unknown budget period: {s}"
        ))),
    }
}

// ── CRUD operations ────────────────────────────────────────────────────────────

/// Create a new budget with its allocations atomically.
///
/// Generates a `UUIDv7` for the budget ID and sets timestamps to now. The budget
/// row and all its allocations are written in a single `SQLite` transaction.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the insert fails.
pub fn create_budget(conn: &Connection, input: &NewBudget) -> Result<StoredBudget, StorageError> {
    let budget_id = Uuid::now_v7().to_string();
    let now = now_iso8601();

    let db_tx = conn.unchecked_transaction()?;

    db_tx.execute(
        "INSERT INTO budgets (id, name, method, period, created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            budget_id,
            input.name,
            method_as_str(input.method),
            period_as_str(input.period),
            now,
            now,
        ],
    )?;

    let allocations = insert_allocations(&db_tx, &budget_id, &input.allocations, &now)?;

    db_tx.commit()?;

    Ok(StoredBudget {
        budget: Budget {
            id: budget_id,
            name: input.name.clone(),
            method: input.method,
            period: input.period,
            allocations,
        },
        created_at: now.clone(),
        modified_at: now,
        version: 1,
        deleted: false,
    })
}

/// Get a budget by ID, including its allocations.
///
/// Returns `None` if not found. Respects `include_deleted` in options.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on query failure, or
/// [`StorageError::InvalidInput`] if a stored value fails to parse.
pub fn get_budget(
    conn: &Connection,
    id: &str,
    opts: &ListOptions,
) -> Result<Option<StoredBudget>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, name, method, period, created_at, modified_at, version, deleted
         FROM budgets WHERE id = ?1"
    } else {
        "SELECT id, name, method, period, created_at, modified_at, version, deleted
         FROM budgets WHERE id = ?1 AND deleted = 0"
    };

    let mut stmt = conn.prepare(sql)?;
    let header = match stmt.query_row([id], row_to_budget_header) {
        Ok(h) => h?,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let allocations = fetch_allocations(conn, &header.budget.id)?;
    Ok(Some(with_allocations(header, allocations)))
}

/// List all budgets with their allocations.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on query failure, or
/// [`StorageError::InvalidInput`] if a stored value fails to parse.
pub fn list_budgets(
    conn: &Connection,
    opts: &ListOptions,
) -> Result<Vec<StoredBudget>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, name, method, period, created_at, modified_at, version, deleted
         FROM budgets ORDER BY name"
    } else {
        "SELECT id, name, method, period, created_at, modified_at, version, deleted
         FROM budgets WHERE deleted = 0 ORDER BY name"
    };

    let mut stmt = conn.prepare(sql)?;
    let headers = stmt
        .query_map([], row_to_budget_header)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(headers.len());
    for header in headers {
        let allocations = fetch_allocations(conn, &header.budget.id)?;
        result.push(with_allocations(header, allocations));
    }

    Ok(result)
}

/// Update a budget's mutable fields and replace all its allocations atomically.
///
/// Uses optimistic concurrency: the update only succeeds if the current version
/// matches `expected_version`. The version is bumped and `modified_at` is set to
/// now.
///
/// # Errors
///
/// Returns [`StorageError::Conflict`] if the version doesn't match (concurrent
/// modification or budget was deleted).
pub fn update_budget(
    conn: &Connection,
    id: &str,
    update: &BudgetUpdate,
) -> Result<StoredBudget, StorageError> {
    let now = now_iso8601();
    let new_version = update.expected_version + 1;

    let db_tx = conn.unchecked_transaction()?;

    let rows = db_tx.execute(
        "UPDATE budgets
         SET name = ?1, method = ?2, period = ?3, modified_at = ?4, version = ?5
         WHERE id = ?6 AND version = ?7 AND deleted = 0",
        rusqlite::params![
            update.name,
            method_as_str(update.method),
            period_as_str(update.period),
            now,
            new_version,
            id,
            update.expected_version,
        ],
    )?;

    if rows == 0 {
        db_tx.rollback()?;
        return Err(StorageError::Conflict(format!(
            "budget '{id}' was modified or deleted (expected version {})",
            update.expected_version
        )));
    }

    // Replace allocations: delete old, insert new.
    db_tx.execute("DELETE FROM budget_allocations WHERE budget_id = ?1", [id])?;
    let allocations = insert_allocations(&db_tx, id, &update.allocations, &now)?;

    // Preserve the original creation timestamp in the returned value.
    let created_at: String = db_tx
        .query_row(
            "SELECT created_at FROM budgets WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap_or_default();

    db_tx.commit()?;

    Ok(StoredBudget {
        budget: Budget {
            id: id.to_string(),
            name: update.name.clone(),
            method: update.method,
            period: update.period,
            allocations,
        },
        created_at,
        modified_at: now,
        version: new_version,
        deleted: false,
    })
}

/// Soft-delete a budget by setting `deleted = 1`.
///
/// The row (and its allocations) are preserved for audit history and the version
/// is bumped.
///
/// # Errors
///
/// Returns [`StorageError::NotFound`] if the budget doesn't exist or is already
/// deleted.
pub fn soft_delete_budget(conn: &Connection, id: &str) -> Result<(), StorageError> {
    let rows = conn.execute(
        "UPDATE budgets SET deleted = 1, version = version + 1, modified_at = ?1
         WHERE id = ?2 AND deleted = 0",
        rusqlite::params![now_iso8601(), id],
    )?;

    if rows == 0 {
        return Err(StorageError::NotFound(format!("budget '{id}'")));
    }
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Insert the allocations for a budget, assigning a deterministic
/// `allocation_order` from the caller-supplied order.
fn insert_allocations(
    conn: &Connection,
    budget_id: &str,
    allocations: &[NewBudgetAllocation],
    now: &str,
) -> Result<Vec<BudgetAllocation>, StorageError> {
    let mut result = Vec::with_capacity(allocations.len());

    for (i, a) in allocations.iter().enumerate() {
        let allocation_id = Uuid::now_v7().to_string();
        let order = i64::try_from(i)
            .map_err(|_| StorageError::InvalidInput("too many allocations".into()))?;

        conn.execute(
            "INSERT INTO budget_allocations
                 (id, budget_id, account, amount, rollover, allocation_order, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                allocation_id,
                budget_id,
                a.account,
                a.amount.to_string(),
                i64::from(a.rollover),
                order,
                now,
            ],
        )?;

        result.push(BudgetAllocation {
            account: a.account.clone(),
            amount: a.amount,
            rollover: a.rollover,
        });
    }

    Ok(result)
}

/// Fetch all allocations for a budget, ordered by `allocation_order`.
fn fetch_allocations(
    conn: &Connection,
    budget_id: &str,
) -> Result<Vec<BudgetAllocation>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT account, amount, rollover
         FROM budget_allocations
         WHERE budget_id = ?1
         ORDER BY allocation_order",
    )?;

    let rows = stmt
        .query_map([budget_id], |row| {
            let account: String = row.get(0)?;
            let amount_str: String = row.get(1)?;
            let rollover: bool = row.get::<_, i64>(2)? != 0;
            Ok((account, amount_str, rollover))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(rows.len());
    for (account, amount_str, rollover) in rows {
        let amount = Decimal::from_str(&amount_str).map_err(|_| {
            StorageError::InvalidInput(format!(
                "budget '{budget_id}' allocation '{account}' has invalid amount: {amount_str}"
            ))
        })?;
        result.push(BudgetAllocation {
            account,
            amount,
            rollover,
        });
    }

    Ok(result)
}

/// Map a budget row to a [`StoredBudget`] header (allocations filled in by the
/// caller). The outer `rusqlite::Result` covers column access; the inner
/// `Result<StoredBudget, StorageError>` covers enum-string parsing.
fn row_to_budget_header(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<StoredBudget, StorageError>> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let method_str: String = row.get(2)?;
    let period_str: String = row.get(3)?;
    let created_at: String = row.get(4)?;
    let modified_at: String = row.get(5)?;
    let version: i64 = row.get(6)?;
    let deleted: bool = row.get::<_, i64>(7)? != 0;

    let method = match method_from_str(&method_str) {
        Ok(m) => m,
        Err(e) => return Ok(Err(e)),
    };
    let period = match period_from_str(&period_str) {
        Ok(p) => p,
        Err(e) => return Ok(Err(e)),
    };

    Ok(Ok(StoredBudget {
        budget: Budget {
            id,
            name,
            method,
            period,
            allocations: Vec::new(), // filled in by caller
        },
        created_at,
        modified_at,
        version,
        deleted,
    }))
}

/// Attach a fetched allocations collection to a budget header.
fn with_allocations(mut header: StoredBudget, allocations: Vec<BudgetAllocation>) -> StoredBudget {
    header.budget.allocations = allocations;
    header
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema::initialize;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        conn
    }

    fn sample_budget() -> NewBudget {
        NewBudget {
            name: "Monthly".into(),
            method: BudgetMethod::Envelope,
            period: BudgetPeriod::Monthly,
            allocations: vec![
                NewBudgetAllocation {
                    account: "Expenses:Food".into(),
                    amount: Decimal::new(500, 0),
                    rollover: true,
                },
                NewBudgetAllocation {
                    account: "Expenses:Transport".into(),
                    amount: Decimal::new(200, 0),
                    rollover: false,
                },
                NewBudgetAllocation {
                    account: "Expenses:Entertainment".into(),
                    amount: Decimal::new(100, 0),
                    rollover: true,
                },
            ],
        }
    }

    // --- Create ---

    #[test]
    fn create_budget_succeeds() {
        let conn = setup();
        let b = create_budget(&conn, &sample_budget()).unwrap();

        assert!(!b.budget.id.is_empty());
        assert_eq!(b.budget.name, "Monthly");
        assert_eq!(b.budget.method, BudgetMethod::Envelope);
        assert_eq!(b.budget.period, BudgetPeriod::Monthly);
        assert_eq!(b.budget.allocations.len(), 3);
        assert_eq!(b.version, 1);
        assert!(!b.deleted);
    }

    #[test]
    fn create_budget_with_no_allocations() {
        let conn = setup();
        let b = create_budget(
            &conn,
            &NewBudget {
                name: "Empty".into(),
                method: BudgetMethod::ZeroBased,
                period: BudgetPeriod::Weekly,
                allocations: vec![],
            },
        )
        .unwrap();

        assert!(b.budget.allocations.is_empty());
    }

    // --- Get ---

    #[test]
    fn get_budget_returns_created() {
        let conn = setup();
        let created = create_budget(&conn, &sample_budget()).unwrap();

        let fetched = get_budget(&conn, &created.budget.id, &ListOptions::default())
            .unwrap()
            .expect("budget should exist");
        assert_eq!(fetched.budget.id, created.budget.id);
        assert_eq!(fetched.budget.name, "Monthly");
        assert_eq!(fetched.budget.allocations.len(), 3);
    }

    #[test]
    fn get_missing_budget_returns_none() {
        let conn = setup();
        assert!(
            get_budget(&conn, "nope", &ListOptions::default())
                .unwrap()
                .is_none()
        );
    }

    /// The nested allocations round-trip through storage unchanged, in the
    /// caller-supplied order.
    #[test]
    fn allocations_round_trip() {
        let conn = setup();
        let created = create_budget(&conn, &sample_budget()).unwrap();

        let fetched = get_budget(&conn, &created.budget.id, &ListOptions::default())
            .unwrap()
            .unwrap();

        let allocs = &fetched.budget.allocations;
        assert_eq!(allocs.len(), 3);

        assert_eq!(allocs[0].account, "Expenses:Food");
        assert_eq!(allocs[0].amount, Decimal::new(500, 0));
        assert!(allocs[0].rollover);

        assert_eq!(allocs[1].account, "Expenses:Transport");
        assert_eq!(allocs[1].amount, Decimal::new(200, 0));
        assert!(!allocs[1].rollover);

        assert_eq!(allocs[2].account, "Expenses:Entertainment");
        assert_eq!(allocs[2].amount, Decimal::new(100, 0));
        assert!(allocs[2].rollover);
    }

    /// Allocation order is deterministic and preserved on read regardless of
    /// account name ordering (i.e. it follows insertion order, not alphabetical).
    #[test]
    fn allocation_order_is_deterministic() {
        let conn = setup();
        let created = create_budget(
            &conn,
            &NewBudget {
                name: "Ordered".into(),
                method: BudgetMethod::Envelope,
                period: BudgetPeriod::Monthly,
                allocations: vec![
                    NewBudgetAllocation {
                        account: "Zebra".into(),
                        amount: Decimal::new(1, 0),
                        rollover: false,
                    },
                    NewBudgetAllocation {
                        account: "Apple".into(),
                        amount: Decimal::new(2, 0),
                        rollover: false,
                    },
                    NewBudgetAllocation {
                        account: "Mango".into(),
                        amount: Decimal::new(3, 0),
                        rollover: false,
                    },
                ],
            },
        )
        .unwrap();

        let fetched = get_budget(&conn, &created.budget.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        let accounts: Vec<&str> = fetched
            .budget
            .allocations
            .iter()
            .map(|a| a.account.as_str())
            .collect();
        assert_eq!(accounts, vec!["Zebra", "Apple", "Mango"]);
    }

    #[test]
    fn stores_decimal_amount_as_text() {
        let conn = setup();
        let created = create_budget(
            &conn,
            &NewBudget {
                name: "Precise".into(),
                method: BudgetMethod::Envelope,
                period: BudgetPeriod::Monthly,
                allocations: vec![NewBudgetAllocation {
                    account: "Expenses:Rent".into(),
                    amount: Decimal::from_str("1234.56").unwrap(),
                    rollover: false,
                }],
            },
        )
        .unwrap();

        let stored: String = conn
            .query_row(
                "SELECT amount FROM budget_allocations WHERE budget_id = ?1",
                [&created.budget.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "1234.56");

        let fetched = get_budget(&conn, &created.budget.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            fetched.budget.allocations[0].amount,
            Decimal::from_str("1234.56").unwrap()
        );
    }

    // --- List ---

    #[test]
    fn list_budgets_excludes_deleted_by_default() {
        let conn = setup();
        let a = create_budget(
            &conn,
            &NewBudget {
                name: "A".into(),
                method: BudgetMethod::Envelope,
                period: BudgetPeriod::Monthly,
                allocations: vec![],
            },
        )
        .unwrap();
        create_budget(
            &conn,
            &NewBudget {
                name: "B".into(),
                method: BudgetMethod::Envelope,
                period: BudgetPeriod::Monthly,
                allocations: vec![],
            },
        )
        .unwrap();

        soft_delete_budget(&conn, &a.budget.id).unwrap();

        let active = list_budgets(&conn, &ListOptions::default()).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].budget.name, "B");

        let all = list_budgets(
            &conn,
            &ListOptions {
                include_deleted: true,
            },
        )
        .unwrap();
        assert_eq!(all.len(), 2);
    }

    // --- Update ---

    #[test]
    fn update_budget_replaces_fields_and_allocations() {
        let conn = setup();
        let created = create_budget(&conn, &sample_budget()).unwrap();

        let updated = update_budget(
            &conn,
            &created.budget.id,
            &BudgetUpdate {
                name: "Renamed".into(),
                method: BudgetMethod::ZeroBased,
                period: BudgetPeriod::Annual,
                allocations: vec![NewBudgetAllocation {
                    account: "Expenses:Savings".into(),
                    amount: Decimal::new(999, 0),
                    rollover: false,
                }],
                expected_version: created.version,
            },
        )
        .unwrap();

        assert_eq!(updated.budget.name, "Renamed");
        assert_eq!(updated.budget.method, BudgetMethod::ZeroBased);
        assert_eq!(updated.budget.period, BudgetPeriod::Annual);
        assert_eq!(updated.version, 2);
        assert_eq!(updated.budget.allocations.len(), 1);
        assert_eq!(updated.budget.allocations[0].account, "Expenses:Savings");
        assert_eq!(updated.created_at, created.created_at);

        // Old allocations are gone (fully replaced).
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM budget_allocations WHERE budget_id = ?1",
                [&created.budget.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn update_budget_version_conflict_is_rejected() {
        let conn = setup();
        let created = create_budget(&conn, &sample_budget()).unwrap();

        let err = update_budget(
            &conn,
            &created.budget.id,
            &BudgetUpdate {
                name: "Stale".into(),
                method: BudgetMethod::Envelope,
                period: BudgetPeriod::Monthly,
                allocations: vec![],
                expected_version: created.version + 5, // wrong version
            },
        )
        .unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));

        // The original allocations are untouched after the failed (rolled-back) update.
        let fetched = get_budget(&conn, &created.budget.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(fetched.budget.allocations.len(), 3);
    }

    // --- Soft delete ---

    #[test]
    fn soft_delete_hides_budget_and_bumps_version() {
        let conn = setup();
        let created = create_budget(&conn, &sample_budget()).unwrap();

        soft_delete_budget(&conn, &created.budget.id).unwrap();

        assert!(
            get_budget(&conn, &created.budget.id, &ListOptions::default())
                .unwrap()
                .is_none()
        );

        let with_deleted = get_budget(
            &conn,
            &created.budget.id,
            &ListOptions {
                include_deleted: true,
            },
        )
        .unwrap()
        .unwrap();
        assert!(with_deleted.deleted);
        assert_eq!(with_deleted.version, created.version + 1);
    }

    #[test]
    fn soft_delete_missing_budget_errors() {
        let conn = setup();
        let err = soft_delete_budget(&conn, "nope").unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    #[test]
    fn double_soft_delete_errors() {
        let conn = setup();
        let created = create_budget(&conn, &sample_budget()).unwrap();
        soft_delete_budget(&conn, &created.budget.id).unwrap();
        let err = soft_delete_budget(&conn, &created.budget.id).unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    // --- Enum string mappings ---

    #[test]
    fn method_string_round_trip() {
        for m in [BudgetMethod::Envelope, BudgetMethod::ZeroBased] {
            assert_eq!(method_from_str(method_as_str(m)).unwrap(), m);
        }
        assert!(method_from_str("bogus").is_err());
    }

    #[test]
    fn period_string_round_trip() {
        for p in [
            BudgetPeriod::Monthly,
            BudgetPeriod::Weekly,
            BudgetPeriod::Quarterly,
            BudgetPeriod::Annual,
        ] {
            assert_eq!(period_from_str(period_as_str(p)).unwrap(), p);
        }
        assert!(period_from_str("bogus").is_err());
    }
}
