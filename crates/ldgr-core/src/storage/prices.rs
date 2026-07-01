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
}
