# Market Data Provider Development Guide

This guide explains how to create a community market data provider for ldgr.

## Architecture Overview

ldgr's market data system is designed around a single principle: **the core library does no I/O**. The `QuoteProvider` trait builds URLs and parses responses — platform code (CLI via reqwest, iOS via URLSession, web via fetch) handles the actual HTTP requests.

```text
Platform code                    ldgr-core (your provider)
─────────────                    ─────────────────────────
                                 quote_url(&["AAPL"])
                                     → "https://api.example.com/v1/quotes?symbols=AAPL"
fetch(url) → response text
                                 parse_quotes(response)
                                     → Vec<Quote>
```

This keeps ldgr-core compilable to WASM and testable without network access.

## Quick Start

1. Copy the example crate:

   ```sh
   cp -r examples/ldgr-provider-example my-provider
   cd my-provider
   ```

2. Update `Cargo.toml`:

   ```toml
   [package]
   name = "ldgr-provider-myapi"
   version = "0.1.0"
   edition = "2024"

   [dependencies]
   ldgr-core = { version = "1.2", features = ["market"] }
   rust_decimal = "1"
   serde_json = "1"
   ```

3. Implement the `QuoteProvider` trait (see below).

4. Run tests: `cargo test`

## The QuoteProvider Trait

```rust
pub trait QuoteProvider: Send + Sync {
    // Required methods:
    fn name(&self) -> &'static str;
    fn supported_asset_classes(&self) -> Vec<AssetClass>;
    fn quote_url(&self, symbols: &[&str]) -> String;
    fn parse_quotes(&self, response: &str) -> Result<Vec<Quote>, MarketError>;
    fn historical_url(&self, symbol: &str, range: &DateRange) -> String;
    fn parse_historical(&self, response: &str) -> Result<Vec<Ohlcv>, MarketError>;

    // Optional (has default):
    fn metadata(&self) -> ProviderMetadata { /* ... */ }
}
```

### Required Methods

| Method | Purpose |
| --- | --- |
| `name()` | Human-readable provider name |
| `supported_asset_classes()` | Which asset types this provider handles |
| `quote_url(symbols)` | Build the URL for fetching current prices |
| `parse_quotes(response)` | Parse the HTTP response into `Vec<Quote>` |
| `historical_url(symbol, range)` | Build the URL for historical OHLCV data |
| `parse_historical(response)` | Parse the HTTP response into `Vec<Ohlcv>` |

### Metadata (Recommended)

Override `metadata()` to provide rich provider information:

```rust
fn metadata(&self) -> ProviderMetadata {
    ProviderMetadata {
        id: "my-provider",           // unique, kebab-case
        display_name: "My Provider",
        description: "Stock and ETF data from My Provider API",
        url: "https://myprovider.com",
        requires_api_key: true,
        rate_limit_hint: Some("100 req/min"),
        tos_url: Some("https://myprovider.com/terms"),
    }
}
```

The `id` field must be unique across all registered providers. Use kebab-case (e.g., `"alpha-vantage"`, `"twelve-data"`).

### Key Types

**`Quote`** — a current or delayed price:

```rust
pub struct Quote {
    pub symbol: String,
    pub price: Decimal,        // use rust_decimal, never f64
    pub change: Decimal,
    pub change_percent: Decimal,
    pub volume: Option<u64>,
    pub market_cap: Option<Decimal>,
    pub name: Option<String>,
    pub currency: String,
    pub exchange: Option<String>,
}
```

**`Ohlcv`** — a historical price bar:

```rust
pub struct Ohlcv {
    pub date: String,   // "YYYY-MM-DD"
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: u64,
}
```

**`AssetClass`** — what the provider supports:

```rust
pub enum AssetClass {
    Stock, Etf, MutualFund, Index, Forex, Crypto,
}
```

## Step-by-Step Implementation

### 1. Define Your Provider Struct

```rust
pub struct MyProvider {
    api_key: String,
}

impl MyProvider {
    pub fn new(api_key: &str) -> Self {
        Self { api_key: api_key.to_string() }
    }
}
```

If no API key is needed, use a unit struct: `pub struct MyProvider;`

### 2. Build URLs

The URL methods return a `String` that platform code will `GET`. Include API keys as query parameters if needed.

```rust
fn quote_url(&self, symbols: &[&str]) -> String {
    let joined = symbols.join(",");
    format!("https://api.example.com/quotes?s={}&key={}", joined, self.api_key)
}
```

**Important**: If your provider requires authentication via HTTP headers (not URL parameters), document this clearly. The current trait only returns URLs — platform-specific code must add headers. Include a section in your README explaining how to configure this.

### 3. Parse Responses

Parse the raw response text (JSON, XML, CSV) into ldgr types. Use `MarketError` for errors:

```rust
fn parse_quotes(&self, response: &str) -> Result<Vec<Quote>, MarketError> {
    let json: serde_json::Value = serde_json::from_str(response)
        .map_err(|e| MarketError::ParseError(format!("invalid JSON: {e}")))?;

    // Extract data from your provider's format...
    // Return MarketError::SymbolNotFound for missing symbols
    // Return MarketError::ProviderError for API errors
}
```

### 4. Write Tests with Mock Responses

Test against representative response strings — no HTTP calls:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RESPONSE: &str = r#"{"quotes": [{"symbol": "AAPL", "last": 185.50}]}"#;

    #[test]
    fn parse_quote() {
        let provider = MyProvider::new("test-key");
        let quotes = provider.parse_quotes(SAMPLE_RESPONSE).unwrap();
        assert_eq!(quotes[0].symbol, "AAPL");
    }
}
```

**Test checklist:**

- Parse a valid single-symbol response
- Parse a valid multi-symbol response
- Handle empty results gracefully
- Return `MarketError::ParseError` on malformed input
- Return `MarketError::SymbolNotFound` when a symbol is missing
- Verify `Decimal` precision (not floating-point approximation)
- Verify URL construction includes all expected parameters

### 5. Register Your Provider

```rust
use ldgr_core::market::ProviderRegistry;

let mut registry = ProviderRegistry::default_registry();
registry.register(Box::new(MyProvider::new("key"))).unwrap();
```

The registry rejects duplicate IDs — each provider must have a unique `metadata().id`.

## API Key Security

- **Never commit API keys** to source control.
- Store keys in environment variables, platform keychains, or configuration files excluded from version control.
- If your provider's URL includes an API key, be aware that URLs may appear in cache keys, debug output, or error messages. Avoid logging full URLs.
- Tests should use mock responses — never call real APIs in tests.

## Built-In Providers

ldgr ships with three built-in providers:

### Yahoo Finance (`yahoo-finance`)

| Property | Value |
| --- | --- |
| Asset classes | Stock, ETF, Mutual Fund, Index, Forex, Crypto |
| API key | Not required |
| Rate limit | ~2000 req/hr (unofficial, subject to change) |
| API | Yahoo Finance Chart API v8 (JSON) |
| TOS | [Yahoo Terms of Service](https://legal.yahoo.com/us/en/yahoo/terms/otos/index.html) |

**Caveats**: Yahoo Finance is an unofficial API. It has no published SLA, rate limits can change without notice, and commercial use may violate Yahoo's Terms of Service. Yahoo may block IPs or require CAPTCHA verification at any time.

### CoinGecko (`coingecko`)

| Property | Value |
| --- | --- |
| Asset classes | Crypto |
| API key | Not required (free tier) |
| Rate limit | 5–15 req/min (anonymous), 30/min (free Demo key) |
| API | CoinGecko Public API v3 (JSON) |
| TOS | [CoinGecko Terms](https://www.coingecko.com/en/terms) |

**Notes**: CoinGecko returns daily close prices for historical data (not true OHLCV). The `open`, `high`, and `low` fields are set equal to `close`. Free tier has strict rate limits; consider adding a Demo API key for higher throughput.

### ECB (`ecb`)

| Property | Value |
| --- | --- |
| Asset classes | Forex |
| API key | Not required |
| Rate limit | No limit (daily XML file) |
| API | ECB Euro Foreign Exchange Reference Rates (XML) |
| Data source | [ECB reference rates](https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/index.en.html) |

**Notes**: Rates are EUR-based. Updated once per business day (~16:00 CET). Historical data is limited to the last 90 business days. These are reference rates, not tradeable quotes.

## Terms of Service Guidance

When creating a community provider, you are responsible for complying with the data source's Terms of Service:

1. **Read the TOS** before implementing. Some providers prohibit redistribution, require attribution, or restrict use to non-commercial applications.

2. **Respect rate limits.** Use the `RateLimiter` from `ldgr_core::market::cache` or implement your own. Document the limits in your provider's `rate_limit_hint`.

3. **Include TOS URL** in your provider's `metadata().tos_url` so users can review the terms.

4. **Do not scrape** websites or bypass access controls. Only use documented public APIs.

5. **API keys are user-provided.** Your provider crate should accept an API key at construction time — never embed keys in source code.

6. **ldgr does not grant rights** to use third-party market data. Each user must independently comply with the data source's terms.

7. **Attribution**: If the provider's TOS requires attribution (e.g., "Powered by X"), document this requirement clearly so that platform UI code can display it.

## Provider Ideas

Community providers could cover additional data sources:

| Provider | Asset Classes | Notes |
| --- | --- | --- |
| Alpha Vantage | Stocks, ETFs, Forex, Crypto | Free tier: 25 req/day. Requires API key. |
| Twelve Data | Stocks, ETFs, Forex, Crypto | Free tier: 800 req/day. Requires API key. |
| Finnhub | Stocks, Forex, Crypto | Free tier: 60 req/min. Requires API key. |
| IEX Cloud | Stocks, ETFs | Pay-per-use pricing. Requires API key. |
| Open Exchange Rates | Forex | Free tier: 1000 req/month. Requires API key. |
| Frankfurter | Forex | Free, powered by ECB data. No API key. |
| Metals API | Commodities | Precious metals. Requires API key. |

## Publishing

1. Name your crate `ldgr-provider-<name>` (e.g., `ldgr-provider-alpha-vantage`).
2. Set `license = "Apache-2.0"` (or your choice — ldgr-core is Apache-2.0).
3. Include a README with setup instructions, API key configuration, and TOS link.
4. Publish to crates.io: `cargo publish`.
5. Open a PR to add your provider to the table above in this guide.
