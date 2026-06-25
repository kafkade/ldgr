//! `SQLite` persistence for the market data cache (ADR-007, Layer 1).
//!
//! The in-memory [`MarketCache`](super::cache::MarketCache) is the read layer;
//! this module is the persistence layer that lets cached prices survive across
//! CLI invocations and app restarts.
//!
//! The cache stores only **public market data** (symbols and prices) — never any
//! user financial data — so it lives in its own `SQLite` database, separate from
//! the encrypted vault. This keeps it usable by standalone commands (e.g.
//! `ldgr watch`) that never unlock a vault.
//!
//! Gated behind the `sqlite` feature so it is excluded from the WASM `core` bundle.

use std::time::Duration;

use rusqlite::Connection;
use thiserror::Error;

use super::cache::{CacheEntry, MarketCache};

/// Errors that can occur in the persistent cache layer.
#[derive(Debug, Error)]
pub enum CacheStoreError {
    /// Underlying `SQLite` error.
    #[error("cache database error: {0}")]
    Database(#[from] rusqlite::Error),
}

/// Initialize the cache schema. Idempotent — safe to call on every startup.
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] if the schema cannot be created.
pub fn initialize(conn: &Connection) -> Result<(), CacheStoreError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS market_cache (
            key       TEXT PRIMARY KEY,
            data      TEXT NOT NULL,
            stored_at INTEGER NOT NULL,
            ttl_secs  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS market_cache_stats (
            id     INTEGER PRIMARY KEY CHECK (id = 1),
            hits   INTEGER NOT NULL DEFAULT 0,
            misses INTEGER NOT NULL DEFAULT 0
        );

        INSERT OR IGNORE INTO market_cache_stats (id, hits, misses) VALUES (1, 0, 0);
        ",
    )?;
    Ok(())
}

/// Load every entry (including expired) for hydrating a [`MarketCache`].
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on query failure.
pub fn load_all(conn: &Connection) -> Result<Vec<(String, CacheEntry)>, CacheStoreError> {
    let mut stmt =
        conn.prepare("SELECT key, data, stored_at, ttl_secs FROM market_cache ORDER BY key")?;
    let rows = stmt.query_map([], |row| {
        let key: String = row.get(0)?;
        let entry = CacheEntry {
            data: row.get(1)?,
            stored_at: row.get(2)?,
            ttl_secs: row.get::<_, i64>(3)?.unsigned_abs(),
        };
        Ok((key, entry))
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Load only entries that are still fresh at `now`.
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on query failure.
pub fn load_fresh(
    conn: &Connection,
    now: i64,
) -> Result<Vec<(String, CacheEntry)>, CacheStoreError> {
    Ok(load_all(conn)?
        .into_iter()
        .filter(|(_, e)| e.is_fresh(now))
        .collect())
}

/// Write a single entry through to the cache database (`INSERT OR REPLACE`).
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on write failure.
#[allow(clippy::cast_possible_wrap)]
pub fn upsert(conn: &Connection, key: &str, entry: &CacheEntry) -> Result<(), CacheStoreError> {
    conn.execute(
        "INSERT OR REPLACE INTO market_cache (key, data, stored_at, ttl_secs)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![key, entry.data, entry.stored_at, entry.ttl_secs as i64],
    )?;
    Ok(())
}

/// Delete expired entries. Returns the number of rows removed.
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on write failure.
pub fn evict_expired(conn: &Connection, now: i64) -> Result<usize, CacheStoreError> {
    let removed = conn.execute(
        "DELETE FROM market_cache WHERE stored_at + ttl_secs <= ?1",
        [now],
    )?;
    Ok(removed)
}

/// Delete all cached entries. Returns the number of rows removed.
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on write failure.
pub fn clear(conn: &Connection) -> Result<usize, CacheStoreError> {
    let removed = conn.execute("DELETE FROM market_cache", [])?;
    Ok(removed)
}

/// Total number of entries (including expired).
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on query failure.
pub fn count(conn: &Connection) -> Result<usize, CacheStoreError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM market_cache", [], |row| row.get(0))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// Number of entries that are still fresh at `now`.
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on query failure.
pub fn count_fresh(conn: &Connection, now: i64) -> Result<usize, CacheStoreError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM market_cache WHERE stored_at + ttl_secs > ?1",
        [now],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// Cumulative cache hit / miss counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

impl CacheStats {
    /// Hit rate in the range `0.0..=1.0`. Returns `0.0` when there is no traffic.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Record a cache hit.
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on write failure.
pub fn record_hit(conn: &Connection) -> Result<(), CacheStoreError> {
    conn.execute(
        "UPDATE market_cache_stats SET hits = hits + 1 WHERE id = 1",
        [],
    )?;
    Ok(())
}

/// Record a cache miss.
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on write failure.
pub fn record_miss(conn: &Connection) -> Result<(), CacheStoreError> {
    conn.execute(
        "UPDATE market_cache_stats SET misses = misses + 1 WHERE id = 1",
        [],
    )?;
    Ok(())
}

/// Read the cumulative hit / miss counters.
///
/// # Errors
///
/// Returns [`CacheStoreError::Database`] on query failure.
pub fn stats(conn: &Connection) -> Result<CacheStats, CacheStoreError> {
    let (hits, misses): (i64, i64) = conn.query_row(
        "SELECT hits, misses FROM market_cache_stats WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(CacheStats {
        hits: hits.unsigned_abs(),
        misses: misses.unsigned_abs(),
    })
}

/// A snapshot of cache state for `ldgr cache status`.
#[derive(Debug, Clone, Copy)]
pub struct CacheStatus {
    /// Total entries stored (including expired).
    pub total_entries: usize,
    /// Entries still within their TTL.
    pub fresh_entries: usize,
    /// Cumulative hit / miss counters.
    pub stats: CacheStats,
}

/// A persistent, two-level market cache.
///
/// Owns a `SQLite` [`Connection`] (the persistence layer) and an in-memory
/// [`MarketCache`] (the read layer). On [`open`](PersistentCache::open) the
/// in-memory layer is hydrated from fresh rows and expired rows are evicted.
pub struct PersistentCache {
    conn: Connection,
    mem: MarketCache,
}

impl PersistentCache {
    /// Open (or create) the cache database at `path`, hydrate fresh entries, and
    /// evict expired ones.
    ///
    /// `now` is the current Unix timestamp, used for the initial eviction and
    /// hydration so that only fresh entries are loaded into memory.
    ///
    /// # Errors
    ///
    /// Returns [`CacheStoreError::Database`] if the database cannot be opened or
    /// initialized.
    pub fn open(path: &std::path::Path, now: i64) -> Result<Self, CacheStoreError> {
        let conn = Connection::open(path)?;
        initialize(&conn)?;
        evict_expired(&conn, now)?;

        let mut mem = MarketCache::new();
        for (key, entry) in load_fresh(&conn, now)? {
            mem.restore(key, entry);
        }

        Ok(Self { conn, mem })
    }

    /// Open an in-memory cache database (primarily for tests).
    ///
    /// # Errors
    ///
    /// Returns [`CacheStoreError::Database`] if initialization fails.
    pub fn open_in_memory() -> Result<Self, CacheStoreError> {
        let conn = Connection::open_in_memory()?;
        initialize(&conn)?;
        Ok(Self {
            conn,
            mem: MarketCache::new(),
        })
    }

    /// Look up a fresh entry, recording a hit or miss.
    ///
    /// Returns the cached data string on a fresh hit.
    ///
    /// # Errors
    ///
    /// Returns [`CacheStoreError::Database`] if updating the stats fails.
    pub fn get(&mut self, key: &str, now: i64) -> Result<Option<String>, CacheStoreError> {
        if let Some(entry) = self.mem.get(key, now) {
            let data = entry.data.clone();
            record_hit(&self.conn)?;
            Ok(Some(data))
        } else {
            record_miss(&self.conn)?;
            Ok(None)
        }
    }

    /// Store an entry in both the in-memory and persistent layers (write-through).
    ///
    /// # Errors
    ///
    /// Returns [`CacheStoreError::Database`] if the write fails.
    pub fn set(
        &mut self,
        key: String,
        data: String,
        ttl: Duration,
        now: i64,
    ) -> Result<(), CacheStoreError> {
        let entry = CacheEntry {
            data,
            stored_at: now,
            ttl_secs: ttl.as_secs(),
        };
        upsert(&self.conn, &key, &entry)?;
        self.mem.restore(key, entry);
        Ok(())
    }

    /// Flush all cached entries from both layers. Returns the number removed.
    ///
    /// # Errors
    ///
    /// Returns [`CacheStoreError::Database`] if the delete fails.
    pub fn clear(&mut self) -> Result<usize, CacheStoreError> {
        let removed = clear(&self.conn)?;
        self.mem = MarketCache::new();
        Ok(removed)
    }

    /// Snapshot of cache state for status reporting.
    ///
    /// # Errors
    ///
    /// Returns [`CacheStoreError::Database`] on query failure.
    pub fn status(&self, now: i64) -> Result<CacheStatus, CacheStoreError> {
        Ok(CacheStatus {
            total_entries: count(&self.conn)?,
            fresh_entries: count_fresh(&self.conn, now)?,
            stats: stats(&self.conn)?,
        })
    }

    /// Borrow the underlying connection (e.g. for additional queries).
    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::cache::QUOTE_TTL;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let unique = format!(
            "ldgr-cache-test-{}-{}-{}.db",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        p.push(unique);
        p
    }

    #[test]
    fn upsert_and_load_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        let entry = CacheEntry {
            data: "quotes-json".into(),
            stored_at: 1000,
            ttl_secs: 900,
        };
        upsert(&conn, "quote:yahoo:AAPL", &entry).unwrap();

        let all = load_all(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "quote:yahoo:AAPL");
        assert_eq!(all[0].1.data, "quotes-json");
        assert_eq!(all[0].1.stored_at, 1000);
        assert_eq!(all[0].1.ttl_secs, 900);
    }

    #[test]
    fn survives_reopen() {
        let path = temp_path("reopen");
        {
            let cache = PersistentCache::open(&path, 1000);
            let mut cache = cache.unwrap();
            cache
                .set("quote:yahoo:MSFT".into(), "data".into(), QUOTE_TTL, 1000)
                .unwrap();
        }

        // Reopen with a fresh connection — simulates a CLI restart.
        let mut reopened = PersistentCache::open(&path, 1100).unwrap();
        let hit = reopened.get("quote:yahoo:MSFT", 1100).unwrap();
        assert_eq!(hit.as_deref(), Some("data"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn expired_entries_not_loaded_or_returned() {
        let path = temp_path("expired");
        {
            let mut cache = PersistentCache::open(&path, 1000).unwrap();
            // 10-second TTL stored at t=1000.
            cache
                .set("stale".into(), "old".into(), Duration::from_secs(10), 1000)
                .unwrap();
        }

        // Reopen well past expiry — entry should be evicted, not hydrated.
        let mut reopened = PersistentCache::open(&path, 5000).unwrap();
        assert_eq!(reopened.get("stale", 5000).unwrap(), None);
        assert_eq!(count(reopened.connection()).unwrap(), 0);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_fresh_filters_expired() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        upsert(
            &conn,
            "fresh",
            &CacheEntry {
                data: "a".into(),
                stored_at: 1000,
                ttl_secs: 900,
            },
        )
        .unwrap();
        upsert(
            &conn,
            "stale",
            &CacheEntry {
                data: "b".into(),
                stored_at: 1000,
                ttl_secs: 10,
            },
        )
        .unwrap();

        let fresh = load_fresh(&conn, 1050).unwrap();
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].0, "fresh");
    }

    #[test]
    fn evict_and_clear() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();

        upsert(
            &conn,
            "fresh",
            &CacheEntry {
                data: "a".into(),
                stored_at: 1000,
                ttl_secs: 900,
            },
        )
        .unwrap();
        upsert(
            &conn,
            "stale",
            &CacheEntry {
                data: "b".into(),
                stored_at: 1000,
                ttl_secs: 10,
            },
        )
        .unwrap();

        assert_eq!(evict_expired(&conn, 1050).unwrap(), 1);
        assert_eq!(count(&conn).unwrap(), 1);
        assert_eq!(clear(&conn).unwrap(), 1);
        assert_eq!(count(&conn).unwrap(), 0);
    }

    #[test]
    fn stats_increment_and_hit_rate() {
        let mut cache = PersistentCache::open_in_memory().unwrap();
        cache.set("k".into(), "v".into(), QUOTE_TTL, 1000).unwrap();

        assert_eq!(cache.get("k", 1000).unwrap().as_deref(), Some("v")); // hit
        assert_eq!(cache.get("missing", 1000).unwrap(), None); // miss
        assert_eq!(cache.get("k", 1000).unwrap().as_deref(), Some("v")); // hit

        let status = cache.status(1000).unwrap();
        assert_eq!(status.stats.hits, 2);
        assert_eq!(status.stats.misses, 1);
        assert!((status.stats.hit_rate() - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(status.total_entries, 1);
        assert_eq!(status.fresh_entries, 1);
    }

    #[test]
    fn hit_rate_zero_with_no_traffic() {
        let cache = PersistentCache::open_in_memory().unwrap();
        let status = cache.status(1000).unwrap();
        assert!((status.stats.hit_rate() - 0.0).abs() < f64::EPSILON);
    }
}
