//! Shared market-data proxy URL builder (ADR-007 Layer 2).
//!
//! Pure, I/O-free URL construction for the shared caching proxy served at
//! `api.ldgr.dev/market/`. Platform code (CLI/iOS/web) performs the actual HTTP
//! fetch and falls back to direct provider requests when the proxy is
//! unavailable.
//!
//! The proxy returns responses in the **exact same shape** as the direct
//! providers (Yahoo-shaped JSON for quotes/history, `CoinGecko`
//! `simple/price` JSON, ECB XML), so the existing provider parsers in this
//! module parse proxy responses without modification.
//!
//! **Privacy**: only symbol names ever reach the proxy — never any vault,
//! balance, or portfolio data. See ADR-007.

use super::types::DateRange;

/// Builds request URLs for the shared market-data proxy.
///
/// This type performs **no I/O**. It only formats URLs for the routes exposed
/// by the Cloudflare Worker (`/quote`, `/historical`, `/crypto`, `/forex`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketProxy {
    /// Base URL with any trailing slash stripped (e.g. `https://api.ldgr.dev/market`).
    base: String,
}

impl MarketProxy {
    /// Default shared proxy endpoint.
    pub const DEFAULT_BASE: &'static str = "https://api.ldgr.dev/market";

    /// Create a proxy targeting `base`. A trailing slash is stripped so route
    /// joining is unambiguous.
    pub fn new(base: impl Into<String>) -> Self {
        let mut base = base.into();
        while base.ends_with('/') {
            base.pop();
        }
        Self { base }
    }

    /// Create a proxy targeting the default shared endpoint.
    pub fn default_base() -> Self {
        Self::new(Self::DEFAULT_BASE)
    }

    /// The normalized base URL (no trailing slash).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Build the `/quote` URL for one or more symbols.
    ///
    /// Symbols are normalized (uppercased, de-duplicated, sorted) so requests
    /// for the same basket resolve to the same edge cache key regardless of
    /// order or case — matching the Worker's own normalization.
    pub fn quote_url(&self, symbols: &[&str]) -> String {
        let joined = normalize_join(symbols, true);
        format!("{}/quote?symbols={joined}", self.base)
    }

    /// Build the `/historical` URL for a single symbol over a date range.
    pub fn historical_url(&self, symbol: &str, range: &DateRange) -> String {
        format!(
            "{}/historical?symbol={}&start={}&end={}",
            self.base,
            encode(&symbol.to_uppercase()),
            encode(&range.start),
            encode(&range.end),
        )
    }

    /// Build the `/crypto` URL for one or more `CoinGecko` coin ids.
    ///
    /// Coin ids are lowercase slugs (e.g. `bitcoin`), so they are lowercased
    /// rather than uppercased.
    pub fn crypto_url(&self, ids: &[&str]) -> String {
        let joined = normalize_join(ids, false);
        format!("{}/crypto?ids={joined}", self.base)
    }

    /// Build the `/forex` URL (EUR-based daily reference rates).
    pub fn forex_url(&self) -> String {
        format!("{}/forex", self.base)
    }
}

/// Normalize a symbol list (case-fold, trim, de-dup, sort) and percent-encode
/// each entry, joining with commas. Mirrors the Worker's `normalizeSymbols`.
fn normalize_join(symbols: &[&str], upper: bool) -> String {
    let mut normalized: Vec<String> = symbols
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if upper {
                s.to_uppercase()
            } else {
                s.to_lowercase()
            }
        })
        .collect();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
        .iter()
        .map(|s| encode(s))
        .collect::<Vec<_>>()
        .join(",")
}

/// Percent-encode a query-parameter value, preserving the unreserved set
/// (`A-Z a-z 0-9 - . _ ~`). Keeps ldgr-core dependency-free (no url crate).
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0x0f));
            }
        }
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_base_has_no_trailing_slash() {
        let p = MarketProxy::default_base();
        assert_eq!(p.base(), "https://api.ldgr.dev/market");
    }

    #[test]
    fn trailing_slashes_are_stripped() {
        let p = MarketProxy::new("https://example.com/market///");
        assert_eq!(p.base(), "https://example.com/market");
    }

    #[test]
    fn quote_url_normalizes_and_sorts() {
        let p = MarketProxy::default_base();
        let a = p.quote_url(&["msft", "AAPL"]);
        let b = p.quote_url(&["AAPL", "MSFT"]);
        assert_eq!(a, b);
        assert_eq!(a, "https://api.ldgr.dev/market/quote?symbols=AAPL,MSFT");
    }

    #[test]
    fn quote_url_dedups_and_drops_empties() {
        let p = MarketProxy::default_base();
        let url = p.quote_url(&["AAPL", "aapl", " ", "MSFT"]);
        assert_eq!(url, "https://api.ldgr.dev/market/quote?symbols=AAPL,MSFT");
    }

    #[test]
    fn quote_url_percent_encodes_special_chars() {
        let p = MarketProxy::default_base();
        // Index (^GSPC) and forex (EURUSD=X) tickers need encoding.
        let url = p.quote_url(&["^GSPC", "EURUSD=X"]);
        assert!(url.contains("%5EGSPC"), "caret should be encoded: {url}");
        assert!(
            url.contains("EURUSD%3DX"),
            "equals should be encoded: {url}"
        );
    }

    #[test]
    fn historical_url_includes_range() {
        let p = MarketProxy::default_base();
        let url = p.historical_url(
            "aapl",
            &DateRange {
                start: "2024-01-01".into(),
                end: "2024-12-31".into(),
            },
        );
        assert_eq!(
            url,
            "https://api.ldgr.dev/market/historical?symbol=AAPL&start=2024-01-01&end=2024-12-31"
        );
    }

    #[test]
    fn crypto_url_lowercases() {
        let p = MarketProxy::default_base();
        let url = p.crypto_url(&["Ethereum", "bitcoin"]);
        assert_eq!(
            url,
            "https://api.ldgr.dev/market/crypto?ids=bitcoin,ethereum"
        );
    }

    #[test]
    fn forex_url_is_static() {
        let p = MarketProxy::new("https://api.ldgr.dev/market/");
        assert_eq!(p.forex_url(), "https://api.ldgr.dev/market/forex");
    }

    #[test]
    fn encode_preserves_unreserved() {
        assert_eq!(encode("AAPL-b.c_d~e"), "AAPL-b.c_d~e");
        assert_eq!(encode("a b"), "a%20b");
    }
}
