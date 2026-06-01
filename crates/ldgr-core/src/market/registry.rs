//! Provider registry for discovery and listing.
//!
//! The registry stores all available `QuoteProvider` implementations
//! and supports lookup by ID, asset class, or listing all metadata.
//! It is separate from [`ProviderChain`](super::ProviderChain), which
//! handles routing and fallback during actual data fetching.

use super::types::{AssetClass, MarketError, ProviderMetadata, QuoteProvider};

/// A registry of market data providers.
///
/// Providers are registered by ID and can be looked up individually or
/// filtered by asset class. Use [`ProviderRegistry::default_registry`]
/// to get the built-in providers (Yahoo Finance, `CoinGecko`, ECB).
///
/// Community providers register themselves via [`register`](Self::register).
pub struct ProviderRegistry {
    providers: Vec<Box<dyn QuoteProvider>>,
}

impl ProviderRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a provider. Returns an error if a provider with the
    /// same metadata ID is already registered.
    pub fn register(&mut self, provider: Box<dyn QuoteProvider>) -> Result<(), MarketError> {
        let id = provider.metadata().id;
        if self.providers.iter().any(|p| p.metadata().id == id) {
            return Err(MarketError::DuplicateProvider(id.to_string()));
        }
        self.providers.push(provider);
        Ok(())
    }

    /// Look up a provider by its metadata ID.
    pub fn get_by_id(&self, id: &str) -> Option<&dyn QuoteProvider> {
        self.providers
            .iter()
            .find(|p| p.metadata().id == id)
            .map(AsRef::as_ref)
    }

    /// List metadata for all registered providers.
    pub fn list_all(&self) -> Vec<ProviderMetadata> {
        self.providers.iter().map(|p| p.metadata()).collect()
    }

    /// Find providers that support a given asset class.
    pub fn for_asset_class(&self, class: AssetClass) -> Vec<&dyn QuoteProvider> {
        self.providers
            .iter()
            .filter(|p| p.supported_asset_classes().contains(&class))
            .map(AsRef::as_ref)
            .collect()
    }

    /// Number of registered providers.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Create a registry pre-loaded with the built-in providers.
    pub fn default_registry() -> Self {
        let mut registry = Self::new();
        for provider in super::types::builtin_providers() {
            // Built-in providers have unique IDs by construction.
            let _ = registry.register(provider);
        }
        registry
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_builtins() {
        let reg = ProviderRegistry::default_registry();
        assert_eq!(reg.len(), 3);

        let names: Vec<_> = reg.list_all().iter().map(|m| m.id).collect();
        assert!(names.contains(&"yahoo-finance"));
        assert!(names.contains(&"coingecko"));
        assert!(names.contains(&"ecb"));
    }

    #[test]
    fn get_by_id() {
        let reg = ProviderRegistry::default_registry();
        let yahoo = reg.get_by_id("yahoo-finance").unwrap();
        assert_eq!(yahoo.name(), "Yahoo Finance");
        assert!(reg.get_by_id("nonexistent").is_none());
    }

    #[test]
    fn for_asset_class_filters() {
        let reg = ProviderRegistry::default_registry();

        let crypto = reg.for_asset_class(AssetClass::Crypto);
        assert!(crypto.len() >= 2); // Yahoo + CoinGecko

        let forex = reg.for_asset_class(AssetClass::Forex);
        assert!(forex.len() >= 2); // Yahoo + ECB
    }

    #[test]
    fn duplicate_id_rejected() {
        let mut reg = ProviderRegistry::default_registry();
        let result = reg.register(Box::new(super::super::yahoo::YahooFinance));
        assert!(result.is_err());
        assert_eq!(reg.len(), 3); // unchanged
    }

    #[test]
    fn metadata_fields_populated() {
        let reg = ProviderRegistry::default_registry();
        let meta = reg.list_all();

        for m in &meta {
            assert!(!m.id.is_empty());
            assert!(!m.display_name.is_empty());
            assert!(!m.description.is_empty());
            assert!(!m.url.is_empty());
        }

        let yahoo = meta.iter().find(|m| m.id == "yahoo-finance").unwrap();
        assert!(yahoo.tos_url.is_some());
        assert!(yahoo.rate_limit_hint.is_some());
        assert!(!yahoo.requires_api_key);
    }
}
