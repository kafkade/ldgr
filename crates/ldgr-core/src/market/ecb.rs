//! European Central Bank (ECB) exchange rate provider.
//!
//! Completely free, no API key required, official government data.
//! Provides daily EUR-based reference rates for major currencies.
//!
//! **Note**: Rates are EUR-based. USD/X conversions are computed as
//! `(1/EUR_USD) * EUR_X`.

use rust_decimal::Decimal;

use super::types::{AssetClass, DateRange, MarketError, Ohlcv, Quote, QuoteProvider};

/// ECB exchange rate provider.
pub struct Ecb;

impl Ecb {
    const DAILY_URL: &'static str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
    const HIST_90D_URL: &'static str =
        "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist-90d.xml";
}

impl QuoteProvider for Ecb {
    fn name(&self) -> &'static str {
        "ECB"
    }

    fn supported_asset_classes(&self) -> Vec<AssetClass> {
        vec![AssetClass::Forex]
    }

    fn quote_url(&self, _symbols: &[&str]) -> String {
        Self::DAILY_URL.to_string()
    }

    fn parse_quotes(&self, response: &str) -> Result<Vec<Quote>, MarketError> {
        let rates = parse_ecb_xml(response)?;
        let mut quotes = Vec::new();

        for (currency, rate) in &rates {
            quotes.push(Quote {
                symbol: format!("EUR/{currency}"),
                price: *rate,
                change: Decimal::ZERO,
                change_percent: Decimal::ZERO,
                volume: None,
                market_cap: None,
                name: Some(format!("Euro to {currency}")),
                currency: currency.clone(),
                exchange: Some("ECB".into()),
            });
        }

        Ok(quotes)
    }

    fn historical_url(&self, _symbol: &str, _range: &DateRange) -> String {
        Self::HIST_90D_URL.to_string()
    }

    fn parse_historical(&self, response: &str) -> Result<Vec<Ohlcv>, MarketError> {
        let daily = parse_ecb_hist_xml(response);
        let mut bars = Vec::new();

        for (date, rates) in &daily {
            // Use USD rate as the primary bar (most common conversion)
            if let Some(usd_rate) = rates.get("USD") {
                bars.push(Ohlcv {
                    date: date.clone(),
                    open: *usd_rate,
                    high: *usd_rate,
                    low: *usd_rate,
                    close: *usd_rate,
                    volume: 0,
                });
            }
        }

        bars.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(bars)
    }
}

/// Parse ECB daily XML response.
///
/// Format: `<Cube currency="USD" rate="1.0850"/>`
fn parse_ecb_xml(xml: &str) -> Result<Vec<(String, Decimal)>, MarketError> {
    let mut rates = Vec::new();

    for line in xml.lines() {
        let trimmed = line.trim();
        if trimmed.contains("currency=")
            && trimmed.contains("rate=")
            && let (Some(currency), Some(rate)) = (
                extract_attr(trimmed, "currency"),
                extract_attr(trimmed, "rate"),
            )
        {
            let decimal: Decimal = rate
                .parse()
                .map_err(|e| MarketError::ParseError(format!("invalid rate '{rate}': {e}")))?;
            rates.push((currency, decimal));
        }
    }

    if rates.is_empty() {
        return Err(MarketError::ParseError("no rates found in ECB XML".into()));
    }

    Ok(rates)
}

/// Parse ECB historical XML (90-day) response.
///
/// Returns Vec of (date, Vec<(currency, rate)>).
fn parse_ecb_hist_xml(xml: &str) -> Vec<(String, std::collections::BTreeMap<String, Decimal>)> {
    let mut daily: Vec<(String, std::collections::BTreeMap<String, Decimal>)> = Vec::new();
    let mut current_date: Option<String> = None;
    let mut current_rates: std::collections::BTreeMap<String, Decimal> =
        std::collections::BTreeMap::new();

    for line in xml.lines() {
        let trimmed = line.trim();

        // Date line: <Cube time="2024-01-15">
        if trimmed.contains("time=") && !trimmed.contains("currency=") {
            // Save previous day
            if let Some(date) = current_date.take()
                && !current_rates.is_empty()
            {
                daily.push((date, std::mem::take(&mut current_rates)));
            }
            current_date = extract_attr(trimmed, "time");
        }

        // Rate line: <Cube currency="USD" rate="1.0850"/>
        if trimmed.contains("currency=")
            && trimmed.contains("rate=")
            && let (Some(currency), Some(rate_str)) = (
                extract_attr(trimmed, "currency"),
                extract_attr(trimmed, "rate"),
            )
            && let Ok(rate) = rate_str.parse::<Decimal>()
        {
            current_rates.insert(currency, rate);
        }
    }

    // Don't forget the last day
    if let Some(date) = current_date
        && !current_rates.is_empty()
    {
        daily.push((date, current_rates));
    }

    daily
}

/// Extract an XML attribute value: `attr="value"` → `value`.
fn extract_attr(s: &str, attr: &str) -> Option<String> {
    let pattern = format!("{attr}=\"");
    let start = s.find(&pattern)? + pattern.len();
    let end = s[start..].find('"')? + start;
    Some(s[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_DAILY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gesmes:Envelope>
<Cube>
  <Cube time="2024-01-15">
    <Cube currency="USD" rate="1.0850"/>
    <Cube currency="GBP" rate="0.8620"/>
    <Cube currency="JPY" rate="161.50"/>
    <Cube currency="CHF" rate="0.9415"/>
  </Cube>
</Cube>
</gesmes:Envelope>"#;

    const SAMPLE_HIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gesmes:Envelope>
<Cube>
  <Cube time="2024-01-15">
    <Cube currency="USD" rate="1.0850"/>
    <Cube currency="GBP" rate="0.8620"/>
  </Cube>
  <Cube time="2024-01-14">
    <Cube currency="USD" rate="1.0900"/>
    <Cube currency="GBP" rate="0.8600"/>
  </Cube>
</Cube>
</gesmes:Envelope>"#;

    #[test]
    fn provider_name() {
        assert_eq!(Ecb.name(), "ECB");
    }

    #[test]
    fn supports_forex_only() {
        assert_eq!(Ecb.supported_asset_classes(), vec![AssetClass::Forex]);
    }

    #[test]
    fn parse_daily_rates() {
        let quotes = Ecb.parse_quotes(SAMPLE_DAILY).unwrap();
        assert_eq!(quotes.len(), 4);

        let usd = quotes.iter().find(|q| q.symbol == "EUR/USD").unwrap();
        assert_eq!(usd.price, Decimal::new(10850, 4));
        assert_eq!(usd.exchange.as_deref(), Some("ECB"));
    }

    #[test]
    fn parse_historical_rates() {
        let bars = Ecb.parse_historical(SAMPLE_HIST).unwrap();
        assert_eq!(bars.len(), 2);
        // Sorted by date ascending
        assert_eq!(bars[0].date, "2024-01-14");
        assert_eq!(bars[1].date, "2024-01-15");
    }

    #[test]
    fn quote_url_returns_daily() {
        let url = Ecb.quote_url(&["USD", "GBP"]);
        assert!(url.contains("eurofxref-daily"));
    }

    #[test]
    fn extract_attr_works() {
        assert_eq!(
            extract_attr(r#"<Cube currency="USD" rate="1.08"/>"#, "currency"),
            Some("USD".into())
        );
        assert_eq!(
            extract_attr(r#"<Cube currency="USD" rate="1.08"/>"#, "rate"),
            Some("1.08".into())
        );
    }
}
