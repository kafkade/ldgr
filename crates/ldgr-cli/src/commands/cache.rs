//! `ldgr cache` — manage the local market data price cache.
//!
//! The cache is a standalone `SQLite` database (`~/.ldgr/market_cache.db`) holding
//! only public market data (symbols and prices). It is independent of the encrypted
//! vault, so these commands work whether or not a vault is unlocked.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ldgr_core::market::PersistentCache;

use crate::session;

/// Resolve the path to the market cache database.
///
/// Stored alongside the vault directory (`~/.ldgr/` by default), but as its own
/// file — it never contains user financial data.
pub fn cache_db_path(vault_path: &Path) -> PathBuf {
    session::resolve_vault_dir(Some(vault_path)).join("market_cache.db")
}

/// Open the persistent market cache, creating it if necessary.
pub fn open_cache(vault_path: &Path) -> Result<PersistentCache> {
    let path = cache_db_path(vault_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
    }
    let now = chrono::Utc::now().timestamp();
    PersistentCache::open(&path, now)
        .with_context(|| format!("failed to open market cache at {}", path.display()))
}

/// `ldgr cache clear` — flush all cached prices.
pub fn run_clear(vault_path: &Path) -> Result<()> {
    let path = cache_db_path(vault_path);
    if !path.exists() {
        eprintln!("Market cache is empty (no cache database yet).");
        return Ok(());
    }

    let mut cache = open_cache(vault_path)?;
    let removed = cache.clear().context("failed to clear market cache")?;
    eprintln!("✓ Cleared {removed} cached price entr{}.", plural(removed));
    Ok(())
}

/// `ldgr cache status` — show entry count and hit rate.
#[allow(clippy::cast_precision_loss)]
pub fn run_status(vault_path: &Path) -> Result<()> {
    let path = cache_db_path(vault_path);
    eprintln!("Cache: {}", path.display());

    if !path.exists() {
        eprintln!("Status: empty (no cache database yet)");
        return Ok(());
    }

    let cache = open_cache(vault_path)?;
    let now = chrono::Utc::now().timestamp();
    let status = cache.status(now).context("failed to read cache status")?;

    let expired = status.total_entries.saturating_sub(status.fresh_entries);
    eprintln!(
        "Entries: {} ({} fresh, {expired} expired)",
        status.total_entries, status.fresh_entries
    );

    let hits = status.stats.hits;
    let misses = status.stats.misses;
    let total = hits + misses;
    if total == 0 {
        eprintln!("Hit rate: n/a (no lookups recorded yet)");
    } else {
        eprintln!(
            "Hit rate: {:.1}% ({hits} hit{}, {misses} miss{})",
            status.stats.hit_rate() * 100.0,
            if hits == 1 { "" } else { "s" },
            if misses == 1 { "" } else { "es" }
        );
    }

    Ok(())
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "y" } else { "ies" }
}
