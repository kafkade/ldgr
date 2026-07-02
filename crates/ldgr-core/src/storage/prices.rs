//! Price CRUD operations.
//!
//! Persists an **observed price point** — a user-entered or imported price for a
//! commodity in a currency on a date — as a first-class, versioned vault entity.
//!
//! # Canonical price entity vs. the market cache
//!
//! This is the **canonical** price store: the source of truth that participates
//! in versioning, soft-delete, and (eventually) sync, mirroring
//! [`crate::storage::accounts`] and [`crate::storage::goals`]. It is deliberately
//! kept **distinct** from `market_cache` ([`crate::market::persist`]), which is a
//! transient cache of raw HTTP provider responses. The market cache answers
//! "what did the provider return recently?"; this table answers "what price does
//! the vault record for this commodity on this date?".
//!
//! A stored price is:
//!
//! ```text
//! id, commodity, currency, price, date, source?, created_at, modified_at,
//! version, deleted
//! ```
//!
//! Rows are versioned with optimistic concurrency and soft-deleted
//! (`deleted = 1`) to preserve audit history. The `price` decimal is stored as
//! TEXT for precision (never a float). `source` is optional (e.g. `"yahoo"`,
//! `"manual"`).
//!
//! This module is gated behind the `sqlite` feature only: `EntityType::Price`
//! is always present in the sync model, so persistence must not depend on the
//! `market` feature. Wiring the `market` module to write observed prices here is
//! intentionally out of scope.

use std::str::FromStr;

use rusqlite::Connection;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::error::StorageError;
use super::sync::SyncContext;

// ── Types ──────────────────────────────────────────────────────────────────────

/// A persisted price row: an observed price point plus storage metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPrice {
    /// Stable unique identifier (`UUIDv7`).
    pub id: String,
    /// The commodity being priced (e.g. `"AAPL"`).
    pub commodity: String,
    /// The currency the price is quoted in (e.g. `"USD"`).
    pub currency: String,
    /// The price of one unit of `commodity` in `currency`.
    pub price: Decimal,
    /// The date the price applies to (ISO 8601 date, e.g. `"2024-01-15"`).
    pub date: String,
    /// Optional provenance (e.g. `"yahoo"`, `"manual"`).
    pub source: Option<String>,
    pub created_at: String,
    pub modified_at: String,
    pub version: i64,
    pub deleted: bool,
}

/// Input for creating a new price.
#[derive(Debug, Clone)]
pub struct NewPrice {
    pub commodity: String,
    pub currency: String,
    pub price: Decimal,
    pub date: String,
    pub source: Option<String>,
}

/// Input for updating an existing price (full replacement of mutable fields).
#[derive(Debug, Clone)]
pub struct PriceUpdate {
    pub commodity: String,
    pub currency: String,
    pub price: Decimal,
    pub date: String,
    pub source: Option<String>,
    /// Must match the current version or the update is rejected.
    pub expected_version: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub include_deleted: bool,
}

// ── CRUD operations ────────────────────────────────────────────────────────────

/// Create a new price.
///
/// Generates a `UUIDv7` for the price ID and sets timestamps to now.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the insert fails.
pub fn create_price(conn: &Connection, input: &NewPrice) -> Result<StoredPrice, StorageError> {
    let id = Uuid::now_v7().to_string();
    let now = now_iso8601();

    conn.execute(
        "INSERT INTO prices
             (id, commodity, currency, price, date, source, created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            input.commodity,
            input.currency,
            input.price.to_string(),
            input.date,
            input.source,
            now,
            now,
        ],
    )?;

    Ok(StoredPrice {
        id,
        commodity: input.commodity.clone(),
        currency: input.currency.clone(),
        price: input.price,
        date: input.date.clone(),
        source: input.source.clone(),
        created_at: now.clone(),
        modified_at: now,
        version: 1,
        deleted: false,
    })
}

/// Get a price by ID.
///
/// Returns `None` if not found. Respects `include_deleted` in options.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on query failure, or
/// [`StorageError::InvalidInput`] if a stored value fails to parse.
pub fn get_price(
    conn: &Connection,
    id: &str,
    opts: &ListOptions,
) -> Result<Option<StoredPrice>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, commodity, currency, price, date, source,
                created_at, modified_at, version, deleted
         FROM prices WHERE id = ?1"
    } else {
        "SELECT id, commodity, currency, price, date, source,
                created_at, modified_at, version, deleted
         FROM prices WHERE id = ?1 AND deleted = 0"
    };

    let mut stmt = conn.prepare(sql)?;
    let result = stmt.query_row([id], row_to_price).optional()?;
    result.transpose()
}

/// List all prices.
///
/// Ordered by commodity, then most-recent date first.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on query failure, or
/// [`StorageError::InvalidInput`] if a stored value fails to parse.
pub fn list_prices(
    conn: &Connection,
    opts: &ListOptions,
) -> Result<Vec<StoredPrice>, StorageError> {
    let sql = if opts.include_deleted {
        "SELECT id, commodity, currency, price, date, source,
                created_at, modified_at, version, deleted
         FROM prices ORDER BY commodity, date DESC"
    } else {
        "SELECT id, commodity, currency, price, date, source,
                created_at, modified_at, version, deleted
         FROM prices WHERE deleted = 0 ORDER BY commodity, date DESC"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], row_to_price)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().collect()
}

/// Update a price's mutable fields.
///
/// Uses optimistic concurrency: the update only succeeds if the current
/// version matches `expected_version`. The version is bumped and
/// `modified_at` is set to now.
///
/// # Errors
///
/// Returns [`StorageError::Conflict`] if the version doesn't match
/// (concurrent modification or price was deleted).
pub fn update_price(
    conn: &Connection,
    id: &str,
    update: &PriceUpdate,
) -> Result<StoredPrice, StorageError> {
    let now = now_iso8601();
    let new_version = update.expected_version + 1;

    let rows = conn.execute(
        "UPDATE prices
         SET commodity = ?1, currency = ?2, price = ?3, date = ?4, source = ?5,
             modified_at = ?6, version = ?7
         WHERE id = ?8 AND version = ?9 AND deleted = 0",
        rusqlite::params![
            update.commodity,
            update.currency,
            update.price.to_string(),
            update.date,
            update.source,
            now,
            new_version,
            id,
            update.expected_version,
        ],
    )?;

    if rows == 0 {
        return Err(StorageError::Conflict(format!(
            "price '{id}' was modified or deleted (expected version {})",
            update.expected_version
        )));
    }

    // Preserve the original creation timestamp in the returned value.
    let created_at: String = conn
        .query_row("SELECT created_at FROM prices WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .unwrap_or_default();

    Ok(StoredPrice {
        id: id.to_string(),
        commodity: update.commodity.clone(),
        currency: update.currency.clone(),
        price: update.price,
        date: update.date.clone(),
        source: update.source.clone(),
        created_at,
        modified_at: now,
        version: new_version,
        deleted: false,
    })
}

/// Soft-delete a price by setting `deleted = 1`.
///
/// The row is preserved for audit history and the version is bumped.
///
/// # Errors
///
/// Returns [`StorageError::NotFound`] if the price doesn't exist or is
/// already deleted.
pub fn soft_delete_price(conn: &Connection, id: &str) -> Result<(), StorageError> {
    let rows = conn.execute(
        "UPDATE prices SET deleted = 1, version = version + 1, modified_at = ?1
         WHERE id = ?2 AND deleted = 0",
        rusqlite::params![now_iso8601(), id],
    )?;

    if rows == 0 {
        return Err(StorageError::NotFound(format!("price '{id}'")));
    }
    Ok(())
}

// ── Sync-aware variants ────────────────────────────────────────────────────────

/// Create a new price with atomic sync event recording.
///
/// Mirrors [`create_price`] but, within the same `SQLite` transaction, also
/// records a `create` outbox event carrying the full [`PricePayload`] so the
/// change propagates on the next `sync push`.
pub fn create_price_with_sync(
    conn: &Connection,
    input: &NewPrice,
    ctx: &SyncContext,
) -> Result<StoredPrice, StorageError> {
    let db_tx = conn.unchecked_transaction()?;

    let id = Uuid::now_v7().to_string();
    let now = now_iso8601();

    db_tx.execute(
        "INSERT INTO prices
             (id, commodity, currency, price, date, source, created_at, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            input.commodity,
            input.currency,
            input.price.to_string(),
            input.date,
            input.source,
            now,
            now,
        ],
    )?;

    let payload = crate::sync::payload::to_bytes(&crate::sync::payload::PricePayload {
        id: id.clone(),
        commodity: input.commodity.clone(),
        currency: input.currency.clone(),
        price: input.price.to_string(),
        date: input.date.clone(),
        source: input.source.clone(),
        created_at: now.clone(),
        modified_at: now.clone(),
    })
    .map_err(|e| StorageError::InvalidInput(format!("failed to serialize price payload: {e}")))?;

    super::sync::record_event(
        &db_tx,
        &ctx.device_id,
        "price",
        &id,
        "create",
        &payload,
        ctx.lamport_clock,
        1,
    )?;

    db_tx.commit()?;

    Ok(StoredPrice {
        id,
        commodity: input.commodity.clone(),
        currency: input.currency.clone(),
        price: input.price,
        date: input.date.clone(),
        source: input.source.clone(),
        created_at: now.clone(),
        modified_at: now,
        version: 1,
        deleted: false,
    })
}

/// Update a price's mutable fields with atomic sync event recording.
///
/// Mirrors [`update_price`] but, within the same `SQLite` transaction, also
/// records an `update` outbox event carrying the full post-update
/// [`PricePayload`] so the change propagates on the next `sync push`. The
/// emitted `event.version` is the resulting row version so the remote apply
/// path's staleness gate works.
pub fn update_price_with_sync(
    conn: &Connection,
    id: &str,
    update: &PriceUpdate,
    ctx: &SyncContext,
) -> Result<StoredPrice, StorageError> {
    let db_tx = conn.unchecked_transaction()?;

    let now = now_iso8601();
    let new_version = update.expected_version + 1;

    // Preserve the original creation timestamp so the emitted payload (and any
    // remote upsert it drives) keeps `created_at` stable across the update.
    let created_at: String = db_tx
        .query_row(
            "SELECT created_at FROM prices WHERE id = ?1 AND deleted = 0",
            [id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::Conflict(format!(
                "price '{id}' was modified or deleted (expected version {})",
                update.expected_version
            )),
            other => StorageError::Database(other),
        })?;

    let rows = db_tx.execute(
        "UPDATE prices
         SET commodity = ?1, currency = ?2, price = ?3, date = ?4, source = ?5,
             modified_at = ?6, version = ?7
         WHERE id = ?8 AND version = ?9 AND deleted = 0",
        rusqlite::params![
            update.commodity,
            update.currency,
            update.price.to_string(),
            update.date,
            update.source,
            now,
            new_version,
            id,
            update.expected_version,
        ],
    )?;

    if rows == 0 {
        return Err(StorageError::Conflict(format!(
            "price '{id}' was modified or deleted (expected version {})",
            update.expected_version
        )));
    }

    let payload = crate::sync::payload::to_bytes(&crate::sync::payload::PricePayload {
        id: id.to_string(),
        commodity: update.commodity.clone(),
        currency: update.currency.clone(),
        price: update.price.to_string(),
        date: update.date.clone(),
        source: update.source.clone(),
        created_at: created_at.clone(),
        modified_at: now.clone(),
    })
    .map_err(|e| StorageError::InvalidInput(format!("failed to serialize price payload: {e}")))?;

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let version_u32 = new_version as u32;

    super::sync::record_event(
        &db_tx,
        &ctx.device_id,
        "price",
        id,
        "update",
        &payload,
        ctx.lamport_clock,
        version_u32,
    )?;

    db_tx.commit()?;

    Ok(StoredPrice {
        id: id.to_string(),
        commodity: update.commodity.clone(),
        currency: update.currency.clone(),
        price: update.price,
        date: update.date.clone(),
        source: update.source.clone(),
        created_at,
        modified_at: now,
        version: new_version,
        deleted: false,
    })
}

/// Soft-delete a price with atomic sync event recording.
///
/// Mirrors [`soft_delete_price`] but, within the same `SQLite` transaction,
/// also records a `delete` outbox event. The emitted `event.version` is the
/// resulting (post-delete) row version so the remote apply path's staleness
/// check stays meaningful.
pub fn soft_delete_price_with_sync(
    conn: &Connection,
    id: &str,
    ctx: &SyncContext,
) -> Result<(), StorageError> {
    let db_tx = conn.unchecked_transaction()?;

    // Read the current version so the emitted event carries the resulting
    // (post-delete) entity version.
    let current_version: i64 = db_tx
        .query_row(
            "SELECT version FROM prices WHERE id = ?1 AND deleted = 0",
            [id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(format!("price '{id}'")),
            other => StorageError::Database(other),
        })?;
    let new_version = current_version + 1;

    db_tx.execute(
        "UPDATE prices SET deleted = 1, version = ?1, modified_at = ?2
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
        "price",
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

/// Apply a remote `Create`/`Update` price event to the canonical table.
///
/// Upserts the price by its **explicit** `payload.id` (unlike [`create_price`],
/// which mints a fresh id). This is the apply half of the sync pipeline: it
/// writes only the canonical table and **never** records a `sync_events` outbox
/// row, so applied remote events do not echo back into our own outbox.
///
/// Staleness rule: the event is applied iff the price does not exist locally
/// **or** the incoming `version` is strictly greater than the local row's
/// version. An equal-or-older version is treated as already-seen and skipped.
///
/// Returns `true` if the row was written, `false` if the event was stale.
///
/// # Errors
///
/// Returns [`StorageError::InvalidInput`] if `payload.price` is not a valid
/// decimal, or [`StorageError::Database`] on write failure.
pub fn apply_remote_price(
    conn: &Connection,
    payload: &crate::sync::payload::PricePayload,
    version: i64,
) -> Result<bool, StorageError> {
    // Validate the decimal string before touching the table.
    if Decimal::from_str(&payload.price).is_err() {
        return Err(StorageError::InvalidInput(format!(
            "remote price '{}' has invalid price: {}",
            payload.id, payload.price
        )));
    }

    if let Some(local_version) = current_price_version(conn, &payload.id)?
        && local_version >= version
    {
        return Ok(false);
    }

    // Commodities are not a synced entity, so a remote price may reference a
    // symbol this device has never seen. Register the bare symbol defensively
    // (idempotent) so the price row's `commodity` foreign key is satisfied;
    // local commodity management can enrich name/format later.
    conn.execute(
        "INSERT OR IGNORE INTO commodities (symbol) VALUES (?1)",
        rusqlite::params![payload.commodity],
    )?;

    conn.execute(
        "INSERT INTO prices
             (id, commodity, currency, price, date, source, created_at, modified_at, version, deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)
         ON CONFLICT(id) DO UPDATE SET
             commodity = excluded.commodity,
             currency = excluded.currency,
             price = excluded.price,
             date = excluded.date,
             source = excluded.source,
             created_at = excluded.created_at,
             modified_at = excluded.modified_at,
             version = excluded.version,
             deleted = 0",
        rusqlite::params![
            payload.id,
            payload.commodity,
            payload.currency,
            payload.price,
            payload.date,
            payload.source,
            payload.created_at,
            payload.modified_at,
            version,
        ],
    )?;

    Ok(true)
}

/// Apply a remote `Delete` price event (soft delete by explicit id).
///
/// No-op (returns `false`) if the price is unknown locally or the incoming
/// `version` is not strictly greater than the local row's version.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on write failure.
pub fn apply_remote_price_delete(
    conn: &Connection,
    id: &str,
    version: i64,
) -> Result<bool, StorageError> {
    match current_price_version(conn, id)? {
        Some(local_version) if local_version < version => {
            conn.execute(
                "UPDATE prices SET deleted = 1, version = ?1, modified_at = ?2 WHERE id = ?3",
                rusqlite::params![version, now_iso8601(), id],
            )?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Current version of a price row (including soft-deleted rows), if it exists.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on query failure.
pub fn current_price_version(conn: &Connection, id: &str) -> Result<Option<i64>, StorageError> {
    let mut stmt = conn.prepare("SELECT version FROM prices WHERE id = ?1")?;
    let result = stmt
        .query_row([id], |row| row.get::<_, i64>(0))
        .optional()?;
    Ok(result)
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Map a row to a [`StoredPrice`], parsing the stored decimal. The outer
/// `rusqlite::Result` covers column access; the inner
/// `Result<StoredPrice, StorageError>` covers value parsing.
fn row_to_price(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<StoredPrice, StorageError>> {
    let id: String = row.get(0)?;
    let commodity: String = row.get(1)?;
    let currency: String = row.get(2)?;
    let price_str: String = row.get(3)?;
    let date: String = row.get(4)?;
    let source: Option<String> = row.get(5)?;
    let created_at: String = row.get(6)?;
    let modified_at: String = row.get(7)?;
    let version: i64 = row.get(8)?;
    let deleted: bool = row.get::<_, i64>(9)? != 0;

    let Ok(price) = Decimal::from_str(&price_str) else {
        return Ok(Err(StorageError::InvalidInput(format!(
            "price '{id}' has invalid price: {price_str}"
        ))));
    };

    Ok(Ok(StoredPrice {
        id,
        commodity,
        currency,
        price,
        date,
        source,
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
        // Satisfy the prices.commodity foreign key (mirrors how the
        // transactions tests insert referenced accounts for posting FKs).
        for symbol in ["AAPL", "BTC", "XAU", "MSFT"] {
            conn.execute("INSERT INTO commodities (symbol) VALUES (?1)", [symbol])
                .unwrap();
        }
        conn
    }

    fn sample_price() -> NewPrice {
        NewPrice {
            commodity: "AAPL".into(),
            currency: "USD".into(),
            price: Decimal::new(18550, 2),
            date: "2024-01-15".into(),
            source: Some("yahoo".into()),
        }
    }

    // --- Create ---

    #[test]
    fn create_price_succeeds() {
        let conn = setup();
        let p = create_price(&conn, &sample_price()).unwrap();

        assert!(!p.id.is_empty());
        assert_eq!(p.commodity, "AAPL");
        assert_eq!(p.currency, "USD");
        assert_eq!(p.price, Decimal::new(18550, 2));
        assert_eq!(p.date, "2024-01-15");
        assert_eq!(p.source.as_deref(), Some("yahoo"));
        assert_eq!(p.version, 1);
        assert!(!p.deleted);
    }

    #[test]
    fn create_price_with_source_none() {
        let conn = setup();
        let p = create_price(
            &conn,
            &NewPrice {
                commodity: "BTC".into(),
                currency: "EUR".into(),
                price: Decimal::new(60000, 0),
                date: "2024-02-01".into(),
                source: None,
            },
        )
        .unwrap();

        assert!(p.source.is_none());
    }

    // --- Get ---

    #[test]
    fn get_price_round_trip() {
        let conn = setup();
        let created = create_price(&conn, &sample_price()).unwrap();

        let fetched = get_price(&conn, &created.id, &ListOptions::default())
            .unwrap()
            .unwrap();

        assert_eq!(created, fetched);
    }

    #[test]
    fn get_price_missing_returns_none() {
        let conn = setup();
        let result = get_price(&conn, "nonexistent", &ListOptions::default()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn get_price_preserves_decimal_precision() {
        let conn = setup();
        let created = create_price(
            &conn,
            &NewPrice {
                commodity: "XAU".into(),
                currency: "USD".into(),
                price: Decimal::from_str("2034.123456789").unwrap(),
                date: "2024-03-01".into(),
                source: None,
            },
        )
        .unwrap();

        let fetched = get_price(&conn, &created.id, &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(fetched.price, Decimal::from_str("2034.123456789").unwrap());
    }

    // --- List ---

    #[test]
    fn list_prices_empty() {
        let conn = setup();
        assert!(
            list_prices(&conn, &ListOptions::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn list_prices_orders_by_commodity_then_date_desc() {
        let conn = setup();
        create_price(
            &conn,
            &NewPrice {
                commodity: "AAPL".into(),
                currency: "USD".into(),
                price: Decimal::new(100, 0),
                date: "2024-01-01".into(),
                source: None,
            },
        )
        .unwrap();
        create_price(
            &conn,
            &NewPrice {
                commodity: "AAPL".into(),
                currency: "USD".into(),
                price: Decimal::new(110, 0),
                date: "2024-02-01".into(),
                source: None,
            },
        )
        .unwrap();
        create_price(
            &conn,
            &NewPrice {
                commodity: "MSFT".into(),
                currency: "USD".into(),
                price: Decimal::new(400, 0),
                date: "2024-01-01".into(),
                source: None,
            },
        )
        .unwrap();

        let prices = list_prices(&conn, &ListOptions::default()).unwrap();
        assert_eq!(prices.len(), 3);
        assert_eq!(prices[0].commodity, "AAPL");
        assert_eq!(prices[0].date, "2024-02-01"); // most recent first
        assert_eq!(prices[1].commodity, "AAPL");
        assert_eq!(prices[1].date, "2024-01-01");
        assert_eq!(prices[2].commodity, "MSFT");
    }

    // --- Update / version bump ---

    #[test]
    fn update_price_bumps_version() {
        let conn = setup();
        let created = create_price(&conn, &sample_price()).unwrap();

        let updated = update_price(
            &conn,
            &created.id,
            &PriceUpdate {
                commodity: "AAPL".into(),
                currency: "USD".into(),
                price: Decimal::new(19000, 2),
                date: "2024-01-16".into(),
                source: Some("manual".into()),
                expected_version: 1,
            },
        )
        .unwrap();

        assert_eq!(updated.version, 2);
        assert_eq!(updated.price, Decimal::new(19000, 2));
        assert_eq!(updated.date, "2024-01-16");
        assert_eq!(updated.source.as_deref(), Some("manual"));
        assert_eq!(updated.created_at, created.created_at);
    }

    #[test]
    fn update_price_version_conflict() {
        let conn = setup();
        let created = create_price(&conn, &sample_price()).unwrap();

        let err = update_price(
            &conn,
            &created.id,
            &PriceUpdate {
                commodity: "AAPL".into(),
                currency: "USD".into(),
                price: Decimal::new(19000, 2),
                date: "2024-01-16".into(),
                source: None,
                expected_version: 99, // wrong
            },
        )
        .unwrap_err();

        assert!(matches!(err, StorageError::Conflict(_)));
    }

    #[test]
    fn update_price_missing_is_conflict() {
        let conn = setup();
        let err = update_price(
            &conn,
            "nonexistent",
            &PriceUpdate {
                commodity: "AAPL".into(),
                currency: "USD".into(),
                price: Decimal::new(1, 0),
                date: "2024-01-01".into(),
                source: None,
                expected_version: 1,
            },
        )
        .unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

    // --- Soft delete ---

    #[test]
    fn soft_delete_price_hides_from_default_queries() {
        let conn = setup();
        let created = create_price(&conn, &sample_price()).unwrap();

        soft_delete_price(&conn, &created.id).unwrap();

        // Hidden from default get/list.
        assert!(
            get_price(&conn, &created.id, &ListOptions::default())
                .unwrap()
                .is_none()
        );
        assert!(
            list_prices(&conn, &ListOptions::default())
                .unwrap()
                .is_empty()
        );

        // Visible with include_deleted, version bumped, deleted flag set.
        let opts = ListOptions {
            include_deleted: true,
        };
        let fetched = get_price(&conn, &created.id, &opts).unwrap().unwrap();
        assert!(fetched.deleted);
        assert_eq!(fetched.version, 2);
    }

    #[test]
    fn soft_delete_missing_returns_not_found() {
        let conn = setup();
        let err = soft_delete_price(&conn, "nonexistent").unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    #[test]
    fn soft_delete_twice_returns_not_found() {
        let conn = setup();
        let created = create_price(&conn, &sample_price()).unwrap();
        soft_delete_price(&conn, &created.id).unwrap();
        let err = soft_delete_price(&conn, &created.id).unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    #[test]
    fn update_deleted_price_is_conflict() {
        let conn = setup();
        let created = create_price(&conn, &sample_price()).unwrap();
        soft_delete_price(&conn, &created.id).unwrap();

        let err = update_price(
            &conn,
            &created.id,
            &PriceUpdate {
                commodity: "AAPL".into(),
                currency: "USD".into(),
                price: Decimal::new(1, 0),
                date: "2024-01-01".into(),
                source: None,
                expected_version: 2,
            },
        )
        .unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

    // --- Sync emitters ---

    fn sync_ctx(device: &str, lamport: u64) -> SyncContext {
        SyncContext {
            device_id: device.into(),
            lamport_clock: lamport,
        }
    }

    #[test]
    fn create_price_with_sync_emits_create_event() {
        use crate::storage::sync::pending_events;

        let conn = setup();
        let p = create_price_with_sync(&conn, &sample_price(), &sync_ctx("dev_a", 1)).unwrap();

        assert_eq!(p.version, 1);
        let pending = pending_events(&conn).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].entity_type, "price");
        assert_eq!(pending[0].entity_id, p.id);
        assert_eq!(pending[0].operation, "create");
        assert_eq!(pending[0].version, 1);
    }

    #[test]
    fn update_price_with_sync_emits_update_event() {
        use crate::storage::sync::pending_events;

        let conn = setup();
        let created =
            create_price_with_sync(&conn, &sample_price(), &sync_ctx("dev_a", 1)).unwrap();

        let updated = update_price_with_sync(
            &conn,
            &created.id,
            &PriceUpdate {
                commodity: "AAPL".into(),
                currency: "USD".into(),
                price: Decimal::new(19000, 2),
                date: "2024-01-16".into(),
                source: Some("manual".into()),
                expected_version: created.version,
            },
            &sync_ctx("dev_a", 2),
        )
        .unwrap();

        assert_eq!(updated.version, created.version + 1);
        assert_eq!(updated.created_at, created.created_at);

        let pending = pending_events(&conn).unwrap();
        assert_eq!(pending.len(), 2);
        let update_ev = pending
            .iter()
            .find(|e| e.operation == "update")
            .expect("update event recorded");
        assert_eq!(update_ev.entity_type, "price");
        assert_eq!(update_ev.entity_id, created.id);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let expected_version = updated.version as u32;
        assert_eq!(update_ev.version, expected_version);
    }

    #[test]
    fn update_price_with_sync_version_conflict_emits_nothing() {
        use crate::storage::sync::pending_event_count;

        let conn = setup();
        let created =
            create_price_with_sync(&conn, &sample_price(), &sync_ctx("dev_a", 1)).unwrap();

        let result = update_price_with_sync(
            &conn,
            &created.id,
            &PriceUpdate {
                commodity: "AAPL".into(),
                currency: "USD".into(),
                price: Decimal::new(1, 0),
                date: "2024-01-16".into(),
                source: None,
                expected_version: created.version + 99,
            },
            &sync_ctx("dev_a", 2),
        );
        assert!(matches!(result, Err(StorageError::Conflict(_))));

        // Only the original create event remains; the failed update rolled back.
        assert_eq!(pending_event_count(&conn).unwrap(), 1);
    }

    #[test]
    fn soft_delete_price_with_sync_emits_delete_event() {
        use crate::storage::sync::pending_events;

        let conn = setup();
        let created =
            create_price_with_sync(&conn, &sample_price(), &sync_ctx("dev_a", 1)).unwrap();

        soft_delete_price_with_sync(&conn, &created.id, &sync_ctx("dev_a", 2)).unwrap();

        let pending = pending_events(&conn).unwrap();
        let delete_ev = pending
            .iter()
            .find(|e| e.operation == "delete")
            .expect("delete event recorded");
        assert_eq!(delete_ev.entity_type, "price");
        assert_eq!(delete_ev.entity_id, created.id);
        assert_eq!(delete_ev.version, 2); // post-delete version

        // Row is hidden from default queries.
        assert!(
            get_price(&conn, &created.id, &ListOptions::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn soft_delete_price_with_sync_missing_emits_nothing() {
        use crate::storage::sync::pending_event_count;

        let conn = setup();
        let err =
            soft_delete_price_with_sync(&conn, "nonexistent", &sync_ctx("dev_a", 1)).unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
        assert_eq!(pending_event_count(&conn).unwrap(), 0);
    }

    // --- Remote apply ---

    fn price_payload(id: &str, commodity: &str, price: &str) -> crate::sync::payload::PricePayload {
        crate::sync::payload::PricePayload {
            id: id.into(),
            commodity: commodity.into(),
            currency: "USD".into(),
            price: price.into(),
            date: "2024-01-15".into(),
            source: Some("yahoo".into()),
            created_at: "2024-01-15T00:00:00Z".into(),
            modified_at: "2024-01-15T00:00:00Z".into(),
        }
    }

    #[test]
    fn apply_remote_price_inserts_new_row() {
        let conn = setup();
        let applied = apply_remote_price(&conn, &price_payload("p1", "AAPL", "185.50"), 1).unwrap();
        assert!(applied);

        let opts = ListOptions {
            include_deleted: true,
        };
        let stored = get_price(&conn, "p1", &opts).unwrap().unwrap();
        assert_eq!(stored.price, Decimal::from_str("185.50").unwrap());
        assert_eq!(stored.version, 1);
    }

    #[test]
    fn apply_remote_price_registers_unknown_commodity() {
        let conn = setup();
        // "DOGE" is not seeded in setup(); apply must register it defensively.
        let applied = apply_remote_price(&conn, &price_payload("p1", "DOGE", "0.12"), 1).unwrap();
        assert!(applied);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM commodities WHERE symbol = 'DOGE'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn apply_remote_price_upserts_newer_version() {
        let conn = setup();
        apply_remote_price(&conn, &price_payload("p1", "AAPL", "185.50"), 1).unwrap();

        let applied = apply_remote_price(&conn, &price_payload("p1", "AAPL", "200.00"), 2).unwrap();
        assert!(applied);

        let stored = get_price(&conn, "p1", &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(stored.price, Decimal::from_str("200.00").unwrap());
        assert_eq!(stored.version, 2);
    }

    #[test]
    fn apply_remote_price_skips_stale_version() {
        let conn = setup();
        apply_remote_price(&conn, &price_payload("p1", "AAPL", "200.00"), 2).unwrap();

        // Equal or older version is treated as already-seen.
        let applied = apply_remote_price(&conn, &price_payload("p1", "AAPL", "185.50"), 2).unwrap();
        assert!(!applied);
        let stored = get_price(&conn, "p1", &ListOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(stored.price, Decimal::from_str("200.00").unwrap());
    }

    #[test]
    fn apply_remote_price_rejects_invalid_decimal() {
        let conn = setup();
        let err =
            apply_remote_price(&conn, &price_payload("p1", "AAPL", "not-a-number"), 1).unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)));
    }

    #[test]
    fn apply_remote_price_delete_soft_deletes_newer() {
        let conn = setup();
        apply_remote_price(&conn, &price_payload("p1", "AAPL", "185.50"), 1).unwrap();

        let applied = apply_remote_price_delete(&conn, "p1", 2).unwrap();
        assert!(applied);
        assert!(
            get_price(&conn, "p1", &ListOptions::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn apply_remote_price_delete_unknown_is_noop() {
        let conn = setup();
        let applied = apply_remote_price_delete(&conn, "nonexistent", 1).unwrap();
        assert!(!applied);
    }

    #[test]
    fn apply_remote_price_delete_stale_is_noop() {
        let conn = setup();
        apply_remote_price(&conn, &price_payload("p1", "AAPL", "185.50"), 3).unwrap();

        // Delete at an older version does not touch the row.
        let applied = apply_remote_price_delete(&conn, "p1", 2).unwrap();
        assert!(!applied);
        assert!(
            get_price(&conn, "p1", &ListOptions::default())
                .unwrap()
                .is_some()
        );
    }
}
