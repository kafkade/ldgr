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
use super::sync::SyncContext;
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

// ── Sync-aware variants ────────────────────────────────────────────────────────

/// Create a new goal with atomic sync event recording.
///
/// Mirrors [`create_goal`] but, within the same `SQLite` transaction, also
/// records a `create` outbox event carrying the full [`GoalPayload`] so the
/// change propagates on the next `sync push`. `event.version` is set to `1`
/// (the resulting row version).
///
/// [`GoalPayload`]: crate::sync::payload::GoalPayload
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the insert fails, or
/// [`StorageError::InvalidInput`] if the payload cannot be serialized.
pub fn create_goal_with_sync(
    conn: &Connection,
    input: &NewGoal,
    ctx: &SyncContext,
) -> Result<StoredGoal, StorageError> {
    let db_tx = conn.unchecked_transaction()?;

    let id = Uuid::now_v7().to_string();
    let now = now_iso8601();

    db_tx.execute(
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

    let payload = crate::sync::payload::to_bytes(&crate::sync::payload::GoalPayload {
        id: id.clone(),
        name: input.name.clone(),
        goal_type: goal_type_as_str(input.goal_type).to_string(),
        target_amount: input.target_amount.to_string(),
        target_date: input.target_date.clone(),
        linked_account: input.linked_account.clone(),
        created_at: now.clone(),
        modified_at: now.clone(),
    })
    .map_err(|e| StorageError::InvalidInput(format!("failed to serialize goal payload: {e}")))?;

    super::sync::record_event(
        &db_tx,
        &ctx.device_id,
        "goal",
        &id,
        "create",
        &payload,
        ctx.lamport_clock,
        1,
    )?;

    db_tx.commit()?;

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

/// Update an existing goal with atomic sync event recording.
///
/// Mirrors [`update_goal`] but, within the same `SQLite` transaction, also
/// records an `update` outbox event carrying the full post-update
/// [`GoalPayload`]. `event.version` is set to the resulting row version so the
/// remote apply path's staleness check works. The original `created_at` is
/// preserved so the emitted payload keeps it stable across the update.
///
/// [`GoalPayload`]: crate::sync::payload::GoalPayload
///
/// # Errors
///
/// Returns [`StorageError::Conflict`] if the version doesn't match (concurrent
/// modification or goal was deleted), or [`StorageError::InvalidInput`] if the
/// payload cannot be serialized.
pub fn update_goal_with_sync(
    conn: &Connection,
    id: &str,
    update: &GoalUpdate,
    ctx: &SyncContext,
) -> Result<StoredGoal, StorageError> {
    let db_tx = conn.unchecked_transaction()?;

    let now = now_iso8601();
    let new_version = update.expected_version + 1;

    // Preserve the original creation timestamp so the emitted payload (and any
    // remote upsert it drives) keeps `created_at` stable across the update.
    let created_at: String = db_tx
        .query_row(
            "SELECT created_at FROM goals WHERE id = ?1 AND deleted = 0",
            [id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(format!("goal '{id}'")),
            other => StorageError::Database(other),
        })?;

    let rows = db_tx.execute(
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

    let payload = crate::sync::payload::to_bytes(&crate::sync::payload::GoalPayload {
        id: id.to_string(),
        name: update.name.clone(),
        goal_type: goal_type_as_str(update.goal_type).to_string(),
        target_amount: update.target_amount.to_string(),
        target_date: update.target_date.clone(),
        linked_account: update.linked_account.clone(),
        created_at: created_at.clone(),
        modified_at: now.clone(),
    })
    .map_err(|e| StorageError::InvalidInput(format!("failed to serialize goal payload: {e}")))?;

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let version_u32 = new_version as u32;

    super::sync::record_event(
        &db_tx,
        &ctx.device_id,
        "goal",
        id,
        "update",
        &payload,
        ctx.lamport_clock,
        version_u32,
    )?;

    db_tx.commit()?;

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

/// Soft-delete a goal with atomic sync event recording.
///
/// Mirrors [`soft_delete_goal`] but, within the same `SQLite` transaction, also
/// records a `delete` outbox event. `event.version` is set to the resulting
/// (post-delete) row version so the remote apply path's staleness check works.
///
/// # Errors
///
/// Returns [`StorageError::NotFound`] if the goal doesn't exist or is already
/// deleted, or [`StorageError::InvalidInput`] if the payload cannot be
/// serialized.
pub fn soft_delete_goal_with_sync(
    conn: &Connection,
    id: &str,
    ctx: &SyncContext,
) -> Result<(), StorageError> {
    let db_tx = conn.unchecked_transaction()?;

    // Read the current version so the emitted event carries the resulting
    // (post-delete) entity version, keeping event.version meaningful for the
    // remote apply path's staleness check.
    let current_version: i64 = db_tx
        .query_row(
            "SELECT version FROM goals WHERE id = ?1 AND deleted = 0",
            [id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(format!("goal '{id}'")),
            other => StorageError::Database(other),
        })?;
    let new_version = current_version + 1;

    db_tx.execute(
        "UPDATE goals SET deleted = 1, version = ?1, modified_at = ?2
         WHERE id = ?3 AND deleted = 0",
        rusqlite::params![new_version, now_iso8601(), id],
    )?;

    let payload =
        crate::sync::payload::to_bytes(&crate::sync::payload::DeletePayload { id: id.to_string() })
            .map_err(|e| {
                StorageError::InvalidInput(format!("failed to serialize delete payload: {e}"))
            })?;

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let version_u32 = new_version as u32;

    super::sync::record_event(
        &db_tx,
        &ctx.device_id,
        "goal",
        id,
        "delete",
        &payload,
        ctx.lamport_clock,
        version_u32,
    )?;

    db_tx.commit()?;
    Ok(())
}

// ── Remote apply (sync ingest) ───────────────────────────────────────────────

/// Apply a remote `Create`/`Update` goal event to the canonical table.
///
/// Upserts the goal by its **explicit** `payload.id` (unlike [`create_goal`],
/// which mints a fresh id). This is the apply half of the sync pipeline: it
/// writes only the canonical table and **never** records a `sync_events` outbox
/// row, so applied remote events do not echo back into our own outbox.
///
/// Staleness rule: the event is applied iff the goal does not exist locally
/// **or** the incoming `version` is strictly greater than the local row's
/// version. An equal-or-older version is treated as already-seen and skipped.
///
/// Returns `true` if the row was written, `false` if the event was stale.
///
/// # Errors
///
/// Returns [`StorageError::InvalidInput`] if the payload's `goal_type` or
/// `target_amount` fails to parse, or [`StorageError::Database`] on write
/// failure.
pub fn apply_remote_goal(
    conn: &Connection,
    payload: &crate::sync::payload::GoalPayload,
    version: i64,
) -> Result<bool, StorageError> {
    // Validate the goal-type and decimal strings before touching the table.
    let goal_type = goal_type_from_str(&payload.goal_type)?;
    if Decimal::from_str(&payload.target_amount).is_err() {
        return Err(StorageError::InvalidInput(format!(
            "remote goal '{}' has invalid target_amount: {}",
            payload.id, payload.target_amount
        )));
    }

    if let Some(local_version) = current_goal_version(conn, &payload.id)?
        && local_version >= version
    {
        return Ok(false);
    }

    conn.execute(
        "INSERT INTO goals
             (id, name, goal_type, target_amount, target_date, linked_account,
              created_at, modified_at, version, deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             goal_type = excluded.goal_type,
             target_amount = excluded.target_amount,
             target_date = excluded.target_date,
             linked_account = excluded.linked_account,
             created_at = excluded.created_at,
             modified_at = excluded.modified_at,
             version = excluded.version,
             deleted = 0",
        rusqlite::params![
            payload.id,
            payload.name,
            goal_type_as_str(goal_type),
            payload.target_amount,
            payload.target_date,
            payload.linked_account,
            payload.created_at,
            payload.modified_at,
            version,
        ],
    )?;

    Ok(true)
}

/// Apply a remote `Delete` goal event (soft delete by explicit id).
///
/// No-op (returns `false`) if the goal is unknown locally or the incoming
/// `version` is not strictly greater than the local row's version. The
/// unknown-id no-op, combined with batch-level vector-clock dominance in the
/// merge step, prevents a stale create from resurrecting a deleted goal.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on write failure.
pub fn apply_remote_goal_delete(
    conn: &Connection,
    id: &str,
    version: i64,
) -> Result<bool, StorageError> {
    match current_goal_version(conn, id)? {
        Some(local_version) if local_version < version => {
            conn.execute(
                "UPDATE goals SET deleted = 1, version = ?1, modified_at = ?2 WHERE id = ?3",
                rusqlite::params![version, now_iso8601(), id],
            )?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Current version of a goal row (including soft-deleted rows), if it exists.
pub(crate) fn current_goal_version(
    conn: &Connection,
    id: &str,
) -> Result<Option<i64>, StorageError> {
    let mut stmt = conn.prepare("SELECT version FROM goals WHERE id = ?1")?;
    let result = stmt
        .query_row([id], |row| row.get::<_, i64>(0))
        .optional()?;
    Ok(result)
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
