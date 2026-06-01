# ldgr-provider-example

Example market data provider for [ldgr](https://github.com/kafkade/ldgr). Use this as a template when building your own community provider.

## What This Is

This crate implements the `QuoteProvider` trait from `ldgr-core` for a fictional "Acme Markets" data source. It demonstrates:

- Building fetch URLs (with API key handling)
- Parsing JSON responses into `Quote` and `Ohlcv` types
- Providing metadata for the provider registry
- Testing with mock response data (no network calls)

## Usage

```rust
use ldgr_provider_example::AcmeMarkets;
use ldgr_core::market::{QuoteProvider, ProviderRegistry};

let mut registry = ProviderRegistry::default_registry();
registry.register(Box::new(AcmeMarkets::new("your-api-key"))).unwrap();

let provider = registry.get_by_id("acme-markets").unwrap();
let url = provider.quote_url(&["AAPL"]);
// Fetch `url` with your platform's HTTP client, then:
// let quotes = provider.parse_quotes(&response)?;
```

## Creating Your Own Provider

See the [Provider Development Guide](../../docs/provider-development-guide.md) for a complete walkthrough.

## License

Apache-2.0
