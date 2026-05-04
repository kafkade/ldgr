//! Yahoo Finance market data provider.
//!
//! Parses Yahoo Finance API v8 JSON responses. The actual HTTP fetching
//! is done by platform code — this module only builds URLs and parses
//! responses.
//!
//! **TOS disclaimer**: Yahoo Finance data is subject to Yahoo's Terms of
//! Service. This provider is community-provided and not affiliated with
//! Yahoo. Use at your own risk.

use rust_decimal::Decimal;

use super::types::{AssetClass, DateRange, MarketError, Ohlcv, Quote, QuoteProvider};

/// Yahoo Finance provider.
pub struct YahooFinance;

impl YahooFinance {
    /// Base URL for Yahoo Finance API v8.
    const QUOTE_BASE: &'static str = "https://query1.finance.yahoo.com/v8/finance/chart/";
}

impl QuoteProvider for YahooFinance {
    fn name(&self) -> &'static str {
        "Yahoo Finance"
    }

    fn supported_asset_classes(&self) -> Vec<AssetClass> {
        vec![
            AssetClass::Stock,
            AssetClass::Etf,
            AssetClass::MutualFund,
            AssetClass::Index,
            AssetClass::Forex,
            AssetClass::Crypto,
        ]
    }

    fn quote_url(&self, symbols: &[&str]) -> String {
        // Yahoo v8 chart API with single symbol (batch via multiple calls)
        if symbols.is_empty() {
            return String::new();
        }
        format!("{}{}?interval=1d&range=1d", Self::QUOTE_BASE, symbols[0])
    }

    fn parse_quotes(&self, response: &str) -> Result<Vec<Quote>, MarketError> {
        let json: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| MarketError::ParseError(format!("invalid JSON: {e}")))?;

        let chart = json
            .get("chart")
            .and_then(|c| c.get("result"))
            .and_then(|r| r.as_array())
            .ok_or_else(|| MarketError::ParseError("missing chart.result".into()))?;

        let mut quotes = Vec::new();

        for result in chart {
            let meta = result
                .get("meta")
                .ok_or_else(|| MarketError::ParseError("missing meta".into()))?;

            let symbol = meta
                .get("symbol")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();

            let price = extract_decimal(meta, "regularMarketPrice")?;
            let prev_close = extract_decimal(meta, "chartPreviousClose")
                .or_else(|_| extract_decimal(meta, "previousClose"))
                .unwrap_or(price);

            let change = price - prev_close;
            let change_percent = if prev_close.is_zero() {
                Decimal::ZERO
            } else {
                (change / prev_close) * Decimal::new(100, 0)
            };

            let currency = meta
                .get("currency")
                .and_then(|c| c.as_str())
                .unwrap_or("USD")
                .to_string();

            let exchange = meta
                .get("exchangeName")
                .and_then(|e| e.as_str())
                .map(String::from);

            let volume = meta
                .get("regularMarketVolume")
                .and_then(serde_json::Value::as_u64);

            quotes.push(Quote {
                symbol,
                price,
                change,
                change_percent,
                volume,
                market_cap: None,
                name: None,
                currency,
                exchange,
            });
        }

        Ok(quotes)
    }

    fn historical_url(&self, symbol: &str, range: &DateRange) -> String {
        // Convert date range to Yahoo period parameters
        let period1 = date_to_unix(&range.start).unwrap_or(0);
        let period2 = date_to_unix(&range.end).unwrap_or(0);

        format!(
            "{}{}?period1={}&period2={}&interval=1d",
            Self::QUOTE_BASE,
            symbol,
            period1,
            period2
        )
    }

    fn parse_historical(&self, response: &str) -> Result<Vec<Ohlcv>, MarketError> {
        let json: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| MarketError::ParseError(format!("invalid JSON: {e}")))?;

        let result = json
            .get("chart")
            .and_then(|c| c.get("result"))
            .and_then(|r| r.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| MarketError::ParseError("missing chart.result".into()))?;

        let timestamps = result
            .get("timestamp")
            .and_then(|t| t.as_array())
            .ok_or_else(|| MarketError::ParseError("missing timestamps".into()))?;

        let indicators = result
            .get("indicators")
            .and_then(|i| i.get("quote"))
            .and_then(|q| q.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| MarketError::ParseError("missing indicators.quote".into()))?;

        let opens = indicators.get("open").and_then(|v| v.as_array());
        let highs = indicators.get("high").and_then(|v| v.as_array());
        let lows = indicators.get("low").and_then(|v| v.as_array());
        let closes = indicators.get("close").and_then(|v| v.as_array());
        let volumes = indicators.get("volume").and_then(|v| v.as_array());

        let mut bars = Vec::new();

        for (i, ts) in timestamps.iter().enumerate() {
            let Some(unix) = ts.as_i64() else {
                continue;
            };
            let date = unix_to_date(unix);

            let open = array_decimal(opens, i).unwrap_or(Decimal::ZERO);
            let high = array_decimal(highs, i).unwrap_or(Decimal::ZERO);
            let low = array_decimal(lows, i).unwrap_or(Decimal::ZERO);
            let close = array_decimal(closes, i).unwrap_or(Decimal::ZERO);
            let volume = volumes
                .and_then(|v| v.get(i))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);

            // Skip bars with null close (market holidays)
            if close.is_zero() {
                continue;
            }

            bars.push(Ohlcv {
                date,
                open,
                high,
                low,
                close,
                volume,
            });
        }

        Ok(bars)
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn extract_decimal(obj: &serde_json::Value, key: &str) -> Result<Decimal, MarketError> {
    obj.get(key)
        .and_then(serde_json::Value::as_f64)
        .map(|f| Decimal::from_f64_retain(f).unwrap_or(Decimal::ZERO))
        .ok_or_else(|| MarketError::ParseError(format!("missing or invalid field: {key}")))
}

fn array_decimal(arr: Option<&Vec<serde_json::Value>>, i: usize) -> Option<Decimal> {
    arr?.get(i)?
        .as_f64()
        .map(|f| Decimal::from_f64_retain(f).unwrap_or(Decimal::ZERO))
}

fn date_to_unix(date: &str) -> Option<i64> {
    let nd = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let dt = nd.and_hms_opt(0, 0, 0)?;
    Some(dt.and_utc().timestamp())
}

fn unix_to_date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_QUOTE_RESPONSE: &str = r#"{
        "chart": {
            "result": [{
                "meta": {
                    "symbol": "AAPL",
                    "regularMarketPrice": 185.5,
                    "chartPreviousClose": 183.25,
                    "currency": "USD",
                    "exchangeName": "NMS",
                    "regularMarketVolume": 54321000
                },
                "timestamp": [1705363200],
                "indicators": {
                    "quote": [{"open": [183.0], "high": [186.0], "low": [182.5], "close": [185.5], "volume": [54321000]}]
                }
            }]
        }
    }"#;

    const SAMPLE_HISTORICAL_RESPONSE: &str = r#"{
        "chart": {
            "result": [{
                "meta": {"symbol": "AAPL"},
                "timestamp": [1705276800, 1705363200, 1705449600],
                "indicators": {
                    "quote": [{
                        "open": [183.0, 184.0, 185.0],
                        "high": [184.5, 186.0, 187.0],
                        "low": [182.0, 183.5, 184.5],
                        "close": [183.5, 185.5, 186.0],
                        "volume": [50000000, 54321000, 48000000]
                    }]
                }
            }]
        }
    }"#;

    #[test]
    fn provider_name() {
        let yf = YahooFinance;
        assert_eq!(yf.name(), "Yahoo Finance");
    }

    #[test]
    fn supports_all_asset_classes() {
        let yf = YahooFinance;
        let classes = yf.supported_asset_classes();
        assert!(classes.contains(&AssetClass::Stock));
        assert!(classes.contains(&AssetClass::Crypto));
        assert!(classes.contains(&AssetClass::Forex));
    }

    #[test]
    fn quote_url_format() {
        let yf = YahooFinance;
        let url = yf.quote_url(&["AAPL"]);
        assert!(url.contains("AAPL"));
        assert!(url.contains("chart"));
    }

    #[test]
    fn parse_quote_response() {
        let yf = YahooFinance;
        let quotes = yf.parse_quotes(SAMPLE_QUOTE_RESPONSE).unwrap();

        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].symbol, "AAPL");
        assert_eq!(quotes[0].price, Decimal::new(1855, 1));
        assert_eq!(quotes[0].currency, "USD");
        assert!(quotes[0].change > Decimal::ZERO);
        assert!(quotes[0].change_percent > Decimal::ZERO);
        assert_eq!(quotes[0].volume, Some(54_321_000));
    }

    #[test]
    fn parse_historical_response() {
        let yf = YahooFinance;
        let bars = yf.parse_historical(SAMPLE_HISTORICAL_RESPONSE).unwrap();

        assert_eq!(bars.len(), 3);
        assert_eq!(bars[0].close, Decimal::new(1835, 1));
        assert_eq!(bars[1].close, Decimal::new(1855, 1));
        assert_eq!(bars[2].close, Decimal::new(1860, 1));
        assert!(bars[0].volume > 0);
    }

    #[test]
    fn historical_url_format() {
        let yf = YahooFinance;
        let url = yf.historical_url(
            "AAPL",
            &DateRange {
                start: "2024-01-01".into(),
                end: "2024-12-31".into(),
            },
        );
        assert!(url.contains("AAPL"));
        assert!(url.contains("period1="));
        assert!(url.contains("period2="));
    }

    #[test]
    fn invalid_json_produces_error() {
        let yf = YahooFinance;
        assert!(yf.parse_quotes("not json").is_err());
    }

    #[test]
    fn date_unix_round_trip() {
        let unix = date_to_unix("2024-01-15").unwrap();
        let date = unix_to_date(unix);
        assert_eq!(date, "2024-01-15");
    }
}
