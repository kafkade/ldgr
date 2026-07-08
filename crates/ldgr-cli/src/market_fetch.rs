//! Market-data fetch pipeline: shared proxy with direct-provider fallback.
//!
//! Implements ADR-007 Layer 2 on the client side. The pure proxy URL builder
//! lives in `ldgr-core` ([`MarketProxy`]); this module owns the platform I/O:
//! resolving configuration from the environment / CLI flags and performing the
//! actual HTTP requests with graceful fallback.
//!
//! Pipeline (the local `SQLite` cache — Layer 1 — is checked by callers first):
//!
//! 1. If the proxy is enabled, fetch `api.ldgr.dev/market/...`.
//! 2. If the proxy is unavailable (network error, non-2xx), fall back to a
//!    direct provider request — the same behavior as before this feature.
//!
//! Only symbol names are ever sent to the proxy; no vault data leaves the
//! device.

use ldgr_core::market::MarketProxy;

/// Environment variable selecting the market-data proxy endpoint.
///
/// - unset → use the default shared proxy ([`MarketProxy::DEFAULT_BASE`])
/// - `none` (case-insensitive) → disable the proxy, fetch providers directly
/// - any other value → use it as the proxy base URL
pub const PROXY_ENV: &str = "LDGR_MARKET_PROXY";

/// Resolved market-data proxy configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyConfig {
    /// Fetch via the shared proxy first, falling back to direct providers.
    Enabled(MarketProxy),
    /// Skip the proxy entirely and fetch providers directly.
    Disabled,
}

impl ProxyConfig {
    /// Resolve the proxy configuration from the `--no-proxy` flag and the
    /// [`PROXY_ENV`] environment variable.
    ///
    /// The flag takes precedence: `--no-proxy` always disables the proxy.
    #[must_use]
    pub fn resolve(no_proxy_flag: bool) -> Self {
        if no_proxy_flag {
            return Self::Disabled;
        }
        Self::from_env_value(std::env::var(PROXY_ENV).ok().as_deref())
    }

    /// Resolve from a raw environment-variable value (testable core of
    /// [`resolve`](Self::resolve)).
    #[must_use]
    pub fn from_env_value(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") => Self::Enabled(MarketProxy::default_base()),
            Some(v) if v.eq_ignore_ascii_case("none") => Self::Disabled,
            Some(v) => Self::Enabled(MarketProxy::new(v)),
        }
    }

    /// The proxy URL builder, if the proxy is enabled.
    #[must_use]
    pub fn proxy(&self) -> Option<&MarketProxy> {
        match self {
            Self::Enabled(p) => Some(p),
            Self::Disabled => None,
        }
    }

    /// Proxy `/quote` URL for a single symbol, or `None` when disabled.
    #[must_use]
    pub fn quote_url(&self, symbol: &str) -> Option<String> {
        self.proxy().map(|p| p.quote_url(&[symbol]))
    }

    /// Proxy `/historical` URL for a symbol/range, or `None` when disabled.
    #[must_use]
    pub fn historical_url(
        &self,
        symbol: &str,
        range: &ldgr_core::market::DateRange,
    ) -> Option<String> {
        self.proxy().map(|p| p.historical_url(symbol, range))
    }
}

/// Fetch response text, trying the proxy first (when present) then the direct
/// provider URL.
///
/// The proxy is treated as an optimization: any proxy failure (transport error
/// or non-2xx status) silently falls back to `direct_url`. Only the direct
/// request's outcome is surfaced as an error, preserving the pre-proxy
/// behavior for callers.
pub async fn fetch_text(
    client: &reqwest::Client,
    proxy_url: Option<String>,
    direct_url: &str,
) -> Result<String, String> {
    if let Some(url) = proxy_url
        && let Some(text) = try_get(client, &url).await
    {
        return Ok(text);
    }
    // Direct provider fetch (fallback, or the only path when proxy disabled).
    match client.get(direct_url).send().await {
        Ok(resp) => resp.text().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// GET a URL, returning the body only on a 2xx response. Any error or non-2xx
/// status yields `None` so the caller can fall back.
async fn try_get(client: &reqwest::Client, url: &str) -> Option<String> {
    match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => resp.text().await.ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_env_uses_default_proxy() {
        let cfg = ProxyConfig::from_env_value(None);
        assert_eq!(cfg, ProxyConfig::Enabled(MarketProxy::default_base()));
    }

    #[test]
    fn empty_env_uses_default_proxy() {
        assert_eq!(
            ProxyConfig::from_env_value(Some("   ")),
            ProxyConfig::Enabled(MarketProxy::default_base())
        );
    }

    #[test]
    fn none_disables_proxy() {
        assert_eq!(
            ProxyConfig::from_env_value(Some("none")),
            ProxyConfig::Disabled
        );
        assert_eq!(
            ProxyConfig::from_env_value(Some("NONE")),
            ProxyConfig::Disabled
        );
        assert_eq!(
            ProxyConfig::from_env_value(Some(" None ")),
            ProxyConfig::Disabled
        );
    }

    #[test]
    fn custom_url_is_used() {
        let cfg = ProxyConfig::from_env_value(Some("https://my.example/market/"));
        assert_eq!(
            cfg,
            ProxyConfig::Enabled(MarketProxy::new("https://my.example/market"))
        );
    }

    #[test]
    fn flag_overrides_env() {
        // Even with a valid env value, the flag forces Disabled.
        assert_eq!(ProxyConfig::resolve(true), ProxyConfig::Disabled);
    }

    #[test]
    fn disabled_yields_no_urls() {
        let cfg = ProxyConfig::Disabled;
        assert!(cfg.quote_url("AAPL").is_none());
        assert!(cfg.proxy().is_none());
    }

    #[test]
    fn enabled_builds_quote_url() {
        let cfg = ProxyConfig::from_env_value(None);
        assert_eq!(
            cfg.quote_url("AAPL").as_deref(),
            Some("https://api.ldgr.dev/market/quote?symbols=AAPL")
        );
    }
}
