//! Sync event outbox, conflict storage, and state management.
//!
//! All sync mutations are recorded atomically alongside the data mutations
//! they describe. The outbox tracks pending events until they are pushed
//! to the remote transport. Conflicts detected during merge are persisted
//! for user review.

use rusqlite::Connection;
use uuid::Uuid;

use super::error::StorageError;

// ── Types ──────────────────────────────────────────────────────────────────────

/// Context for recording sync events atomically with data mutations.
///
/// Pass this to `_with_sync` mutation variants to record an outbox event
/// inside the same `SQLite` transaction as the data change.
pub struct SyncContext {
    pub device_id: String,
    pub lamport_clock: u64,
}

/// A sync event stored in the outbox.
#[derive(Debug, Clone)]
pub struct StoredSyncEvent {
    pub id: String,
    pub device_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub payload: Vec<u8>,
    pub lamport_clock: u64,
    pub version: u32,
    pub created_at: String,
    pub synced: bool,
}

/// A persisted sync conflict awaiting resolution.
#[derive(Debug, Clone)]
pub struct StoredConflict {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub local_event_id: String,
    pub remote_event_id: String,
    pub local_payload: Vec<u8>,
    pub remote_payload: Vec<u8>,
    pub detected_at: String,
    pub resolved: bool,
    pub resolution: Option<String>,
}

// ── Outbox operations ──────────────────────────────────────────────────────────

/// Record a sync event in the outbox.
///
/// **Must be called within the same `SQLite` transaction** as the data mutation
/// it describes to guarantee atomicity.
#[allow(clippy::too_many_arguments)]
pub fn record_event(
    conn: &Connection,
    device_id: &str,
    entity_type: &str,
    entity_id: &str,
    operation: &str,
    payload: &[u8],
    lamport_clock: u64,
    version: u32,
) -> Result<String, StorageError> {
    let id = Uuid::now_v7().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    #[allow(clippy::cast_possible_wrap)]
    let lamport_i64 = lamport_clock as i64;

    conn.execute(
        "INSERT INTO sync_events (id, device_id, entity_type, entity_id, operation, payload, lamport_clock, version, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![id, device_id, entity_type, entity_id, operation, payload, lamport_i64, i64::from(version), now],
    )?;

    Ok(id)
}

/// Get all pending (unsynced) events, ordered by lamport clock.
pub fn pending_events(conn: &Connection) -> Result<Vec<StoredSyncEvent>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, device_id, entity_type, entity_id, operation, payload, lamport_clock, version, created_at, synced
         FROM sync_events WHERE synced = 0 ORDER BY lamport_clock ASC",
    )?;

    let events = stmt
        .query_map([], row_to_event)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(events)
}

/// Count pending (unsynced) events.
pub fn pending_event_count(conn: &Connection) -> Result<u32, StorageError> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM sync_events WHERE synced = 0",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Mark a batch of events as synced.
pub fn mark_events_synced(conn: &Connection, event_ids: &[String]) -> Result<(), StorageError> {
    if event_ids.is_empty() {
        return Ok(());
    }

    let placeholders: Vec<String> = (1..=event_ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE sync_events SET synced = 1 WHERE id IN ({})",
        placeholders.join(", ")
    );

    let params: Vec<&dyn rusqlite::types::ToSql> = event_ids
        .iter()
        .map(|id| id as &dyn rusqlite::types::ToSql)
        .collect();

    conn.execute(&sql, params.as_slice())?;
    Ok(())
}

// ── Conflict operations ────────────────────────────────────────────────────────

/// Store one or more conflicts detected during merge.
pub fn store_conflicts(
    conn: &Connection,
    conflicts: &[StoredConflict],
) -> Result<(), StorageError> {
    let mut stmt = conn.prepare(
        "INSERT INTO sync_conflicts (id, entity_type, entity_id, local_event_id, remote_event_id, local_payload, remote_payload, detected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;

    for c in conflicts {
        stmt.execute(rusqlite::params![
            c.id,
            c.entity_type,
            c.entity_id,
            c.local_event_id,
            c.remote_event_id,
            c.local_payload,
            c.remote_payload,
            c.detected_at,
        ])?;
    }

    Ok(())
}

/// List unresolved conflicts.
pub fn list_unresolved_conflicts(conn: &Connection) -> Result<Vec<StoredConflict>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, entity_type, entity_id, local_event_id, remote_event_id, local_payload, remote_payload, detected_at, resolved, resolution
         FROM sync_conflicts WHERE resolved = 0 ORDER BY detected_at ASC",
    )?;

    let conflicts = stmt
        .query_map([], row_to_conflict)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(conflicts)
}

/// Count unresolved conflicts.
pub fn unresolved_conflict_count(conn: &Connection) -> Result<u32, StorageError> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM sync_conflicts WHERE resolved = 0",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Resolve a conflict with the given resolution.
pub fn resolve_conflict(
    conn: &Connection,
    conflict_id: &str,
    resolution: &str,
) -> Result<(), StorageError> {
    let rows = conn.execute(
        "UPDATE sync_conflicts SET resolved = 1, resolution = ?1 WHERE id = ?2 AND resolved = 0",
        rusqlite::params![resolution, conflict_id],
    )?;

    if rows == 0 {
        return Err(StorageError::NotFound(format!(
            "conflict '{conflict_id}' (may already be resolved)"
        )));
    }
    Ok(())
}

// ── State operations ───────────────────────────────────────────────────────────

/// Get a sync state value by key.
pub fn get_state(conn: &Connection, key: &str) -> Result<Option<String>, StorageError> {
    let mut stmt = conn.prepare("SELECT value FROM sync_state WHERE key = ?1")?;
    let result = stmt
        .query_row([key], |row| row.get::<_, String>(0))
        .optional()?;
    Ok(result)
}

/// Set a sync state value (upsert).
pub fn set_state(conn: &Connection, key: &str, value: &str) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO sync_state (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Get the device ID, auto-generating one if it doesn't exist yet.
pub fn device_id(conn: &Connection) -> Result<String, StorageError> {
    if let Some(id) = get_state(conn, "device_id")? {
        return Ok(id);
    }

    let id = Uuid::now_v7().to_string();
    set_state(conn, "device_id", &id)?;
    Ok(id)
}

/// Get the current Lamport clock value (stored persistently).
pub fn lamport_clock(conn: &Connection) -> Result<u64, StorageError> {
    match get_state(conn, "lamport_clock")? {
        Some(v) => v
            .parse::<u64>()
            .map_err(|e| StorageError::InvalidInput(format!("corrupt lamport clock: {e}"))),
        None => Ok(0),
    }
}

/// Increment and persist the Lamport clock. Returns the new value.
pub fn tick_lamport(conn: &Connection) -> Result<u64, StorageError> {
    let current = lamport_clock(conn)?;
    let next = current + 1;
    set_state(conn, "lamport_clock", &next.to_string())?;
    Ok(next)
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSyncEvent> {
    let lamport_raw: i64 = row.get(6)?;
    let version_raw: i64 = row.get(7)?;
    Ok(StoredSyncEvent {
        id: row.get(0)?,
        device_id: row.get(1)?,
        entity_type: row.get(2)?,
        entity_id: row.get(3)?,
        operation: row.get(4)?,
        payload: row.get(5)?,
        #[allow(clippy::cast_sign_loss)]
        lamport_clock: lamport_raw as u64,
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        version: version_raw as u32,
        created_at: row.get(8)?,
        synced: row.get::<_, i64>(9)? != 0,
    })
}

fn row_to_conflict(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredConflict> {
    Ok(StoredConflict {
        id: row.get(0)?,
        entity_type: row.get(1)?,
        entity_id: row.get(2)?,
        local_event_id: row.get(3)?,
        remote_event_id: row.get(4)?,
        local_payload: row.get(5)?,
        remote_payload: row.get(6)?,
        detected_at: row.get(7)?,
        resolved: row.get::<_, i64>(8)? != 0,
        resolution: row.get(9)?,
    })
}

use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        schema::initialize(&conn).unwrap();
        conn
    }

    #[test]
    fn record_and_list_pending_events() {
        let conn = setup_db();
        let id = record_event(
            &conn,
            "dev1",
            "transaction",
            "txn1",
            "create",
            b"payload data",
            1,
            1,
        )
        .unwrap();
        assert!(!id.is_empty());

        let events = pending_events(&conn).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].entity_id, "txn1");
        assert_eq!(events[0].operation, "create");
        assert!(!events[0].synced);
    }

    #[test]
    fn mark_synced_removes_from_pending() {
        let conn = setup_db();
        let id1 =
            record_event(&conn, "dev1", "transaction", "txn1", "create", b"p1", 1, 1).unwrap();
        let _id2 = record_event(&conn, "dev1", "account", "acc1", "create", b"p2", 2, 1).unwrap();

        assert_eq!(pending_event_count(&conn).unwrap(), 2);

        mark_events_synced(&conn, &[id1]).unwrap();

        assert_eq!(pending_event_count(&conn).unwrap(), 1);
        let remaining = pending_events(&conn).unwrap();
        assert_eq!(remaining[0].entity_id, "acc1");
    }

    #[test]
    fn store_and_resolve_conflict() {
        let conn = setup_db();

        let conflict = StoredConflict {
            id: "c1".into(),
            entity_type: "transaction".into(),
            entity_id: "txn1".into(),
            local_event_id: "e1".into(),
            remote_event_id: "e2".into(),
            local_payload: b"local".to_vec(),
            remote_payload: b"remote".to_vec(),
            detected_at: "2024-01-15T00:00:00Z".into(),
            resolved: false,
            resolution: None,
        };

        store_conflicts(&conn, &[conflict]).unwrap();

        let unresolved = list_unresolved_conflicts(&conn).unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].entity_id, "txn1");

        resolve_conflict(&conn, "c1", "keep_local").unwrap();

        let after = list_unresolved_conflicts(&conn).unwrap();
        assert!(after.is_empty());
        assert_eq!(unresolved_conflict_count(&conn).unwrap(), 0);
    }

    #[test]
    fn device_id_auto_generates() {
        let conn = setup_db();
        let id1 = device_id(&conn).unwrap();
        let id2 = device_id(&conn).unwrap();
        assert_eq!(id1, id2); // idempotent
        assert!(!id1.is_empty());
    }

    #[test]
    fn lamport_clock_ticks() {
        let conn = setup_db();
        assert_eq!(lamport_clock(&conn).unwrap(), 0);
        assert_eq!(tick_lamport(&conn).unwrap(), 1);
        assert_eq!(tick_lamport(&conn).unwrap(), 2);
        assert_eq!(lamport_clock(&conn).unwrap(), 2);
    }

    #[test]
    fn sync_state_get_set() {
        let conn = setup_db();
        assert!(get_state(&conn, "foo").unwrap().is_none());
        set_state(&conn, "foo", "bar").unwrap();
        assert_eq!(get_state(&conn, "foo").unwrap().unwrap(), "bar");
        set_state(&conn, "foo", "baz").unwrap();
        assert_eq!(get_state(&conn, "foo").unwrap().unwrap(), "baz");
    }

    #[test]
    fn create_account_with_sync_records_event() {
        use crate::storage::accounts::AccountType;
        use crate::storage::accounts::{NewAccount, create_account_with_sync};

        let conn = setup_db();
        let ctx = SyncContext {
            device_id: "dev-test".into(),
            lamport_clock: 1,
        };

        let input = NewAccount {
            name: "Assets:Cash".into(),
            account_type: AccountType::Asset,
            commodity: Some("USD".into()),
            parent_id: None,
            note: None,
        };

        let account = create_account_with_sync(&conn, &input, &ctx).unwrap();
        assert_eq!(account.name, "Assets:Cash");

        let events = pending_events(&conn).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].entity_type, "account");
        assert_eq!(events[0].entity_id, account.id);
        assert_eq!(events[0].operation, "create");
        assert_eq!(events[0].device_id, "dev-test");
    }

    #[test]
    fn soft_delete_account_with_sync_records_event() {
        use crate::storage::accounts::AccountType;
        use crate::storage::accounts::{NewAccount, create_account, soft_delete_account_with_sync};

        let conn = setup_db();
        let input = NewAccount {
            name: "Assets:Bank".into(),
            account_type: AccountType::Asset,
            commodity: None,
            parent_id: None,
            note: None,
        };
        let account = create_account(&conn, &input).unwrap();

        let ctx = SyncContext {
            device_id: "dev-test".into(),
            lamport_clock: 2,
        };

        soft_delete_account_with_sync(&conn, &account.id, &ctx).unwrap();

        let events = pending_events(&conn).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].entity_type, "account");
        assert_eq!(events[0].operation, "delete");
    }

    #[test]
    fn create_transaction_with_sync_records_event() {
        use crate::storage::accounts::AccountType;
        use crate::storage::accounts::{NewAccount, create_account};
        use crate::storage::transactions::{
            NewPosting, NewTransaction, TransactionStatus, create_transaction_with_sync,
        };

        let conn = setup_db();

        let a1 = create_account(
            &conn,
            &NewAccount {
                name: "Assets:Cash".into(),
                account_type: AccountType::Asset,
                commodity: Some("USD".into()),
                parent_id: None,
                note: None,
            },
        )
        .unwrap();

        let a2 = create_account(
            &conn,
            &NewAccount {
                name: "Expenses:Food".into(),
                account_type: AccountType::Expense,
                commodity: Some("USD".into()),
                parent_id: None,
                note: None,
            },
        )
        .unwrap();

        let ctx = SyncContext {
            device_id: "dev-test".into(),
            lamport_clock: 3,
        };

        let input = NewTransaction {
            date: "2024-01-15".into(),
            status: TransactionStatus::Cleared,
            code: None,
            description: "Lunch".into(),
            comment: None,
            postings: vec![
                NewPosting {
                    account_id: a1.id.clone(),
                    amount_quantity: Some("-10.00".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                },
                NewPosting {
                    account_id: a2.id.clone(),
                    amount_quantity: Some("10.00".into()),
                    amount_commodity: Some("USD".into()),
                    balance_assertion_quantity: None,
                    balance_assertion_commodity: None,
                },
            ],
        };

        let txn = create_transaction_with_sync(&conn, &input, &ctx).unwrap();
        assert_eq!(txn.postings.len(), 2);

        let events = pending_events(&conn).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].entity_type, "transaction");
        assert_eq!(events[0].entity_id, txn.id);
        assert_eq!(events[0].operation, "create");
    }

    #[test]
    fn soft_delete_transaction_with_sync_records_event() {
        use crate::storage::accounts::AccountType;
        use crate::storage::accounts::{NewAccount, create_account};
        use crate::storage::transactions::{
            NewPosting, NewTransaction, TransactionStatus, create_transaction,
            soft_delete_transaction_with_sync,
        };

        let conn = setup_db();

        let a1 = create_account(
            &conn,
            &NewAccount {
                name: "Assets:Cash".into(),
                account_type: AccountType::Asset,
                commodity: Some("USD".into()),
                parent_id: None,
                note: None,
            },
        )
        .unwrap();
        let a2 = create_account(
            &conn,
            &NewAccount {
                name: "Expenses:Food".into(),
                account_type: AccountType::Expense,
                commodity: Some("USD".into()),
                parent_id: None,
                note: None,
            },
        )
        .unwrap();

        let txn = create_transaction(
            &conn,
            &NewTransaction {
                date: "2024-01-15".into(),
                status: TransactionStatus::Cleared,
                code: None,
                description: "Dinner".into(),
                comment: None,
                postings: vec![
                    NewPosting {
                        account_id: a1.id,
                        amount_quantity: Some("-20.00".into()),
                        amount_commodity: Some("USD".into()),
                        balance_assertion_quantity: None,
                        balance_assertion_commodity: None,
                    },
                    NewPosting {
                        account_id: a2.id,
                        amount_quantity: Some("20.00".into()),
                        amount_commodity: Some("USD".into()),
                        balance_assertion_quantity: None,
                        balance_assertion_commodity: None,
                    },
                ],
            },
        )
        .unwrap();

        let ctx = SyncContext {
            device_id: "dev-test".into(),
            lamport_clock: 4,
        };

        soft_delete_transaction_with_sync(&conn, &txn.id, &ctx).unwrap();

        let events = pending_events(&conn).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].entity_type, "transaction");
        assert_eq!(events[0].operation, "delete");
    }
}
