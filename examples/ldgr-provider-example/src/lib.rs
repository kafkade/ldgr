//! Example ldgr market data provider.
//!
//! This crate demonstrates how to implement a community market data
//! provider for ldgr. It implements the [`QuoteProvider`] trait from
//! `ldgr-core` for a fictional "Acme Markets" data source.
//!
//! **This is a template.** Copy this crate, replace the parsing logic
//! with your provider's API format, and publish as a standalone crate.
//!
//! # Architecture
//!
//! The [`QuoteProvider`] trait is **I/O-free**:
//!
//! - [`quote_url`](QuoteProvider::quote_url) and
//!   [`historical_url`](QuoteProvider::historical_url) build the HTTP
//!   URLs that platform code should fetch.
//! - [`parse_quotes`](QuoteProvider::parse_quotes) and
//!   [`parse_historical`](QuoteProvider::parse_historical) parse the
//!   raw response text into structured [`Quote`] and [`Ohlcv`] values.
//!
//! Platform code (CLI via reqwest, iOS via URLSession, web via fetch)
//! performs the actual HTTP request. This keeps `ldgr-core` compilable
//! to WASM and testable without network access.
//!
//! # Usage
//!
//! ```rust
//! use ldgr_provider_example::AcmeMarkets;
//! use ldgr_core::market::{QuoteProvider, ProviderRegistry};
//!
//! // Register with a registry
//! let mut registry = ProviderRegistry::default_registry();
//! registry.register(Box::new(AcmeMarkets::new("your-api-key")))
//!     .expect("register provider");
//!
//! // Build a URL for platform code to fetch
//! let provider = registry.get_by_id("acme-markets").unwrap();
//! let url = provider.quote_url(&["AAPL", "MSFT"]);
//! // Platform code fetches `url`, then:
//! // let quotes = provider.parse_quotes(&response_text)?;
//! ```

use ldgr_core::market::{
    AssetClass, DateRange, MarketError, Ohlcv, ProviderMetadata, Quote, QuoteProvider,
};
use rust_decimal::Decimal;

/// Example provider for the fictional "Acme Markets" API.
///
/// Demonstrates how to implement `QuoteProvider` for a JSON-based
/// market data API that requires an API key.
pub struct AcmeMarkets {
    api_key: String,
}

impl AcmeMarkets {
    const BASE: &'static str = "https://api.example.invalid/v1";

    /// Create a new Acme Markets provider with the given API key.
    ///
    /// **Security note**: Store API keys in environment variables or
    /// platform-specific secure storage (Keychain, credential manager).
    /// Never hard-code keys in source code or commit them to version
    /// control.
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
        }
    }
}

impl QuoteProvider for AcmeMarkets {
    fn name(&self) -> &'static str {
        "Acme Markets"
    }

    fn supported_asset_classes(&self) -> Vec<AssetClass> {
        vec![AssetClass::Stock, AssetClass::Etf]
    }

    fn quote_url(&self, symbols: &[&str]) -> String {
        // Note: API key is included in the URL. If your provider uses
        // headers for authentication, you'll need platform-specific
        // code to add the header. Document this in your README.
        let joined = symbols.join(",");
        format!(
            "{}/quotes?symbols={}&apikey={}",
            Self::BASE,
            joined,
            self.api_key
        )
    }

    fn parse_quotes(&self, response: &str) -> Result<Vec<Quote>, MarketError> {
        // Parse the provider's JSON format into ldgr Quote structs.
        // This example expects:
        // {
        //   "quotes": [
        //     {
        //       "symbol": "AAPL",
        //       "last": 185.50,
        //       "prev_close": 183.25,
        //       "volume": 54321000,
        //       "currency": "USD"
        //     }
        //   ]
        // }
        let json: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| MarketError::ParseError(format!("invalid JSON: {e}")))?;

        let items = json
            .get("quotes")
            .and_then(|q| q.as_array())
            .ok_or_else(|| MarketError::ParseError("missing 'quotes' array".into()))?;

        let mut quotes = Vec::new();

        for item in items {
            let symbol = item
                .get("symbol")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();

            let price = extract_decimal(item, "last")?;
            let prev_close = extract_decimal(item, "prev_close").unwrap_or(price);

            let change = price - prev_close;
            let change_percent = if prev_close.is_zero() {
                Decimal::ZERO
            } else {
                (change / prev_close) * Decimal::new(100, 0)
            };

            let volume = item.get("volume").and_then(serde_json::Value::as_u64);

            let currency = item
                .get("currency")
                .and_then(|c| c.as_str())
                .unwrap_or("USD")
                .to_string();

            quotes.push(Quote {
                symbol,
                price,
                change,
                change_percent,
                volume,
                market_cap: None,
                name: None,
                currency,
                exchange: Some("Acme".into()),
            });
        }

        Ok(quotes)
    }

    fn historical_url(&self, symbol: &str, range: &DateRange) -> String {
        format!(
            "{}/history?symbol={}&from={}&to={}&apikey={}",
            Self::BASE,
            symbol,
            range.start,
            range.end,
            self.api_key
        )
    }

    fn parse_historical(&self, response: &str) -> Result<Vec<Ohlcv>, MarketError> {
        // Expected format:
        // {
        //   "bars": [
        //     {"date": "2024-01-15", "o": 183.0, "h": 186.0, "l": 182.5, "c": 185.5, "v": 54321000}
        //   ]
        // }
        let json: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| MarketError::ParseError(format!("invalid JSON: {e}")))?;

        let bars_arr = json
            .get("bars")
            .and_then(|b| b.as_array())
            .ok_or_else(|| MarketError::ParseError("missing 'bars' array".into()))?;

        let mut bars = Vec::new();

        for bar in bars_arr {
            let date = bar
                .get("date")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();

            bars.push(Ohlcv {
                date,
                open: extract_decimal(bar, "o").unwrap_or(Decimal::ZERO),
                high: extract_decimal(bar, "h").unwrap_or(Decimal::ZERO),
                low: extract_decimal(bar, "l").unwrap_or(Decimal::ZERO),
                close: extract_decimal(bar, "c").unwrap_or(Decimal::ZERO),
                volume: bar.get("v").and_then(serde_json::Value::as_u64).unwrap_or(0),
            });
        }

        Ok(bars)
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: "acme-markets",
            display_name: "Acme Markets",
            description: "Stocks and ETFs from the Acme Markets API (example provider)",
            url: "https://example.invalid",
            requires_api_key: true,
            rate_limit_hint: Some("100 req/min"),
            tos_url: Some("https://example.invalid/terms"),
        }
    }
}

fn extract_decimal(obj: &serde_json::Value, key: &str) -> Result<Decimal, MarketError> {
    obj.get(key)
        .and_then(serde_json::Value::as_f64)
        .map(|f| Decimal::from_f64_retain(f).unwrap_or(Decimal::ZERO))
        .ok_or_else(|| MarketError::ParseError(format!("missing field: {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_QUOTES: &str = r#"{
        "quotes": [
            {
                "symbol": "AAPL",
                "last": 185.50,
                "prev_close": 183.25,
                "volume": 54321000,
                "currency": "USD"
            },
            {
                "symbol": "MSFT",
                "last": 380.00,
                "prev_close": 378.50,
                "volume": 22000000,
                "currency": "USD"
            }
        ]
    }"#;

    const SAMPLE_HISTORY: &str = r#"{
        "bars": [
            {"date": "2024-01-14", "o": 183.0, "h": 184.5, "l": 182.0, "c": 183.5, "v": 50000000},
            {"date": "2024-01-15", "o": 184.0, "h": 186.0, "l": 183.5, "c": 185.5, "v": 54321000}
        ]
    }"#;

    fn provider() -> AcmeMarkets {
        AcmeMarkets::new("test-key-123")
    }

    #[test]
    fn name_and_metadata() {
        let p = provider();
        assert_eq!(p.name(), "Acme Markets");

        let meta = p.metadata();
        assert_eq!(meta.id, "acme-markets");
        assert!(meta.requires_api_key);
        assert!(meta.tos_url.is_some());
    }

    #[test]
    fn supported_classes() {
        let classes = provider().supported_asset_classes();
        assert!(classes.contains(&AssetClass::Stock));
        assert!(classes.contains(&AssetClass::Etf));
        assert!(!classes.contains(&AssetClass::Crypto));
    }

    #[test]
    fn quote_url_includes_key() {
        let url = provider().quote_url(&["AAPL", "MSFT"]);
        assert!(url.contains("AAPL,MSFT"));
        assert!(url.contains("apikey=test-key-123"));
    }

    #[test]
    fn parse_quotes_success() {
        let quotes = provider().parse_quotes(SAMPLE_QUOTES).unwrap();
        assert_eq!(quotes.len(), 2);

        let aapl = quotes.iter().find(|q| q.symbol == "AAPL").unwrap();
        assert_eq!(aapl.price, Decimal::new(1855, 1));
        assert!(aapl.change > Decimal::ZERO);
        assert_eq!(aapl.currency, "USD");
        assert_eq!(aapl.volume, Some(54_321_000));
    }

    #[test]
    fn parse_historical_success() {
        let bars = provider().parse_historical(SAMPLE_HISTORY).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].date, "2024-01-14");
        assert_eq!(bars[1].close, Decimal::new(1855, 1));
        assert!(bars[0].volume > 0);
    }

    #[test]
    fn invalid_json_error() {
        assert!(provider().parse_quotes("not json").is_err());
        assert!(provider().parse_historical("{]").is_err());
    }

    #[test]
    fn historical_url_format() {
        let url = provider().historical_url(
            "AAPL",
            &DateRange {
                start: "2024-01-01".into(),
                end: "2024-12-31".into(),
            },
        );
        assert!(url.contains("AAPL"));
        assert!(url.contains("from=2024-01-01"));
        assert!(url.contains("to=2024-12-31"));
    }

    #[test]
    fn registers_in_registry() {
        use ldgr_core::market::ProviderRegistry;

        let mut reg = ProviderRegistry::default_registry();
        assert_eq!(reg.len(), 3); // built-ins only

        reg.register(Box::new(provider())).unwrap();
        assert_eq!(reg.len(), 4);

        let acme = reg.get_by_id("acme-markets").unwrap();
        assert_eq!(acme.name(), "Acme Markets");
    }
}
