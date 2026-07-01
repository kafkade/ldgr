//! Goal CRUD operations.
//!
//! Persists the *definition* of a financial goal (see [`crate::goals::Goal`]) as
//! a first-class, versioned vault entity. Computed progress
//! ([`crate::goals::GoalProgress`]) is never stored — it stays pure compute.
//!
//! This is the flat-entity persistence layer, mirroring
//! [`crate::storage::accounts`]. Rows are versioned with optimistic concurrency
//! and soft-deleted (`deleted = 1`) to preserve audit history. The
//! [`Goal::target_amount`] decimal is stored as TEXT for precision, and
//! `linked_account` is a soft reference (no foreign key) since the referenced
//! account may not exist on every device.

use std::str::FromStr;

use rusqlite::Connection;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::error::StorageError;
use crate::goals::{Goal, GoalType};

// ── Types ──────────────────────────────────────────────────────────────────────

/// A persisted goal row: the [`Goal`] definition plus storage metadata.
#[derive(Debug, Clone)]
pub struct StoredGoal {
    /// The goal definition (id, name, type, target, dates, linked account).
    pub goal: Goal,
    pub created_at: String,
    pub modified_at: String,
    pub version: i64,
    pub deleted: bool,
}

/// Input for creating a new goal.
#[derive(Debug, Clone)]
pub struct NewGoal {
    pub name: String,
    pub goal_type: GoalType,
    pub target_amount: Decimal,
    pub target_date: Option<String>,
    pub linked_account: Option<String>,
}

/// Input for updating an existing goal (full replacement of mutable fields).
#[derive(Debug, Clone)]
pub struct GoalUpdate {
    pub name: String,
    pub goal_type: GoalType,
    pub target_amount: Decimal,
    pub target_date: Option<String>,
    pub linked_account: Option<String>,
    /// Must match the current version or the update is rejected.
    pub expected_version: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub include_deleted: bool,
}

// ── Canonical `GoalType` string mapping ─────────────────────────────────────────

/// Canonical persistence string for a [`GoalType`].
///
/// These strings are stable — the future sync payload depends on them, so they
/// must not change once written to disk.
#[must_use]
pub fn goal_type_as_str(goal_type: GoalType) -> &'static str {
    match goal_type {
        GoalType::Savings => "savings",
        GoalType::DebtPayoff => "debt_payoff",
        GoalType::Investment => "investment",
        GoalType::EmergencyFund => "emergency_fund",
        GoalType::Retirement => "retirement",
        GoalType::Custom => "custom",
    }
}

/// Parse a canonical persistence string into a [`GoalType`].
///
/// # Errors
///
/// Returns [`StorageError::InvalidInput`] for an unknown string.
pub fn goal_type_from_str(s: &str) -> Result<GoalType, StorageError> {
    match s {
        "savings" => Ok(GoalType::Savings),
        "debt_payoff" => Ok(GoalType::DebtPayoff),
        "investment" => Ok(GoalType::Investment),
        "emergency_fund" => Ok(GoalType::EmergencyFund),
        "retirement" => Ok(GoalType::Retirement),
        "custom" => Ok(GoalType::Custom),
        _ => Err(StorageError::InvalidInput(format!(
            "unknown goal type: {s}"
        ))),
    }
}

// ── CRUD operations ────────────────────────────────────────────────────────────

/// Create a new goal.
///
/// Generates a `UUIDv7` for the goal ID and sets timestamps to now.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the insert fails.
pub fn create_goal(conn: &Connection, input: &NewGoal) -> Result<StoredGoal, StorageError> {
    let id = Uuid::now_v7().to_string();
    let now = now_iso8601();

    conn.execute(
        "INSERT INTO goals
             (id, name, goal_type, target_amount, target_date, linked_account,
              created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            input.name,
            goal_type_as_str(input.goal_type),
            input.target_amount.to_string(),
            input.target_date,
            input.linked_account,
            now,
            now,
        ],
    )?;

    Ok(StoredGoal {
        goal: Goal {
            id,
            name: input.name.clone(),
            goal_type: input.goal_type,
            target_amount: input.target_amount,
            target_date: input.target_date.clone(),
            linked_account: input.linked_account.clone(),
        },
        created_at: now.clone(),
        modified_at: now,
        version: 1,
        deleted: false,
    })
}

/// Get a goal by ID.
///
/// Returns `None` if not found. Respects `include_deleted` in options.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on query failure, or
/// [`StorageError::InvalidInput`] if a stored value fails to parse.
pub fn get_goal(
    conn: &Connection,
    id: &str,
    opts: &ListOptions,
) -> Result<Option<StoredGoal>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, name, goal_type, target_amount, target_date, linked_account,
                created_at, modified_at, version, deleted
         FROM goals WHERE id = ?1"
    } else {
        "SELECT id, name, goal_type, target_amount, target_date, linked_account,
                created_at, modified_at, version, deleted
         FROM goals WHERE id = ?1 AND deleted = 0"
    };

    let mut stmt = conn.prepare(sql)?;
    let result = stmt.query_row([id], row_to_goal).optional()?;
    result.transpose()
}

/// List all goals.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on query failure, or
/// [`StorageError::InvalidInput`] if a stored value fails to parse.
pub fn list_goals(conn: &Connection, opts: &ListOptions) -> Result<Vec<StoredGoal>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, name, goal_type, target_amount, target_date, linked_account,
                created_at, modified_at, version, deleted
         FROM goals ORDER BY name"
    } else {
        "SELECT id, name, goal_type, target_amount, target_date, linked_account,
                created_at, modified_at, version, deleted
         FROM goals WHERE deleted = 0 ORDER BY name"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], row_to_goal)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().collect()
}

/// Update a goal's mutable fields.
///
/// Uses optimistic concurrency: the update only succeeds if the current
/// version matches `expected_version`. The version is bumped and
/// `modified_at` is set to now.
///
/// # Errors
///
/// Returns [`StorageError::Conflict`] if the version doesn't match
/// (concurrent modification or goal was deleted).
pub fn update_goal(
    conn: &Connection,
    id: &str,
    update: &GoalUpdate,
) -> Result<StoredGoal, StorageError> {
    let now = now_iso8601();
    let new_version = update.expected_version + 1;

    let rows = conn.execute(
        "UPDATE goals
         SET name = ?1, goal_type = ?2, target_amount = ?3, target_date = ?4,
             linked_account = ?5, modified_at = ?6, version = ?7
         WHERE id = ?8 AND version = ?9 AND deleted = 0",
        rusqlite::params![
            update.name,
            goal_type_as_str(update.goal_type),
            update.target_amount.to_string(),
            update.target_date,
            update.linked_account,
            now,
            new_version,
            id,
            update.expected_version,
        ],
    )?;

    if rows == 0 {
        return Err(StorageError::Conflict(format!(
            "goal '{id}' was modified or deleted (expected version {})",
            update.expected_version
        )));
    }

    // Preserve the original creation timestamp in the returned value.
    let created_at: String = conn
        .query_row("SELECT created_at FROM goals WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .unwrap_or_default();

    Ok(StoredGoal {
        goal: Goal {
            id: id.to_string(),
            name: update.name.clone(),
            goal_type: update.goal_type,
            target_amount: update.target_amount,
            target_date: update.target_date.clone(),
            linked_account: update.linked_account.clone(),
        },
        created_at,
        modified_at: now,
        version: new_version,
        deleted: false,
    })
}

/// Soft-delete a goal by setting `deleted = 1`.
///
/// The row is preserved for audit history and the version is bumped.
///
/// # Errors
///
/// Returns [`StorageError::NotFound`] if the goal doesn't exist or is
/// already deleted.
pub fn soft_delete_goal(conn: &Connection, id: &str) -> Result<(), StorageError> {
    let rows = conn.execute(
        "UPDATE goals SET deleted = 1, version = version + 1, modified_at = ?1
         WHERE id = ?2 AND deleted = 0",
        rusqlite::params![now_iso8601(), id],
    )?;

    if rows == 0 {
        return Err(StorageError::NotFound(format!("goal '{id}'")));
    }
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Map a row to a [`StoredGoal`], parsing the stored decimal and goal-type
/// strings. The outer `rusqlite::Result` covers column access; the inner
/// `Result<StoredGoal, StorageError>` covers value parsing.
fn row_to_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<StoredGoal, StorageError>> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let goal_type_str: String = row.get(2)?;
    let target_amount_str: String = row.get(3)?;
    let target_date: Option<String> = row.get(4)?;
    let linked_account: Option<String> = row.get(5)?;
    let created_at: String = row.get(6)?;
    let modified_at: String = row.get(7)?;
    let version: i64 = row.get(8)?;
    let deleted: bool = row.get::<_, i64>(9)? != 0;

    let goal_type = match goal_type_from_str(&goal_type_str) {
        Ok(gt) => gt,
        Err(e) => return Ok(Err(e)),
    };
    let Ok(target_amount) = Decimal::from_str(&target_amount_str) else {
        return Ok(Err(StorageError::InvalidInput(format!(
            "goal '{id}' has invalid target_amount: {target_amount_str}"
        ))));
    };

    Ok(Ok(StoredGoal {
        goal: Goal {
            id,
            name,
            goal_type,
            target_amount,
            target_date,
            linked_account,
        },
        created_at,
        modified_at,
        version,
        deleted,
    }))
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

    fn sample_goal() -> NewGoal {
        NewGoal {
            name: "Emergency Fund".into(),
            goal_type: GoalType::EmergencyFund,
            target_amount: Decimal::new(10000, 0),
            target_date: Some("2025-12-31".into()),
            linked_account: Some("Assets:Savings".into()),
        }
    }

    // --- Create ---

    #[test]
    fn create_goal_succeeds() {
        let conn = setup();
        let g = create_goal(&conn, &sample_goal()).unwrap();

        assert!(!g.goal.id.is_empty());
        assert_eq!(g.goal.name, "Emergency Fund");
        assert_eq!(g.goal.goal_type, GoalType::EmergencyFund);
        assert_eq!(g.goal.target_amount, Decimal::new(10000, 0));
        assert_eq!(g.goal.target_date.as_deref(), Some("2025-12-31"));
        assert_eq!(g.goal.linked_account.as_deref(), Some("Assets:Savings"));
        assert_eq!(g.version, 1);
        assert!(!g.deleted);
    }

    #[test]
    fn create_goal_with_optional_fields_none() {
        let conn = setup();
        let g = create_goal(
            &conn,
            &NewGoal {
                name: "New Car".into(),
                goal_type: GoalType::Custom,
                target_amount: Decimal::new(25000, 0),
                target_date: None,
                linked_account: None,
            },
        )
        .unwrap();

        assert!(g.goal.target_date.is_none());
        assert!(g.goal.linked_account.is_none());
    }

    // --- Get ---

    #[test]
    fn get_goal_returns_created() {
        let conn = setup();
        let created = create_goal(&conn, &sample_goal()).unwrap();

        let fetched = get_goal(&conn, &created.goal.id, &ListOptions::default())
            .unwrap()
            .expect("goal should exist");
        assert_eq!(fetched.goal.id, created.goal.id);
        assert_eq!(fetched.goal.name, "Emergency Fund");
        assert_eq!(fetched.goal.target_amount, Decimal::new(10000, 0));
    }

    #[test]
    fn get_missing_goal_returns_none() {
        let conn = setup();
        let result = get_goal(&conn, "nonexistent", &ListOptions::default()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn get_deleted_goal_hidden_by_default() {
        let conn = setup();
        let created = create_goal(&conn, &sample_goal()).unwrap();
        soft_delete_goal(&conn, &created.goal.id).unwrap();

        assert!(
            get_goal(&conn, &created.goal.id, &ListOptions::default())
                .unwrap()
                .is_none()
        );

        let with_deleted = get_goal(
            &conn,
            &created.goal.id,
            &ListOptions {
                include_deleted: true,
            },
        )
        .unwrap()
        .expect("goal should exist when include_deleted");
        assert!(with_deleted.deleted);
    }

    // --- List ---

    #[test]
    fn list_goals_orders_by_name() {
        let conn = setup();
        create_goal(
            &conn,
            &NewGoal {
                name: "Zebra".into(),
                goal_type: GoalType::Savings,
                target_amount: Decimal::new(100, 0),
                target_date: None,
                linked_account: None,
            },
        )
        .unwrap();
        create_goal(
            &conn,
            &NewGoal {
                name: "Alpha".into(),
                goal_type: GoalType::Savings,
                target_amount: Decimal::new(200, 0),
                target_date: None,
                linked_account: None,
            },
        )
        .unwrap();

        let goals = list_goals(&conn, &ListOptions::default()).unwrap();
        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0].goal.name, "Alpha");
        assert_eq!(goals[1].goal.name, "Zebra");
    }

    #[test]
    fn list_goals_excludes_deleted_by_default() {
        let conn = setup();
        let g1 = create_goal(&conn, &sample_goal()).unwrap();
        create_goal(
            &conn,
            &NewGoal {
                name: "Retirement".into(),
                goal_type: GoalType::Retirement,
                target_amount: Decimal::new(1_000_000, 0),
                target_date: None,
                linked_account: None,
            },
        )
        .unwrap();
        soft_delete_goal(&conn, &g1.goal.id).unwrap();

        let active = list_goals(&conn, &ListOptions::default()).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].goal.name, "Retirement");

        let all = list_goals(
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
    fn update_goal_bumps_version_and_fields() {
        let conn = setup();
        let created = create_goal(&conn, &sample_goal()).unwrap();

        let updated = update_goal(
            &conn,
            &created.goal.id,
            &GoalUpdate {
                name: "Bigger Emergency Fund".into(),
                goal_type: GoalType::Savings,
                target_amount: Decimal::new(20000, 0),
                target_date: Some("2026-06-30".into()),
                linked_account: Some("Assets:HYSA".into()),
                expected_version: 1,
            },
        )
        .unwrap();

        assert_eq!(updated.version, 2);
        assert_eq!(updated.goal.name, "Bigger Emergency Fund");
        assert_eq!(updated.goal.goal_type, GoalType::Savings);
        assert_eq!(updated.goal.target_amount, Decimal::new(20000, 0));
        assert_eq!(updated.goal.target_date.as_deref(), Some("2026-06-30"));
        assert_eq!(updated.goal.linked_account.as_deref(), Some("Assets:HYSA"));
        assert_eq!(updated.created_at, created.created_at);

        // Confirm the change round-trips through the DB.
        let fetched = get_goal(&conn, &created.goal.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(fetched.version, 2);
        assert_eq!(fetched.goal.target_amount, Decimal::new(20000, 0));
    }

    #[test]
    fn update_goal_version_mismatch_conflicts() {
        let conn = setup();
        let created = create_goal(&conn, &sample_goal()).unwrap();

        let result = update_goal(
            &conn,
            &created.goal.id,
            &GoalUpdate {
                name: "Stale".into(),
                goal_type: GoalType::EmergencyFund,
                target_amount: Decimal::new(10000, 0),
                target_date: None,
                linked_account: None,
                expected_version: 99,
            },
        );
        assert!(matches!(result, Err(StorageError::Conflict(_))));
    }

    #[test]
    fn update_deleted_goal_conflicts() {
        let conn = setup();
        let created = create_goal(&conn, &sample_goal()).unwrap();
        soft_delete_goal(&conn, &created.goal.id).unwrap();

        let result = update_goal(
            &conn,
            &created.goal.id,
            &GoalUpdate {
                name: "Resurrect".into(),
                goal_type: GoalType::EmergencyFund,
                target_amount: Decimal::new(10000, 0),
                target_date: None,
                linked_account: None,
                expected_version: 1,
            },
        );
        assert!(matches!(result, Err(StorageError::Conflict(_))));
    }

    // --- Soft delete ---

    #[test]
    fn soft_delete_goal_succeeds() {
        let conn = setup();
        let created = create_goal(&conn, &sample_goal()).unwrap();

        soft_delete_goal(&conn, &created.goal.id).unwrap();

        let fetched = get_goal(
            &conn,
            &created.goal.id,
            &ListOptions {
                include_deleted: true,
            },
        )
        .unwrap()
        .unwrap();
        assert!(fetched.deleted);
        assert_eq!(fetched.version, 2);
    }

    #[test]
    fn soft_delete_missing_goal_not_found() {
        let conn = setup();
        let result = soft_delete_goal(&conn, "nonexistent");
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[test]
    fn soft_delete_already_deleted_not_found() {
        let conn = setup();
        let created = create_goal(&conn, &sample_goal()).unwrap();
        soft_delete_goal(&conn, &created.goal.id).unwrap();

        let result = soft_delete_goal(&conn, &created.goal.id);
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    // --- Round-trip & type mapping ---

    #[test]
    fn decimal_precision_round_trips() {
        let conn = setup();
        let created = create_goal(
            &conn,
            &NewGoal {
                name: "Precise".into(),
                goal_type: GoalType::Investment,
                target_amount: Decimal::from_str("12345.6789").unwrap(),
                target_date: None,
                linked_account: None,
            },
        )
        .unwrap();

        let fetched = get_goal(&conn, &created.goal.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            fetched.goal.target_amount,
            Decimal::from_str("12345.6789").unwrap()
        );
    }

    #[test]
    fn all_goal_types_round_trip() {
        for gt in [
            GoalType::Savings,
            GoalType::DebtPayoff,
            GoalType::Investment,
            GoalType::EmergencyFund,
            GoalType::Retirement,
            GoalType::Custom,
        ] {
            assert_eq!(goal_type_from_str(goal_type_as_str(gt)).unwrap(), gt);
        }
    }

    #[test]
    fn goal_type_from_unknown_string_errors() {
        assert!(matches!(
            goal_type_from_str("bogus"),
            Err(StorageError::InvalidInput(_))
        ));
    }

    #[test]
    fn goal_type_persists_correctly() {
        let conn = setup();
        let created = create_goal(
            &conn,
            &NewGoal {
                name: "Debt".into(),
                goal_type: GoalType::DebtPayoff,
                target_amount: Decimal::new(5000, 0),
                target_date: None,
                linked_account: None,
            },
        )
        .unwrap();

        let stored_str: String = conn
            .query_row(
                "SELECT goal_type FROM goals WHERE id = ?1",
                [&created.goal.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_str, "debt_payoff");

        let fetched = get_goal(&conn, &created.goal.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(fetched.goal.goal_type, GoalType::DebtPayoff);
    }
}
