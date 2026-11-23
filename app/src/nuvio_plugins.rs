//! Nuvio-compatible plugin repositories.
//!
//! SENZO keeps the repository/provider metadata local. A repository is a
//! manifest URL; its providers are individual scripts. Installing a repository
//! never executes a script and never enables a provider automatically.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use url::Url;

use crate::{MainWindow, direct_sources::net::is_blocked_host};

pub const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/D3adlyRocket/All-in-One-Nuvio/refs/heads/main/manifest.json";
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_SCRIPT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    pub enabled: bool,
    pub group_by_repository: bool,
    pub repositories: Vec<PluginRepository>,
    pub providers: Vec<PluginProvider>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            group_by_repository: false,
            repositories: vec![PluginRepository::default()],
            providers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginRepository {
    pub manifest_url: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub last_updated: u64,
}

impl Default for PluginRepository {
    fn default() -> Self {
        Self {
            manifest_url: DEFAULT_MANIFEST_URL.to_owned(),
            name: "All-in-One-Nuvio".to_owned(),
            version: String::new(),
            description: "Community Nuvio provider repository.".to_owned(),
            last_updated: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginProvider {
    /// Stable local key: repository URL + manifest provider id.
    pub key: String,
    pub id: String,
    pub repository_url: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub filename: String,
    pub source_url: String,
    pub supported_types: Vec<String>,
    pub enabled: bool,
    pub manifest_enabled: bool,
    pub has_settings: bool,
    pub logo: String,
    pub languages: Vec<String>,
    pub formats: Vec<String>,
}

impl Default for PluginProvider {
    fn default() -> Self {
        Self {
            key: String::new(),
            id: String::new(),
            repository_url: String::new(),
            name: String::new(),
            description: String::new(),
            version: String::new(),
            filename: String::new(),
            source_url: String::new(),
            supported_types: vec!["movie".to_owned(), "series".to_owned()],
            enabled: false,
            manifest_enabled: true,
            has_settings: false,
            logo: String::new(),
            languages: Vec::new(),
            formats: Vec::new(),
        }
    }
}

impl PluginProvider {
    pub fn supports_type(&self, media_type: &str) -> bool {
        let normalized = normalize_type(media_type);
        self.supported_types
            .iter()
            .any(|value| normalize_type(value) == normalized)
    }
}

fn normalize_type(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "series" | "show" | "other" => "tv".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    scrapers: Vec<ManifestScraper>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestScraper {
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: String,
    #[serde(default)]
    filename: String,
    #[serde(default = "default_supported_types")]
    supported_types: Vec<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    has_settings: bool,
    #[serde(default)]
    logo: Option<String>,
    #[serde(default)]
    content_language: Option<Vec<String>>,
    #[serde(default)]
    formats: Option<Vec<String>>,
}

fn default_supported_types() -> Vec<String> {
    vec!["movie".to_owned(), "tv".to_owned()]
}

fn default_true() -> bool {
    true
}

#[derive(Clone)]
pub struct NuvioPlugins {
    inner: Arc<Inner>,
}

struct Inner {
    ui: slint::Weak<MainWindow>,
    config: RwLock<PluginsConfig>,
    status: RwLock<HashMap<String, (String, bool)>>,
    busy: RwLock<bool>,
}

static MANAGER: OnceLock<NuvioPlugins> = OnceLock::new();

impl NuvioPlugins {
    fn config(&self) -> PluginsConfig {
        self.inner
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn replace_config(&self, config: PluginsConfig) {
        *self
            .inner
            .config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config.clone();
        crate::config::save_config(&crate::config::AppConfig {
            plugins: config,
            ..crate::config::load_config()
        });
    }

    fn status(&self) -> HashMap<String, (String, bool)> {
        self.inner
            .status
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set_status(&self, key: &str, message: impl Into<String>, error: bool) {
        self.inner
            .status
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.to_owned(), (message.into(), error));
    }

    fn set_busy(&self, busy: bool) {
        *self
            .inner
            .busy
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = busy;
        self.project();
    }

    fn project(&self) {
        let Some(ui) = self.inner.ui.upgrade() else {
            return;
        };
        let config = self.config();
        let status = self.status();
        let busy = *self
            .inner
            .busy
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let repositories = config
            .repositories
            .iter()
            .map(|repository| {
                let provider_count = config
                    .providers
                    .iter()
                    .filter(|provider| provider.repository_url == repository.manifest_url)
                    .count();
                let (message, error) = status
                    .get(&repository.manifest_url)
                    .cloned()
                    .unwrap_or_else(|| ("Not refreshed".to_owned(), false));
                crate::PluginRepositoryCard {
                    url: repository.manifest_url.clone().into(),
                    name: repository.name.clone().into(),
                    version: if repository.version.is_empty() {
                        "—".into()
                    } else {
                        repository.version.clone().into()
                    },
                    provider_count: i32::try_from(provider_count).unwrap_or(i32::MAX),
                    status: message.into(),
                    status_error: error,
                }
            })
            .collect::<Vec<_>>();

        let mut providers = config.providers.clone();
        providers.sort_by(|left, right| {
            if config.group_by_repository {
                left.repository_url
                    .cmp(&right.repository_url)
                    .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            } else {
                left.name.to_lowercase().cmp(&right.name.to_lowercase())
            }
        });
        let providers = providers
            .into_iter()
            .map(|provider| {
                let (message, error) = status.get(&provider.key).cloned().unwrap_or_else(|| {
                    if provider.enabled {
                        ("Enabled".to_owned(), false)
                    } else {
                        ("Disabled".to_owned(), false)
                    }
                });
                crate::PluginProviderCard {
                    id: provider.key.clone().into(),
                    name: provider.name.clone().into(),
                    description: if provider.description.is_empty() {
                        "No description provided.".into()
                    } else {
                        provider.description.clone().into()
                    },
                    repository: provider.repository_url.clone().into(),
                    version: if provider.version.is_empty() {
                        "—".into()
                    } else {
                        provider.version.clone().into()
                    },
                    content_types: provider.supported_types.join(", ").into(),
                    languages: if provider.languages.is_empty() {
                        "unknown".into()
                    } else {
                        provider.languages.join(", ").into()
                    },
                    formats: if provider.formats.is_empty() {
                        "auto".into()
                    } else {
                        provider.formats.join(", ").into()
                    },
                    status: message.into(),
                    status_error: error,
                    enabled: provider.enabled,
                    has_settings: provider.has_settings,
                }
            })
            .collect::<Vec<_>>();

        ui.set_plugins_master_enabled(config.enabled);
        ui.set_plugins_group_by_repository(config.group_by_repository);
        ui.set_plugins_busy(busy);
        ui.set_plugin_repositories(slint::ModelRc::new(slint::VecModel::from(repositories)));
        ui.set_plugin_providers(slint::ModelRc::new(slint::VecModel::from(providers)));
    }

    fn set_message(&self, message: &str, error: bool) {
        if let Some(ui) = self.inner.ui.upgrade() {
            ui.set_plugins_status(message.into());
            ui.set_plugins_status_error(error);
        }
    }

    pub async fn refresh_repository(&self, manifest_url: &str) {
        self.set_busy(true);
        self.set_status(manifest_url, "Refreshing…", false);
        self.set_message("Fetching plugin manifest…", false);
        match fetch_manifest(manifest_url).await {
            Ok(manifest) => {
                let mut config = self.config();
                let Some(repository) = config
                    .repositories
                    .iter_mut()
                    .find(|repository| repository.manifest_url == manifest_url)
                else {
                    self.set_status(manifest_url, "Repository is not installed", true);
                    self.set_busy(false);
                    return;
                };
                repository.name = manifest.name;
                repository.version = manifest.version;
                repository.description = manifest.description.unwrap_or_default();
                repository.last_updated = now_seconds();

                let previous = config
                    .providers
                    .iter()
                    .filter(|provider| provider.repository_url == manifest_url)
                    .map(|provider| (provider.key.clone(), provider.clone()))
                    .collect::<HashMap<_, _>>();
                config
                    .providers
                    .retain(|provider| provider.repository_url != manifest_url);
                for scraper in manifest.scrapers {
                    if scraper.id.trim().is_empty() || scraper.name.trim().is_empty() {
                        continue;
                    }
                    let key = format!("{manifest_url}#{}", scraper.id);
                    let old = previous.get(&key);
                    let source_url = resolve_source_url(manifest_url, &scraper.filename)
                        .unwrap_or_else(|_| String::new());
                    config.providers.push(PluginProvider {
                        key: key.clone(),
                        id: scraper.id,
                        repository_url: manifest_url.to_owned(),
                        name: scraper.name,
                        description: scraper.description.unwrap_or_default(),
                        version: scraper.version,
                        filename: scraper.filename,
                        source_url,
                        supported_types: scraper.supported_types,
                        enabled: old.is_some_and(|provider| provider.enabled),
                        manifest_enabled: scraper.enabled,
                        has_settings: scraper.has_settings,
                        logo: scraper.logo.unwrap_or_default(),
                        languages: scraper.content_language.unwrap_or_default(),
                        formats: scraper.formats.unwrap_or_default(),
                    });
                }
                config
                    .repositories
                    .sort_by(|left, right| left.name.cmp(&right.name));
                config.providers.sort_by(|left, right| {
                    left.repository_url
                        .cmp(&right.repository_url)
                        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                });
                let count = config
                    .providers
                    .iter()
                    .filter(|provider| provider.repository_url == manifest_url)
                    .count();
                self.replace_config(config);
                self.set_status(manifest_url, format!("{count} providers ready"), false);
                self.set_message(&format!("Loaded {} plugin provider(s).", count), false);
                self.set_busy(false);
            }
            Err(error) => {
                self.set_status(manifest_url, error.clone(), true);
                self.set_message(&error, true);
                self.set_busy(false);
            }
        }
    }

    async fn add_repository(&self, raw_url: &str) {
        let Ok(manifest_url) = validate_manifest_url(raw_url) else {
            self.set_message("Enter a valid HTTPS plugin manifest URL.", true);
            return;
        };
        if self
            .config()
            .repositories
            .iter()
            .any(|repository| repository.manifest_url == manifest_url)
        {
            self.set_message("That plugin repository is already installed.", true);
            return;
        }
        self.set_busy(true);
        self.set_message("Installing plugin repository…", false);
        match fetch_manifest(&manifest_url).await {
            Ok(manifest) => {
                let mut config = self.config();
                config.repositories.push(PluginRepository {
                    manifest_url: manifest_url.clone(),
                    name: manifest.name,
                    version: manifest.version,
                    description: manifest.description.unwrap_or_default(),
                    last_updated: now_seconds(),
                });
                self.replace_config(config);
                if let Some(ui) = self.inner.ui.upgrade() {
                    ui.set_plugins_repository_url(String::new().into());
                }
                self.set_busy(false);
                self.refresh_repository(&manifest_url).await;
            }
            Err(error) => {
                self.set_message(&error, true);
                self.set_busy(false);
            }
        }
    }

    fn remove_repository(&self, manifest_url: &str) {
        let mut config = self.config();
        config
            .repositories
            .retain(|repository| repository.manifest_url != manifest_url);
        config
            .providers
            .retain(|provider| provider.repository_url != manifest_url);
        self.replace_config(config);
        self.set_message("Plugin repository removed.", false);
        self.project();
    }

    fn toggle_master(&self, enabled: bool) {
        let mut config = self.config();
        config.enabled = enabled;
        self.replace_config(config);
        self.project();
    }

    fn toggle_group(&self, grouped: bool) {
        let mut config = self.config();
        config.group_by_repository = grouped;
        self.replace_config(config);
        self.project();
    }

    fn toggle_provider(&self, key: &str, enabled: bool) {
        let mut config = self.config();
        let Some(provider) = config
            .providers
            .iter_mut()
            .find(|provider| provider.key == key)
        else {
            return;
        };
        if enabled && !provider.manifest_enabled {
            self.set_message(
                "That provider is disabled by its repository manifest.",
                true,
            );
            self.project();
            return;
        }
        provider.enabled = enabled;
        self.replace_config(config);
        self.set_status(key, if enabled { "Enabled" } else { "Disabled" }, false);
        self.project();
    }

    async fn test_provider(&self, key: &str) {
        let Some(provider) = self
            .config()
            .providers
            .into_iter()
            .find(|provider| provider.key == key)
        else {
            return;
        };
        self.set_status(key, "Fetching script…", false);
        self.set_message("Fetching plugin script…", false);
        match fetch_script(&provider.source_url).await {
            Ok(source) => {
                if source.to_lowercase().contains("getstreams") {
                    self.set_status(key, "Ready", false);
                    self.set_message("Plugin script loaded successfully.", false);
                } else {
                    self.set_status(key, "No getStreams export", true);
                    self.set_message("The script does not expose getStreams.", true);
                }
            }
            Err(error) => {
                self.set_status(key, error.clone(), true);
                self.set_message(&error, true);
            }
        }
        self.project();
    }

    /// Enabled providers for a media type. Used by the stream drawer and by
    /// the optional JavaScript runner.
    pub fn enabled_providers(&self, media_type: &str) -> Vec<PluginProvider> {
        let config = self.config();
        if !config.enabled {
            return Vec::new();
        }
        config
            .providers
            .into_iter()
            .filter(|provider| provider.enabled && provider.supports_type(media_type))
            .collect()
    }
}

pub fn setup(ui: &MainWindow, config: &PluginsConfig) {
    let manager = NuvioPlugins {
        inner: Arc::new(Inner {
            ui: ui.as_weak(),
            config: RwLock::new(config.clone()),
            status: RwLock::new(HashMap::new()),
            busy: RwLock::new(false),
        }),
    };
    let _ = MANAGER.set(manager.clone());
    manager.project();

    ui.on_plugins_master_changed({
        let manager = manager.clone();
        move |enabled| manager.toggle_master(enabled)
    });
    ui.on_plugins_group_changed({
        let manager = manager.clone();
        move |grouped| manager.toggle_group(grouped)
    });
    ui.on_plugin_repository_add({
        let manager = manager.clone();
        move |url| {
            let manager = manager.clone();
            let url = url.to_string();
            tokio::spawn(async move { manager.add_repository(url.as_str()).await });
        }
    });
    ui.on_plugin_repository_refresh({
        let manager = manager.clone();
        move |url| {
            let manager = manager.clone();
            let url = url.to_string();
            tokio::spawn(async move { manager.refresh_repository(url.as_str()).await });
        }
    });
    ui.on_plugin_repository_remove({
        let manager = manager.clone();
        move |url| manager.remove_repository(url.as_str())
    });
    ui.on_plugin_provider_toggled({
        let manager = manager.clone();
        move |key, enabled| manager.toggle_provider(key.as_str(), enabled)
    });
    ui.on_plugin_provider_tested({
        let manager = manager.clone();
        move |key| {
            let manager = manager.clone();
            let key = key.to_string();
            tokio::spawn(async move { manager.test_provider(key.as_str()).await });
        }
    });
    ui.on_plugin_refresh_all({
        let manager = manager.clone();
        move || {
            let manager = manager.clone();
            let repositories = manager.config().repositories;
            tokio::spawn(async move {
                for repository in repositories {
                    manager.refresh_repository(&repository.manifest_url).await;
                }
            });
        }
    });
}

pub fn manager() -> Option<NuvioPlugins> {
    MANAGER.get().cloned()
}

/// Called when the Plugins tab is opened for the first time. It fetches a
/// repository that has metadata but no provider records; a refresh button is
/// always available afterwards.
pub fn ensure_loaded() {
    let Some(manager) = manager() else {
        return;
    };
    let config = manager.config();
    if !config.enabled {
        return;
    }
    let status = manager.status();
    let empty = config
        .repositories
        .iter()
        .filter(|repository| {
            !config
                .providers
                .iter()
                .any(|provider| provider.repository_url == repository.manifest_url)
                && !status
                    .get(&repository.manifest_url)
                    .is_some_and(|(message, _)| message.starts_with("Refreshing"))
        })
        .map(|repository| repository.manifest_url.clone())
        .collect::<Vec<_>>();
    for manifest_url in empty {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager.refresh_repository(&manifest_url).await;
        });
    }
}

fn validate_manifest_url(raw_url: &str) -> Result<String, ()> {
    let url = Url::parse(raw_url.trim()).map_err(|_| ())?;
    if url.scheme() != "https" || url.host_str().is_none() || is_blocked_host(&url) {
        return Err(());
    }
    Ok(url.to_string())
}

fn resolve_source_url(manifest_url: &str, filename: &str) -> Result<String, ()> {
    if filename.trim().is_empty() {
        return Err(());
    }
    let base = Url::parse(manifest_url).map_err(|_| ())?;
    let resolved = base.join(filename).map_err(|_| ())?;
    if resolved.scheme() != "https" || is_blocked_host(&resolved) {
        return Err(());
    }
    Ok(resolved.to_string())
}

async fn fetch_text(url: &str, limit: usize) -> Result<String, String> {
    let parsed = Url::parse(url).map_err(|_| "Invalid plugin URL.".to_owned())?;
    if parsed.scheme() != "https" || is_blocked_host(&parsed) {
        return Err("Plugin repositories must use HTTPS on a public host.".to_owned());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || is_blocked_host(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .user_agent("SENZO-PluginManager/1.0 (+https://github.com/Sensei002/senzo)")
        .build()
        .map_err(|error| format!("Could not create plugin HTTP client: {error}"))?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| format!("Plugin request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Plugin server returned HTTP {}.",
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("Could not read plugin response: {error}"))?;
    if bytes.len() > limit {
        return Err("Plugin response is larger than the allowed limit.".to_owned());
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| "Plugin response is not valid UTF-8.".to_owned())
}

async fn fetch_manifest(manifest_url: &str) -> Result<Manifest, String> {
    let text = fetch_text(manifest_url, MAX_MANIFEST_BYTES).await?;
    serde_json::from_str(&text).map_err(|error| format!("Invalid plugin manifest: {error}"))
}

async fn fetch_script(source_url: &str) -> Result<String, String> {
    fetch_text(source_url, MAX_SCRIPT_BYTES).await
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_defaults_are_disabled() {
        let provider = PluginProvider::default();
        assert!(!provider.enabled);
        assert!(provider.supports_type("series"));
        assert!(provider.supports_type("tv"));
    }

    #[test]
    fn manifest_urls_must_be_https() {
        assert!(validate_manifest_url("http://example.com/manifest.json").is_err());
        assert!(validate_manifest_url("https://example.com/manifest.json").is_ok());
    }

    #[test]
    fn relative_scripts_are_resolved() {
        assert_eq!(
            resolve_source_url(
                "https://example.com/repo/manifest.json",
                "providers/example.js"
            )
            .unwrap(),
            "https://example.com/repo/providers/example.js"
        );
    }
}
