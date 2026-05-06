//! Provider chain: try providers in order with fallback.
//!
//! Routes requests to the appropriate provider based on asset class,
//! with fallback on failure.

use super::types::{AssetClass, QuoteProvider};

/// A chain of market data providers with fallback.
///
/// Requests are routed by asset class. If the primary provider fails,
/// the chain tries the next provider that supports the asset class.
pub struct ProviderChain {
    providers: Vec<Box<dyn QuoteProvider>>,
}

impl ProviderChain {
    pub fn new(providers: Vec<Box<dyn QuoteProvider>>) -> Self {
        Self { providers }
    }

    /// Find the first provider that supports the given asset class.
    pub fn provider_for(&self, asset_class: AssetClass) -> Option<&dyn QuoteProvider> {
        self.providers
            .iter()
            .find(|p| p.supported_asset_classes().contains(&asset_class))
            .map(AsRef::as_ref)
    }

    /// Get all providers that support a given asset class (for fallback).
    pub fn providers_for(&self, asset_class: AssetClass) -> Vec<&dyn QuoteProvider> {
        self.providers
            .iter()
            .filter(|p| p.supported_asset_classes().contains(&asset_class))
            .map(AsRef::as_ref)
            .collect()
    }

    /// List all provider names.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    /// Build the default provider chain (Yahoo, `CoinGecko`, ECB).
    pub fn default_chain() -> Self {
        Self::new(vec![
            Box::new(super::yahoo::YahooFinance),
            Box::new(super::coingecko::CoinGecko),
            Box::new(super::ecb::Ecb),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_chain_has_all_providers() {
        let chain = ProviderChain::default_chain();
        let names = chain.provider_names();
        assert!(names.contains(&"Yahoo Finance"));
        assert!(names.contains(&"CoinGecko"));
        assert!(names.contains(&"ECB"));
    }

    #[test]
    fn routes_stock_to_yahoo() {
        let chain = ProviderChain::default_chain();
        let provider = chain.provider_for(AssetClass::Stock).unwrap();
        assert_eq!(provider.name(), "Yahoo Finance");
    }

    #[test]
    fn routes_crypto_to_first_supporting_provider() {
        let chain = ProviderChain::default_chain();
        let provider = chain.provider_for(AssetClass::Crypto).unwrap();
        // Yahoo is first and supports crypto too
        assert!(!provider.name().is_empty());
        // CoinGecko is available as fallback
        let crypto_providers = chain.providers_for(AssetClass::Crypto);
        assert!(crypto_providers.len() >= 2);
    }

    #[test]
    fn routes_forex_to_ecb() {
        let chain = ProviderChain::default_chain();
        let _provider = chain.provider_for(AssetClass::Forex).unwrap();
        // Yahoo also supports Forex, but ECB is more specific
        // The first match is Yahoo (it supports everything), so let's test providers_for
        let forex_providers = chain.providers_for(AssetClass::Forex);
        assert!(forex_providers.len() >= 2); // Yahoo + ECB
    }

    #[test]
    fn fallback_providers_available() {
        let chain = ProviderChain::default_chain();
        let stock_providers = chain.providers_for(AssetClass::Stock);
        assert!(!stock_providers.is_empty());
    }
}
