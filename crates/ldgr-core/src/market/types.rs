//! Market data types and provider trait.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Market data errors.
#[derive(Debug, Error)]
pub enum MarketError {
    #[error("parse error: {0}")]
    ParseError(String),

    #[error("symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("provider error: {0}")]
    ProviderError(String),

    #[error("duplicate provider id: {0}")]
    DuplicateProvider(String),
}

/// Asset class for filtering provider capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetClass {
    Stock,
    Etf,
    MutualFund,
    Index,
    Forex,
    Crypto,
}

/// A real-time or delayed quote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub symbol: String,
    pub price: Decimal,
    pub change: Decimal,
    pub change_percent: Decimal,
    pub volume: Option<u64>,
    pub market_cap: Option<Decimal>,
    pub name: Option<String>,
    pub currency: String,
    pub exchange: Option<String>,
}

/// OHLCV (Open/High/Low/Close/Volume) bar for historical data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ohlcv {
    pub date: String,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: u64,
}

/// Date range for historical queries.
#[derive(Debug, Clone)]
pub struct DateRange {
    pub start: String,
    pub end: String,
}

/// Provider metadata for discovery, documentation, and registry listing.
#[derive(Debug, Clone)]
pub struct ProviderMetadata {
    /// Machine-readable identifier (e.g., `"yahoo-finance"`).
    pub id: &'static str,
    /// Human-readable name (e.g., `"Yahoo Finance"`).
    pub display_name: &'static str,
    /// One-line description of what this provider offers.
    pub description: &'static str,
    /// Provider website URL.
    pub url: &'static str,
    /// Whether an API key is required.
    pub requires_api_key: bool,
    /// Human-readable rate limit hint (e.g., `"2000 req/hr"`).
    pub rate_limit_hint: Option<&'static str>,
    /// URL to the provider's Terms of Service.
    pub tos_url: Option<&'static str>,
}

/// Pluggable market data provider trait.
///
/// **This trait is I/O-free.** It builds URLs for the platform to fetch
/// and parses the raw response text into structured data. This keeps
/// ldgr-core compilable to WASM and testable without network access.
///
/// Platform code (CLI via reqwest, iOS via `URLSession`, web via fetch)
/// handles the actual HTTP requests.
///
/// # Implementing a Community Provider
///
/// See `docs/provider-development-guide.md` for a step-by-step walkthrough
/// and `examples/ldgr-provider-example/` for a working template.
pub trait QuoteProvider: Send + Sync {
    /// Provider name (e.g., "Yahoo Finance").
    fn name(&self) -> &'static str;

    /// Asset classes this provider supports.
    fn supported_asset_classes(&self) -> Vec<AssetClass>;

    /// Build the URL for fetching current quotes.
    fn quote_url(&self, symbols: &[&str]) -> String;

    /// Parse a quote response into structured data.
    fn parse_quotes(&self, response: &str) -> Result<Vec<Quote>, MarketError>;

    /// Build the URL for fetching historical OHLCV data.
    fn historical_url(&self, symbol: &str, range: &DateRange) -> String;

    /// Parse a historical data response into OHLCV bars.
    fn parse_historical(&self, response: &str) -> Result<Vec<Ohlcv>, MarketError>;

    /// Provider metadata for registry listing and documentation.
    ///
    /// The default implementation returns minimal metadata derived from
    /// [`name()`](QuoteProvider::name). Override this to provide richer
    /// information (description, TOS URL, rate limits, etc.).
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: self.name(),
            display_name: self.name(),
            description: "",
            url: "",
            requires_api_key: false,
            rate_limit_hint: None,
            tos_url: None,
        }
    }
}

/// Create the set of built-in providers shipped with ldgr.
///
/// Used by both [`super::ProviderChain`] and [`super::ProviderRegistry`]
/// to ensure a single source of truth for the default provider list.
pub fn builtin_providers() -> Vec<Box<dyn QuoteProvider>> {
    vec![
        Box::new(super::yahoo::YahooFinance),
        Box::new(super::coingecko::CoinGecko),
        Box::new(super::ecb::Ecb),
    ]
}
