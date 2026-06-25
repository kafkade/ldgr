//! Client-side market data cache with TTL.
//!
//! Pure computation — cache logic operates on timestamps and durations.
//! The actual storage (`SQLite` or in-memory) is managed by the caller.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Default TTLs for different data types (per ADR-007).
pub const QUOTE_TTL: Duration = Duration::from_mins(15); // 15 minutes
pub const HISTORICAL_TTL: Duration = Duration::from_hours(24); // 24 hours
pub const FOREX_TTL: Duration = Duration::from_hours(24); // 24 hours

/// A cached market data entry with expiration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// The cached response data (JSON string).
    pub data: String,
    /// Unix timestamp when this entry was stored.
    pub stored_at: i64,
    /// TTL in seconds.
    pub ttl_secs: u64,
}

impl CacheEntry {
    /// Check if this entry is still fresh.
    #[allow(clippy::cast_possible_wrap)]
    pub fn is_fresh(&self, now: i64) -> bool {
        now < self.stored_at + self.ttl_secs as i64
    }

    /// Seconds remaining until expiration.
    #[allow(clippy::cast_possible_wrap)]
    pub fn remaining_secs(&self, now: i64) -> i64 {
        (self.stored_at + self.ttl_secs as i64) - now
    }
}

/// In-memory cache for market data.
#[derive(Debug, Default)]
pub struct MarketCache {
    entries: BTreeMap<String, CacheEntry>,
}

impl MarketCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a cached entry if it exists and is fresh.
    pub fn get(&self, key: &str, now: i64) -> Option<&CacheEntry> {
        self.entries.get(key).filter(|e| e.is_fresh(now))
    }

    /// Store an entry in the cache.
    pub fn set(&mut self, key: String, data: String, ttl: Duration, now: i64) {
        self.entries.insert(
            key,
            CacheEntry {
                data,
                stored_at: now,
                ttl_secs: ttl.as_secs(),
            },
        );
    }

    /// Insert a pre-built entry, preserving its stored timestamp and TTL.
    ///
    /// Used to hydrate the in-memory layer from persistent storage.
    pub fn restore(&mut self, key: String, entry: CacheEntry) {
        self.entries.insert(key, entry);
    }

    /// Remove expired entries.
    pub fn evict_expired(&mut self, now: i64) {
        self.entries.retain(|_, e| e.is_fresh(now));
    }

    /// Number of entries (including expired).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Build the cache key for a quote request.
    pub fn quote_key(provider: &str, symbols: &mut [&str]) -> String {
        symbols.sort_unstable();
        format!("quote:{provider}:{}", symbols.join(","))
    }

    /// Build the cache key for a historical request.
    pub fn historical_key(provider: &str, symbol: &str, start: &str, end: &str) -> String {
        format!("hist:{provider}:{symbol}:{start}:{end}")
    }
}

/// Simple rate limiter tracking request timestamps.
#[derive(Debug)]
pub struct RateLimiter {
    /// Maximum requests per window.
    max_requests: usize,
    /// Window duration in seconds.
    window_secs: i64,
    /// Timestamps of recent requests.
    timestamps: Vec<i64>,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests,
            #[allow(clippy::cast_possible_wrap)]
            window_secs: window.as_secs() as i64,
            timestamps: Vec::new(),
        }
    }

    /// Check if a request is allowed at the given time.
    pub fn check(&mut self, now: i64) -> bool {
        // Remove timestamps outside the window
        let cutoff = now - self.window_secs;
        self.timestamps.retain(|&t| t > cutoff);

        self.timestamps.len() < self.max_requests
    }

    /// Record that a request was made.
    pub fn record(&mut self, now: i64) {
        self.timestamps.push(now);
    }

    /// Try to make a request: check + record if allowed.
    pub fn try_request(&mut self, now: i64) -> bool {
        if self.check(now) {
            self.record(now);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_entry_fresh() {
        let entry = CacheEntry {
            data: "test".into(),
            stored_at: 1000,
            ttl_secs: 900, // 15 min
        };
        assert!(entry.is_fresh(1500)); // 500s later
        assert!(!entry.is_fresh(2000)); // 1000s later (expired)
    }

    #[test]
    fn cache_get_set() {
        let mut cache = MarketCache::new();
        cache.set("key1".into(), "data1".into(), QUOTE_TTL, 1000);

        assert!(cache.get("key1", 1500).is_some());
        assert!(cache.get("key1", 2000).is_none()); // expired (>15min)
        assert!(cache.get("missing", 1000).is_none());
    }

    #[test]
    fn cache_evict_expired() {
        let mut cache = MarketCache::new();
        cache.set("fresh".into(), "a".into(), QUOTE_TTL, 1000);
        cache.set("stale".into(), "b".into(), Duration::from_secs(10), 1000);

        cache.evict_expired(1050);
        assert_eq!(cache.len(), 1);
        assert!(cache.get("fresh", 1050).is_some());
    }

    #[test]
    fn cache_key_normalized() {
        let k1 = MarketCache::quote_key("yahoo", &mut ["MSFT", "AAPL"]);
        let k2 = MarketCache::quote_key("yahoo", &mut ["AAPL", "MSFT"]);
        assert_eq!(k1, k2); // sorted
    }

    #[test]
    fn rate_limiter_allows_within_limit() {
        let mut rl = RateLimiter::new(3, Duration::from_mins(1));
        assert!(rl.try_request(100));
        assert!(rl.try_request(110));
        assert!(rl.try_request(120));
        assert!(!rl.try_request(130)); // 4th request denied
    }

    #[test]
    fn rate_limiter_window_slides() {
        let mut rl = RateLimiter::new(2, Duration::from_mins(1));
        assert!(rl.try_request(100));
        assert!(rl.try_request(110));
        assert!(!rl.try_request(120)); // denied

        // After window slides past first request
        assert!(rl.try_request(170)); // 100 + 60 = 160, now 170 → first expired
    }
}
