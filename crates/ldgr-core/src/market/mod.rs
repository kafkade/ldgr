//! Market data types and provider trait.
//!
//! The provider trait is I/O-free: it builds URLs and parses responses.
//! Platform code (CLI/iOS/web) handles the actual HTTP fetching.
//!
//! **Scope**: ldgr uses market data for **net worth tracking**, not trading.
//! Investment holdings are valued at current market prices to compute
//! overall net worth. For investment decisions, use specialized tools
//! (brokerage platforms, Bloomberg Terminal, etc.).
//!
//! See ADR-007 for the caching architecture.

pub mod cache;
pub mod chain;
pub mod coingecko;
pub mod ecb;
#[cfg(feature = "sqlite")]
pub mod persist;
pub mod registry;
pub mod types;
pub mod yahoo;

pub use cache::{MarketCache, RateLimiter};
pub use chain::ProviderChain;
pub use coingecko::CoinGecko;
pub use ecb::Ecb;
#[cfg(feature = "sqlite")]
pub use persist::{CacheStats, CacheStatus, CacheStoreError, PersistentCache};
pub use registry::ProviderRegistry;
pub use types::*;
pub use yahoo::YahooFinance;
