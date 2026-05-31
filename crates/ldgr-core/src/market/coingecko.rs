//! `CoinGecko` market data provider for cryptocurrency prices.
//!
//! Free public API — no API key required for basic endpoints.
//! Rate limit: 5–15 calls/min (anonymous), 30/min with free Demo account.
//!
//! **Scope**: ldgr uses market data for net worth tracking, not trading.
//! `CoinGecko` data helps value crypto holdings as part of the overall
//! financial picture.

use rust_decimal::Decimal;

use super::types::{
    AssetClass, DateRange, MarketError, Ohlcv, ProviderMetadata, Quote, QuoteProvider,
};

/// `CoinGecko` cryptocurrency data provider.
pub struct CoinGecko;

impl CoinGecko {
    const BASE: &'static str = "https://api.coingecko.com/api/v3";
}

impl QuoteProvider for CoinGecko {
    fn name(&self) -> &'static str {
        "CoinGecko"
    }

    fn supported_asset_classes(&self) -> Vec<AssetClass> {
        vec![AssetClass::Crypto]
    }

    fn quote_url(&self, symbols: &[&str]) -> String {
        if symbols.is_empty() {
            return String::new();
        }
        let ids = symbols.join(",");
        format!(
            "{}/simple/price?ids={}&vs_currencies=usd&include_24hr_change=true&include_market_cap=true&include_24hr_vol=true",
            Self::BASE,
            ids
        )
    }

    fn parse_quotes(&self, response: &str) -> Result<Vec<Quote>, MarketError> {
        let json: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| MarketError::ParseError(format!("invalid JSON: {e}")))?;

        let obj = json
            .as_object()
            .ok_or_else(|| MarketError::ParseError("expected JSON object".into()))?;

        let mut quotes = Vec::new();

        for (id, data) in obj {
            let price = data
                .get("usd")
                .and_then(serde_json::Value::as_f64)
                .map_or(Decimal::ZERO, |f| {
                    Decimal::from_f64_retain(f).unwrap_or(Decimal::ZERO)
                });

            let change_pct = data
                .get("usd_24h_change")
                .and_then(serde_json::Value::as_f64)
                .map_or(Decimal::ZERO, |f| {
                    Decimal::from_f64_retain(f).unwrap_or(Decimal::ZERO)
                });

            let change = price * change_pct / Decimal::new(100, 0);

            let market_cap = data
                .get("usd_market_cap")
                .and_then(serde_json::Value::as_f64)
                .map(|f| Decimal::from_f64_retain(f).unwrap_or(Decimal::ZERO));

            let volume = data
                .get("usd_24h_vol")
                .and_then(serde_json::Value::as_f64)
                .map(|f| {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        f as u64
                    }
                });

            quotes.push(Quote {
                symbol: id.clone(),
                price,
                change,
                change_percent: change_pct,
                volume,
                market_cap,
                name: None,
                currency: "USD".into(),
                exchange: Some("CoinGecko".into()),
            });
        }

        Ok(quotes)
    }

    fn historical_url(&self, symbol: &str, range: &DateRange) -> String {
        let from = date_to_unix(&range.start).unwrap_or(0);
        let to = date_to_unix(&range.end).unwrap_or(0);
        format!(
            "{}/coins/{}/market_chart/range?vs_currency=usd&from={}&to={}",
            Self::BASE,
            symbol,
            from,
            to
        )
    }

    fn parse_historical(&self, response: &str) -> Result<Vec<Ohlcv>, MarketError> {
        let json: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| MarketError::ParseError(format!("invalid JSON: {e}")))?;

        let prices = json
            .get("prices")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| MarketError::ParseError("missing prices array".into()))?;

        let volumes = json
            .get("total_volumes")
            .and_then(serde_json::Value::as_array);

        let mut bars = Vec::new();

        for (i, point) in prices.iter().enumerate() {
            let arr = point
                .as_array()
                .ok_or_else(|| MarketError::ParseError("invalid price point".into()))?;
            if arr.len() < 2 {
                continue;
            }

            let ts_ms = arr[0].as_i64().unwrap_or(0);
            let price = arr[1].as_f64().map_or(Decimal::ZERO, |f| {
                Decimal::from_f64_retain(f).unwrap_or(Decimal::ZERO)
            });

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let volume = volumes
                .and_then(|v| v.get(i))
                .and_then(serde_json::Value::as_array)
                .and_then(|a| a.get(1))
                .and_then(serde_json::Value::as_f64)
                .map_or(0, |f| f as u64);

            let date = unix_ms_to_date(ts_ms);

            // CoinGecko returns daily close prices, not OHLCV
            bars.push(Ohlcv {
                date,
                open: price,
                high: price,
                low: price,
                close: price,
                volume,
            });
        }

        Ok(bars)
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: "coingecko",
            display_name: "CoinGecko",
            description: "Cryptocurrency prices, market caps, and historical data",
            url: "https://www.coingecko.com",
            requires_api_key: false,
            rate_limit_hint: Some("5-15 req/min (anonymous), 30/min (free Demo key)"),
            tos_url: Some("https://www.coingecko.com/en/terms"),
        }
    }
}

fn date_to_unix(date: &str) -> Option<i64> {
    let nd = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let dt = nd.and_hms_opt(0, 0, 0)?;
    Some(dt.and_utc().timestamp())
}

fn unix_ms_to_date(ts_ms: i64) -> String {
    chrono::DateTime::from_timestamp(ts_ms / 1000, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_QUOTE: &str = r#"{
        "bitcoin": {
            "usd": 67500.50,
            "usd_24h_change": 2.35,
            "usd_market_cap": 1330000000000.0,
            "usd_24h_vol": 28500000000.0
        },
        "ethereum": {
            "usd": 3450.25,
            "usd_24h_change": -1.20,
            "usd_market_cap": 415000000000.0,
            "usd_24h_vol": 14200000000.0
        }
    }"#;

    const SAMPLE_HISTORICAL: &str = r#"{
        "prices": [
            [1705276800000, 42500.0],
            [1705363200000, 43000.0],
            [1705449600000, 42800.0]
        ],
        "total_volumes": [
            [1705276800000, 25000000000.0],
            [1705363200000, 28000000000.0],
            [1705449600000, 26000000000.0]
        ]
    }"#;

    #[test]
    fn provider_name() {
        assert_eq!(CoinGecko.name(), "CoinGecko");
    }

    #[test]
    fn supports_crypto_only() {
        let classes = CoinGecko.supported_asset_classes();
        assert_eq!(classes, vec![AssetClass::Crypto]);
    }

    #[test]
    fn quote_url_format() {
        let url = CoinGecko.quote_url(&["bitcoin", "ethereum"]);
        assert!(url.contains("bitcoin,ethereum"));
        assert!(url.contains("simple/price"));
    }

    #[test]
    fn parse_quote_response() {
        let quotes = CoinGecko.parse_quotes(SAMPLE_QUOTE).unwrap();
        assert_eq!(quotes.len(), 2);

        let btc = quotes.iter().find(|q| q.symbol == "bitcoin").unwrap();
        assert!(btc.price > Decimal::new(67000, 0));
        assert_eq!(btc.currency, "USD");
        assert!(btc.market_cap.is_some());
    }

    #[test]
    fn parse_historical_response() {
        let bars = CoinGecko.parse_historical(SAMPLE_HISTORICAL).unwrap();
        assert_eq!(bars.len(), 3);
        assert!(bars[0].close > Decimal::new(42000, 0));
        assert!(bars[0].volume > 0);
    }

    #[test]
    fn historical_url_format() {
        let url = CoinGecko.historical_url(
            "bitcoin",
            &DateRange {
                start: "2024-01-01".into(),
                end: "2024-12-31".into(),
            },
        );
        assert!(url.contains("bitcoin"));
        assert!(url.contains("from="));
        assert!(url.contains("to="));
    }
}
