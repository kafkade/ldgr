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

/// Pluggable market data provider trait.
///
/// **This trait is I/O-free.** It builds URLs for the platform to fetch
/// and parses the raw response text into structured data. This keeps
/// ldgr-core compilable to WASM and testable without network access.
///
/// Platform code (CLI via reqwest, iOS via `URLSession`, web via fetch)
/// handles the actual HTTP requests.
pub trait QuoteProvider {
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
}
