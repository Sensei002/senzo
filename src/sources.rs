//! Provider-neutral source discovery boundary.
//!
//! The UI and playback pipeline consume these results rather than depending on a
//! particular provider protocol. New authorized adapters can be registered here
//! without changing the metadata client or MPV integration.

use std::{future::Future, pin::Pin};

use crate::{
    addons::{AddonClient, StremioStream},
    storage::StoredAddon,
};

#[derive(Debug, Clone)]
pub struct SourceProviderResult {
    /// Stable adapter/provider identity, not a display name.
    pub provider_id: String,
    pub provider_name: String,
    pub streams: Vec<StremioStream>,
    pub error: Option<String>,
}

pub trait SourceAdapter: Send + Sync {
    fn id(&self) -> &'static str;

    fn discover<'a>(
        &'a self,
        addons: &'a [StoredAddon],
        media_type: &'a str,
        media_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Vec<SourceProviderResult>> + Send + 'a>>;
}

/// First adapter: explicitly installed and enabled Stremio-compatible HTTP add-ons.
/// It deliberately does not install, bundle, or enable any provider.
pub struct StremioAddonAdapter {
    client: AddonClient,
}

impl StremioAddonAdapter {
    pub fn new() -> Self {
        Self {
            client: AddonClient::new(),
        }
    }
}

impl Default for StremioAddonAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceAdapter for StremioAddonAdapter {
    fn id(&self) -> &'static str {
        "stremio-addon"
    }

    fn discover<'a>(
        &'a self,
        addons: &'a [StoredAddon],
        media_type: &'a str,
        media_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Vec<SourceProviderResult>> + Send + 'a>> {
        Box::pin(async move {
            self.client
                .discover_streams(addons, media_type, media_id)
                .await
                .into_iter()
                .map(|result| SourceProviderResult {
                    provider_id: result.addon_id,
                    provider_name: result.addon_name,
                    streams: result.streams,
                    error: result.error,
                })
                .collect()
        })
    }
}

/// Registry for provider adapters. Adapters are queried concurrently; failures are
/// isolated to their own result so a broken provider cannot hide healthy results.
pub struct SourceProviderRegistry {
    adapters: Vec<Box<dyn SourceAdapter>>,
}

impl SourceProviderRegistry {
    pub fn new() -> Self {
        Self {
            adapters: vec![Box::new(StremioAddonAdapter::new())],
        }
    }

    pub fn adapter_ids(&self) -> Vec<&'static str> {
        self.adapters.iter().map(|adapter| adapter.id()).collect()
    }

    pub fn register(&mut self, adapter: Box<dyn SourceAdapter>) {
        if self
            .adapters
            .iter()
            .any(|existing| existing.id() == adapter.id())
        {
            return;
        }
        self.adapters.push(adapter);
    }

    pub async fn discover(
        &self,
        addons: &[StoredAddon],
        media_type: &str,
        media_id: &str,
    ) -> Vec<SourceProviderResult> {
        let requests = self
            .adapters
            .iter()
            .map(|adapter| adapter.discover(addons, media_type, media_id));
        futures::future::join_all(requests)
            .await
            .into_iter()
            .flatten()
            .collect()
    }
}

impl Default for SourceProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_registers_stremio_without_preinstalling_any_addon() {
        let registry = SourceProviderRegistry::default();
        assert_eq!(registry.adapter_ids(), vec!["stremio-addon"]);
    }

    struct EmptyAdapter;

    impl SourceAdapter for EmptyAdapter {
        fn id(&self) -> &'static str {
            "test-empty"
        }

        fn discover<'a>(
            &'a self,
            _addons: &'a [StoredAddon],
            _media_type: &'a str,
            _media_id: &'a str,
        ) -> Pin<Box<dyn Future<Output = Vec<SourceProviderResult>> + Send + 'a>> {
            Box::pin(async { Vec::new() })
        }
    }

    #[test]
    fn adapters_can_be_registered_without_changing_the_registry_contract() {
        let mut registry = SourceProviderRegistry::default();
        registry.register(Box::new(EmptyAdapter));
        registry.register(Box::new(EmptyAdapter));
        assert_eq!(registry.adapter_ids(), vec!["stremio-addon", "test-empty"]);
    }
}
