//! `SQLite` schema definitions and migration mechanism.
//!
//! Schema versioning uses a `schema_version` table with a single row.
//! Migrations are numbered functions applied in order. All pending
//! migrations run inside a single transaction for atomicity.

use rusqlite::Connection;

use super::error::StorageError;

/// Current schema version (incremented with each migration).
const CURRENT_VERSION: u32 = 1;

/// Initialize the database schema, running any pending migrations.
///
/// This function is idempotent — safe to call on every app startup.
/// All pending migrations run inside a single transaction.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if any migration step fails.
pub fn initialize(conn: &Connection) -> Result<(), StorageError> {
    // Create schema_version table if it doesn't exist
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        );",
    )?;

    let version = current_version(conn)?;

    if version < CURRENT_VERSION {
        let tx = conn.unchecked_transaction()?;

        // Run each pending migration
        for v in (version + 1)..=CURRENT_VERSION {
            run_migration(&tx, v)?;
        }

        // Update schema version
        if version == 0 {
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [CURRENT_VERSION],
            )?;
        } else {
            tx.execute("UPDATE schema_version SET version = ?1", [CURRENT_VERSION])?;
        }

        tx.commit()?;
    }

    Ok(())
}

/// Query the current schema version. Returns 0 if no version row exists.
pub fn current_version(conn: &Connection) -> Result<u32, StorageError> {
    let mut stmt = conn.prepare("SELECT version FROM schema_version LIMIT 1")?;
    let version = stmt.query_row([], |row| row.get::<_, u32>(0)).unwrap_or(0);
    Ok(version)
}

/// Dispatch to the correct migration function by version number.
fn run_migration(conn: &Connection, version: u32) -> Result<(), StorageError> {
    match version {
        1 => migrate_v1(conn),
        _ => Err(StorageError::Database(rusqlite::Error::QueryReturnedNoRows)),
    }
}

/// Migration v1: Initial schema — core tables.
fn migrate_v1(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "
        -- Commodities (referenced by other tables)
        CREATE TABLE commodities (
            symbol TEXT PRIMARY KEY,
            name TEXT,
            decimal_places INTEGER DEFAULT 2,
            format TEXT
        );

        -- Accounts
        CREATE TABLE accounts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            type TEXT NOT NULL,
            commodity TEXT,
            parent_id TEXT REFERENCES accounts(id),
            note TEXT,
            created_at TEXT NOT NULL,
            modified_at TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            deleted INTEGER NOT NULL DEFAULT 0
        );

        -- Partial unique index: account names are unique among active rows
        CREATE UNIQUE INDEX idx_accounts_name_active
            ON accounts(name) WHERE deleted = 0;

        CREATE INDEX idx_accounts_parent ON accounts(parent_id);
        CREATE INDEX idx_accounts_type ON accounts(type);

        -- Transactions
        CREATE TABLE transactions (
            id TEXT PRIMARY KEY,
            date TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'unmarked',
            code TEXT,
            description TEXT NOT NULL,
            comment TEXT,
            created_at TEXT NOT NULL,
            modified_at TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            deleted INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX idx_transactions_date ON transactions(date);
        CREATE INDEX idx_transactions_status ON transactions(status);

        -- Postings (belong to transactions)
        CREATE TABLE postings (
            id TEXT PRIMARY KEY,
            transaction_id TEXT NOT NULL REFERENCES transactions(id),
            account_id TEXT NOT NULL REFERENCES accounts(id),
            amount_quantity TEXT,
            amount_commodity TEXT,
            balance_assertion_quantity TEXT,
            balance_assertion_commodity TEXT,
            posting_order INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 1
        );

        CREATE INDEX idx_postings_transaction ON postings(transaction_id);
        CREATE INDEX idx_postings_account ON postings(account_id);

        -- Tags (on transactions and postings)
        CREATE TABLE tags (
            id TEXT PRIMARY KEY,
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            key TEXT NOT NULL,
            value TEXT
        );

        CREATE INDEX idx_tags_entity ON tags(entity_type, entity_id);

        -- Market Prices
        CREATE TABLE prices (
            id TEXT PRIMARY KEY,
            commodity TEXT NOT NULL REFERENCES commodities(symbol),
            currency TEXT NOT NULL,
            price TEXT NOT NULL,
            date TEXT NOT NULL,
            source TEXT,
            created_at TEXT NOT NULL
        );

        CREATE INDEX idx_prices_commodity_date ON prices(commodity, date DESC);
        ",
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        Connection::open_in_memory().expect("in-memory SQLite should work")
    }

    #[test]
    fn initialize_creates_all_tables() {
        let conn = memory_db();
        initialize(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        for expected in [
            "accounts",
            "commodities",
            "postings",
            "prices",
            "schema_version",
            "tags",
            "transactions",
        ] {
            assert!(
                tables.contains(&expected.to_string()),
                "missing table: {expected}"
            );
        }
    }

    #[test]
    fn initialize_sets_version() {
        let conn = memory_db();
        initialize(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn initialize_is_idempotent() {
        let conn = memory_db();
        initialize(&conn).unwrap();
        initialize(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn empty_db_has_version_zero() {
        let conn = memory_db();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
            .unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn indexes_created() {
        let conn = memory_db();
        initialize(&conn).unwrap();

        let indexes: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        for expected in [
            "idx_accounts_name_active",
            "idx_accounts_parent",
            "idx_accounts_type",
            "idx_transactions_date",
            "idx_transactions_status",
            "idx_postings_transaction",
            "idx_postings_account",
            "idx_tags_entity",
            "idx_prices_commodity_date",
        ] {
            assert!(
                indexes.contains(&expected.to_string()),
                "missing index: {expected}"
            );
        }
    }
}
