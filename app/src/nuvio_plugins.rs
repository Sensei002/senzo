//! Nuvio-compatible plugin repositories.
//!
//! SENZO keeps the repository/provider metadata local. A repository is a
//! manifest URL; its providers are individual scripts. Installing a repository
//! never executes a script and never enables a provider automatically. Enabled
//! providers are executed on demand in the bounded QuickJS runtime.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use futures::{
    StreamExt,
    stream::{self, FuturesUnordered},
};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model};
use tokio::sync::Semaphore;
use url::Url;

use crate::{
    MainWindow,
    app_model::AppModel,
    direct_sources::net::is_blocked_host,
    playback::{PlaybackSelections, PluginSelection},
    plugin_runtime::{
        self, PluginStream, ProviderQuery,
        net::{PublicHost, classify_public_host, is_public_url},
    },
};

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
    /// Provider-owned settings JSON. Scripts read it as SCRAPER_SETTINGS.
    #[serde(default = "default_settings")]
    pub settings: String,
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
            settings: default_settings(),
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

/// A provider ready to be stored, after CloudStream build metadata has been
/// normalized onto SENZO's provider record.
#[derive(Debug)]
struct ManifestScraper {
    id: String,
    name: String,
    description: Option<String>,
    version: String,
    filename: String,
    supported_types: Vec<String>,
    enabled: bool,
    has_settings: bool,
    logo: Option<String>,
    content_language: Option<Vec<String>>,
    formats: Option<Vec<String>>,
}

/// Everything one install or refresh contributed.
#[derive(Debug, Default)]
struct RepositoryContent {
    name: String,
    version: String,
    description: String,
    /// Providers are paired with the document that declared them because a
    /// relative `filename` resolves against that document, not the repository
    /// URL the user pasted.
    scrapers: Vec<(String, ManifestScraper)>,
}

/// Tolerant view of one manifest document.
///
/// Community repositories publish three shapes: a manifest with a `scrapers`
/// array, an index with `pluginLists` pointing at those manifests, and a bare
/// array of plugin entries (what build branches usually serve). Unknown fields
/// are ignored and every field is optional so one odd repository cannot make
/// the whole install fail.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RawManifest {
    name: Option<String>,
    version: Option<serde_json::Value>,
    description: Option<String>,
    scrapers: Vec<RawScraper>,
    plugin_lists: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RawScraper {
    id: Option<String>,
    name: Option<String>,
    internal_name: Option<String>,
    description: Option<String>,
    version: Option<serde_json::Value>,
    filename: Option<String>,
    url: Option<String>,
    supported_types: Vec<String>,
    tv_types: Vec<String>,
    enabled: Option<bool>,
    status: Option<i64>,
    has_settings: Option<bool>,
    logo: Option<String>,
    icon_url: Option<String>,
    content_language: Option<serde_json::Value>,
    language: Option<String>,
    formats: Vec<String>,
}

fn default_supported_types() -> Vec<String> {
    vec!["movie".to_owned(), "tv".to_owned()]
}

fn json_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.trim().to_owned(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

/// CloudStream spells content kinds inconsistently (`TvSeries`, `tv`, `Live`).
fn normalize_content_type(value: &str) -> String {
    let kind = value.trim().to_ascii_lowercase();
    match kind.as_str() {
        "movie" | "movies" | "film" | "films" => "movie".to_owned(),
        "series" | "tv" | "tvseries" | "tv series" | "tvshow" | "tv shows" | "show" | "shows" => {
            "tv".to_owned()
        }
        "live" | "livetv" | "live tv" | "channel" | "channels" => "live".to_owned(),
        "anime" => "anime".to_owned(),
        "nsfw" | "adult" => "nsfw".to_owned(),
        _ => kind,
    }
}

impl RawScraper {
    /// Returns `None` for entries that cannot describe a runnable provider.
    fn normalize(self) -> Option<ManifestScraper> {
        let name = self
            .name
            .or(self.internal_name.clone())
            .unwrap_or_default()
            .trim()
            .to_owned();
        let filename = self
            .filename
            .or(self.url)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if name.is_empty() || filename.is_empty() {
            return None;
        }
        let id = self
            .id
            .or(self.internal_name)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| name.clone());
        let mut supported_types = Vec::new();
        for kind in self.supported_types.iter().chain(self.tv_types.iter()) {
            let kind = normalize_content_type(kind);
            if !kind.is_empty() && !supported_types.contains(&kind) {
                supported_types.push(kind);
            }
        }
        if supported_types.is_empty() {
            supported_types = default_supported_types();
        }
        let languages = match self.content_language {
            Some(serde_json::Value::Array(values)) => values
                .iter()
                .filter_map(|value| value.as_str())
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>(),
            Some(serde_json::Value::String(text)) if !text.trim().is_empty() => {
                vec![text.trim().to_owned()]
            }
            _ => self
                .language
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty())
                .into_iter()
                .collect(),
        };
        Some(ManifestScraper {
            id,
            name,
            description: self.description,
            version: self.version.as_ref().map(json_text).unwrap_or_default(),
            filename,
            supported_types,
            enabled: self
                .enabled
                .or_else(|| self.status.map(|status| status != 0))
                .unwrap_or(true),
            has_settings: self.has_settings.unwrap_or(false),
            logo: self.logo.or(self.icon_url),
            content_language: Some(languages).filter(|languages| !languages.is_empty()),
            formats: Some(self.formats).filter(|formats| !formats.is_empty()),
        })
    }
}

fn parse_scraper(value: serde_json::Value) -> Option<ManifestScraper> {
    serde_json::from_value::<RawScraper>(value)
        .ok()
        .and_then(RawScraper::normalize)
}

/// Filter labels mirror the options offered by the Plugins page.
fn provider_matches_filter(provider: &PluginProvider, filter: &str) -> bool {
    match filter {
        "Enabled" => provider.enabled,
        "Movies" => provider.supports_type("movie"),
        "Series" => provider.supports_type("tv"),
        "Live TV" => provider.supports_type("live"),
        "Anime" => provider.supports_type("anime"),
        "Other" => {
            !(provider.supports_type("movie")
                || provider.supports_type("tv")
                || provider.supports_type("live")
                || provider.supports_type("anime"))
        }
        _ => true,
    }
}

/// The label shown for "no repository filter".
const ALL_REPOSITORIES: &str = "All repositories";

/// Option labels for the repository filter, paired with the repository URL.
///
/// Repository names are not unique (several community projects publish under
/// the same name), so repeated labels get a numeric suffix and the label stays
/// a stable key for the selected repository.
fn repository_filter_labels(repositories: &[PluginRepository]) -> Vec<(String, String)> {
    let mut labels: Vec<(String, String)> = Vec::with_capacity(repositories.len());
    for repository in repositories {
        let base = if repository.name.trim().is_empty() {
            manifest_fallback_name(&repository.manifest_url)
        } else {
            repository.name.trim().to_owned()
        };
        let mut label = base.clone();
        let mut suffix = 2;
        while labels.iter().any(|(existing, _)| existing == &label) {
            label = format!("{base} ({suffix})");
            suffix += 1;
        }
        labels.push((label, repository.manifest_url.clone()));
    }
    labels
}

fn provider_matches_query(provider: &PluginProvider, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut haystack = String::new();
    for part in [
        provider.name.as_str(),
        provider.description.as_str(),
        provider.id.as_str(),
    ] {
        haystack.push_str(part);
        haystack.push(' ');
    }
    for part in provider.supported_types.iter().chain(provider.languages.iter()) {
        haystack.push_str(part);
        haystack.push(' ');
    }
    let haystack = haystack.to_lowercase();
    query.split_whitespace().all(|term| haystack.contains(term))
}

fn manifest_fallback_name(manifest_url: &str) -> String {
    Url::parse(manifest_url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.rfind(|part| !part.is_empty()))
                .map(|segment| segment.to_owned())
        })
        .unwrap_or_else(|| "Plugin repository".to_owned())
}

fn default_settings() -> String {
    "{}".to_owned()
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
    /// Provider list filters. The UI owns the values; the manager keeps a copy
    /// so a refresh or a toggle does not reset what the user is looking at.
    provider_filter: RwLock<String>,
    provider_query: RwLock<String>,
    /// Selected "repository" filter, stored as the option label the UI shows.
    /// Empty means every installed repository.
    repository_filter: RwLock<String>,
    runtime: Arc<stremio_core::runtime::Runtime<core_env::DesktopEnv, AppModel>>,
    playback: Arc<PlaybackSelections>,
    generation: AtomicU64,
    last_query: Mutex<Option<String>>,
    scripts: Mutex<HashMap<String, (Instant, String)>>,
    tmdb_ids: Mutex<HashMap<String, (Instant, String)>>,
    results: Mutex<HashMap<String, (Instant, Vec<PluginStream>)>>,
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

    fn set_provider_filter(&self, filter: String) {
        *self
            .inner
            .provider_filter
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = filter;
        self.project();
    }

    fn set_provider_query(&self, query: String) {
        *self
            .inner
            .provider_query
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = query;
        self.project();
    }

    fn set_repository_filter(&self, filter: String) {
        *self
            .inner
            .repository_filter
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = filter;
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
                let enabled_count = config
                    .providers
                    .iter()
                    .filter(|provider| {
                        provider.repository_url == repository.manifest_url && provider.enabled
                    })
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
                    enabled_count: i32::try_from(enabled_count).unwrap_or(i32::MAX),
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
        let provider_total = providers.len();
        let repository_filters = repository_filter_labels(&config.repositories);
        let stored_repository = self
            .inner
            .repository_filter
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let repository_url = repository_filters
            .iter()
            .find(|(label, _)| label == &stored_repository)
            .map(|(_, url)| url.clone());
        // Keep the UI in step when the filtered repository was removed or
        // renamed: the filter falls back to every repository instead of
        // leaving a selection that matches nothing.
        let selected_repository = repository_url
            .as_ref()
            .and_then(|url| {
                repository_filters
                    .iter()
                    .find(|(_, candidate)| candidate == url)
                    .map(|(label, _)| label.clone())
            })
            .unwrap_or_default();
        if selected_repository != stored_repository {
            *self
                .inner
                .repository_filter
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = selected_repository.clone();
        }
        let filter = self
            .inner
            .provider_filter
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let query = self
            .inner
            .provider_query
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .trim()
            .to_lowercase();
        providers.retain(|provider| {
            provider_matches_filter(provider, &filter)
                && provider_matches_query(provider, &query)
                && repository_url
                    .as_deref()
                    .is_none_or(|url| provider.repository_url == url)
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
                    settings: provider.settings.clone().into(),
                }
            })
            .collect::<Vec<_>>();

        let mut repository_options = Vec::with_capacity(repository_filters.len() + 1);
        repository_options.push(ALL_REPOSITORIES.to_owned());
        repository_options.extend(repository_filters.into_iter().map(|(label, _)| label));

        ui.set_plugins_master_enabled(config.enabled);
        ui.set_plugins_group_by_repository(config.group_by_repository);
        ui.set_plugins_busy(busy);
        ui.set_plugins_repository_filters(slint::ModelRc::new(slint::VecModel::from(
            repository_options.into_iter().map(Into::into).collect::<Vec<slint::SharedString>>(),
        )));
        ui.set_plugins_repository_filter(
            if selected_repository.is_empty() {
                ALL_REPOSITORIES
            } else {
                selected_repository.as_str()
            }
            .into(),
        );
        // Keep the existing rows when the list keeps its length: replacing the
        // model rebuilds every provider row, which is slow for the repositories
        // that publish dozens of providers and discards an unsaved settings
        // draft in the other rows.
        let existing_repositories = ui.get_plugin_repositories();
        if existing_repositories.row_count() == repositories.len() {
            for (index, repository) in repositories.into_iter().enumerate() {
                existing_repositories.set_row_data(index, repository);
            }
        } else {
            ui.set_plugin_repositories(slint::ModelRc::new(slint::VecModel::from(repositories)));
        }
        let existing_providers = ui.get_plugin_providers();
        if existing_providers.row_count() == providers.len() {
            for (index, provider) in providers.into_iter().enumerate() {
                existing_providers.set_row_data(index, provider);
            }
        } else {
            ui.set_plugin_providers(slint::ModelRc::new(slint::VecModel::from(providers)));
        }
        ui.set_plugin_provider_total(i32::try_from(provider_total).unwrap_or(i32::MAX));
    }

    fn set_message(&self, message: &str, error: bool) {
        if let Some(ui) = self.inner.ui.upgrade() {
            ui.set_plugins_status(message.into());
            ui.set_plugins_status_error(error);
        }
    }

    pub async fn refresh_repository(&self, manifest_url: &str) {
        let installed = self
            .config()
            .repositories
            .iter()
            .any(|repository| repository.manifest_url == manifest_url);
        if !installed {
            self.set_status(manifest_url, "Repository is not installed", true);
            self.set_busy(false);
            return;
        }
        self.set_busy(true);
        self.set_status(manifest_url, "Refreshing…", false);
        self.set_message("Fetching plugin manifest…", false);
        match load_repository(manifest_url).await {
            Ok(content) => self.apply_repository(manifest_url, content),
            Err(error) => {
                self.set_status(manifest_url, error.clone(), true);
                self.set_message(&error, true);
                self.set_busy(false);
            }
        }
    }

    /// Merge a freshly loaded repository into the saved configuration.
    ///
    /// Providers are re-created on every refresh so a new version, name or file
    /// name is picked up, while the enabled flag and user settings survive
    /// because they are keyed by provider id.
    fn apply_repository(&self, manifest_url: &str, content: RepositoryContent) {
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
        repository.name = content.name;
        repository.version = content.version;
        repository.description = content.description;
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
        let mut seen = HashSet::new();
        for (base_url, scraper) in content.scrapers {
            if scraper.id.trim().is_empty() || scraper.name.trim().is_empty() {
                continue;
            }
            let key = format!("{manifest_url}#{}", scraper.id);
            if !seen.insert(key.clone()) {
                continue;
            }
            let old = previous.get(&key);
            let source_url = resolve_source_url(&base_url, &scraper.filename).unwrap_or_default();
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
                settings: old
                    .map(|provider| provider.settings.clone())
                    .unwrap_or_else(default_settings),
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
        self.invalidate_results();
        self.replace_config(config);
        self.set_status(manifest_url, format!("{count} providers ready"), false);
        self.set_message(&format!("Loaded {count} plugin provider(s)."), false);
        self.set_busy(false);
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
        match load_repository(&manifest_url).await {
            Ok(content) => {
                let mut config = self.config();
                config.repositories.push(PluginRepository {
                    manifest_url: manifest_url.clone(),
                    name: content.name.clone(),
                    version: content.version.clone(),
                    description: content.description.clone(),
                    last_updated: now_seconds(),
                });
                self.replace_config(config);
                if let Some(ui) = self.inner.ui.upgrade() {
                    ui.set_plugins_repository_url(String::new().into());
                }
                self.apply_repository(&manifest_url, content);
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
        self.invalidate_results();
        self.replace_config(config);
        self.set_message("Plugin repository removed.", false);
        self.project();
    }

    fn toggle_master(&self, enabled: bool) {
        let mut config = self.config();
        config.enabled = enabled;
        self.invalidate_results();
        self.replace_config(config);
        self.project();
    }

    fn invalidate_results(&self) {
        self.inner
            .results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        *self
            .inner
            .last_query
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.inner
            .scripts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
    }

    fn update_provider_settings(&self, key: &str, settings: &str) {
        if !serde_json::from_str::<serde_json::Value>(settings)
            .is_ok_and(|value| value.is_object())
        {
            self.set_message("Provider settings must be a JSON object.", true);
            return;
        }
        let mut config = self.config();
        let Some(provider) = config.providers.iter_mut().find(|provider| provider.key == key) else {
            return;
        };
        provider.settings = settings.to_owned();
        self.invalidate_results();
        self.replace_config(config);
        self.set_status(key, "Settings saved", false);
        self.set_message("Plugin settings saved.", false);
        self.project();
    }

    fn toggle_group(&self, grouped: bool) {
        let mut config = self.config();
        config.group_by_repository = grouped;
        self.replace_config(config);
        self.project();
    }

    /// Enable or disable every provider a repository publishes, which is the
    /// one-click path for repositories that ship dozens of scrapers.
    fn set_repository_enabled(&self, manifest_url: &str, enabled: bool) {
        let mut config = self.config();
        let mut changed = 0usize;
        let mut blocked = 0usize;
        for provider in &mut config.providers {
            if provider.repository_url != manifest_url {
                continue;
            }
            if enabled && !provider.manifest_enabled {
                blocked += 1;
                continue;
            }
            if provider.enabled != enabled {
                provider.enabled = enabled;
                changed += 1;
            }
        }
        if changed == 0 {
            let message = if blocked > 0 {
                "That repository does not allow its providers to be enabled."
            } else if enabled {
                "Every provider in that repository is already enabled."
            } else {
                "Every provider in that repository is already disabled."
            };
            self.set_message(message, blocked > 0);
            self.project();
            return;
        }
        self.invalidate_results();
        self.replace_config(config);
        self.set_message(
            if enabled {
                "All providers in that repository enabled."
            } else {
                "All providers in that repository disabled."
            },
            false,
        );
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
        self.invalidate_results();
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
                let settings = if serde_json::from_str::<serde_json::Value>(&provider.settings)
                    .is_ok_and(|value| value.is_object())
                {
                    provider.settings.clone()
                } else {
                    "{}".to_owned()
                };
                let key_for_check = provider.key.clone();
                let result = tokio::task::spawn_blocking(move || {
                    plugin_runtime::validate(&key_for_check, &source, &settings)
                })
                .await;
                match result {
                    Ok(Ok(())) => {
                        self.set_status(key, "Ready", false);
                        self.set_message("Plugin script compiled and exported getStreams.", false);
                    }
                    Ok(Err(error)) => {
                        self.set_status(key, truncate_message(&error, 90), true);
                        self.set_message(&error, true);
                    }
                    Err(_) => {
                        self.set_status(key, "Plugin test failed", true);
                        self.set_message("The plugin sandbox could not complete the test.", true);
                    }
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
    /// the JavaScript runner.
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

#[derive(Clone, Debug)]
struct DetailsQuery {
    key: String,
    meta_id: String,
    video_id: String,
    media_type: String,
    title: String,
    year: Option<String>,
    season: Option<u32>,
    episode: Option<u32>,
}

/// Start a provider query when the details route or selected episode changes.
/// This is intentionally fire-and-forget: the core details projection must not
/// wait for third-party network responses.
pub fn on_details_synced(
    ui: &slint::Weak<MainWindow>,
    meta_item: &stremio_core::types::resource::MetaItem,
    details: &stremio_core::models::meta_details::MetaDetails,
) {
    let Some(manager) = manager() else {
        return;
    };
    let Some(selected) = details.selected.as_ref() else {
        return;
    };
    let Some(stream_path) = selected.stream_path.as_ref() else {
        return;
    };
    let media_type = match selected.meta_path.r#type.as_str() {
        "series" | "show" => "tv",
        _ => "movie",
    };
    let (season, episode) = if media_type == "tv" {
        meta_item
            .videos
            .iter()
            .find(|video| video.id == stream_path.id)
            .and_then(|video| video.series_info.as_ref())
            .map(|info| (Some(info.season), Some(info.episode)))
            .or_else(|| parse_episode_id(&stream_path.id))
            .unwrap_or((Some(1), Some(1)))
    } else {
        (None, None)
    };
    let year = meta_item
        .preview
        .released
        .map(|date| date.format("%Y").to_string())
        .or_else(|| {
            meta_item
                .preview
                .release_info
                .as_deref()
                .and_then(|value| value.get(..4))
                .map(str::to_owned)
        });
    let query = DetailsQuery {
        key: format!(
            "{}|{}|{}|{}|{}|{}|{}",
            selected.meta_path.id,
            stream_path.id,
            media_type,
            season.unwrap_or_default(),
            episode.unwrap_or_default(),
            manager.config().enabled,
            manager.inner.playback.source_generation()
        ),
        meta_id: selected.meta_path.id.clone(),
        video_id: stream_path.id.clone(),
        media_type: media_type.to_owned(),
        title: meta_item.preview.name.clone(),
        year,
        season,
        episode,
    };
    {
        let mut last = manager
            .inner
            .last_query
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.as_deref() == Some(query.key.as_str()) {
            return;
        }
        *last = Some(query.key.clone());
    }
    let generation = manager.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
    manager.inner.playback.clear_plugin_results();
    {
        let manager = manager.clone();
        let _ = slint::invoke_from_event_loop(move || manager.rebuild_streams());
    }
    if !manager.config().enabled {
        return;
    }
    let ui = ui.clone();
    let manager = manager.clone();
    tokio::spawn(async move {
        manager.run_details_query(query, generation, ui).await;
    });
}

fn parse_episode_id(video_id: &str) -> Option<(Option<u32>, Option<u32>)> {
    let mut parts = video_id.rsplit(':');
    let episode = parts.next()?.parse::<u32>().ok()?;
    let season = parts.next()?.parse::<u32>().ok()?;
    Some((Some(season), Some(episode)))
}

impl NuvioPlugins {
    fn rebuild_streams(&self) {
        let views = self
            .inner
            .runtime
            .model()
            .ok()
            .map(|model| {
                self.inner
                    .playback
                    .rebuild(&model.meta_details, &model.ctx.profile.addons)
            })
            .unwrap_or_default();
        let ui = self.inner.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                let weak = ui.as_weak();
                crate::event_loop::project_stream_selection_views(&ui, &weak, views);
            }
        });
    }

    fn apply_plugin_results(&self, generation: u64, streams: Vec<PluginStream>, video_id: String) {
        if self.inner.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let selections = streams
            .into_iter()
            .enumerate()
            .map(|(index, stream)| PluginSelection {
                id: format!("plugin:{}:{index}", stream.provider_key),
                provider_name: stream.provider_name,
                video_id: Some(video_id.clone()),
                title: stream.name,
                description: stream.description,
                url: stream.url,
                info_hash: stream.info_hash,
                file_idx: stream.file_idx,
                headers: stream.headers,
                subtitles: stream.subtitles,
                seeders: stream.seeders.unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        self.inner.playback.set_plugin_results(selections);
        let manager = self.clone();
        let _ = slint::invoke_from_event_loop(move || manager.rebuild_streams());
    }

    async fn script_for(&self, provider: &PluginProvider) -> Result<String, String> {
        let cached = self
            .inner
            .scripts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&provider.source_url)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(600))
            .map(|(_, script)| script.clone());
        if let Some(script) = cached {
            return Ok(script);
        }
        let script = fetch_script(&provider.source_url).await?;
        let mut cache = self
            .inner
            .scripts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(provider.source_url.clone(), (Instant::now(), script.clone()));
        Ok(script)
    }

    async fn resolve_tmdb_id(&self, query: &DetailsQuery) -> Result<String, String> {
        let raw = query.meta_id.trim();
        if let Some(id) = raw.strip_prefix("tmdb:")
            && id.chars().all(|character| character.is_ascii_digit())
            && !id.is_empty()
        {
            return Ok(id.to_owned());
        }
        if raw.chars().all(|character| character.is_ascii_digit()) && !raw.is_empty() {
            return Ok(raw.to_owned());
        }
        if let Some(cached) = self
            .inner
            .tmdb_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(raw)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(86_400))
            .map(|(_, id)| id.clone())
        {
            return Ok(cached);
        }

        // If the user configured the TMDB metadata integration, use it first:
        // it resolves IMDb ids without sending the title through search.
        if let Ok(profile_id) = crate::profiles::active_profile_id().await
            && let Ok(providers) = crate::integrations::enabled_metadata_providers(profile_id.as_str()).await
        {
            for provider in providers {
                if provider.kind() != media_integrations::ProviderKind::Tmdb {
                    continue;
                }
                if let Ok(meta) = provider.enrich(raw).await
                    && let Some(id) = meta.external_ids.get("tmdb")
                    && id.chars().all(|character| character.is_ascii_digit())
                {
                    self.cache_tmdb(raw, id);
                    return Ok(id.clone());
                }
            }
        }

        let gateway_type = if query.media_type == "tv" { "tv" } else { "movie" };
        let mut url = Url::parse("https://senzo-api.vercel.app/api/tmdb/search")
            .map_err(|_| "The metadata gateway URL is invalid.".to_owned())?;
        url.query_pairs_mut()
            .append_pair("query", query.title.trim())
            .append_pair("type", gateway_type);
        let text = fetch_text(url.as_str(), 2 * 1024 * 1024).await?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|_| "The metadata gateway returned invalid JSON.".to_owned())?;
        let results = value
            .get("results")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "The metadata gateway returned no results.".to_owned())?;
        let year = query.year.as_deref().and_then(|value| value.get(..4));
        let selected = results
            .iter()
            .find(|result| {
                let date = if gateway_type == "tv" {
                    result.get("first_air_date").and_then(serde_json::Value::as_str)
                } else {
                    result.get("release_date").and_then(serde_json::Value::as_str)
                };
                year.is_some_and(|year| date.is_some_and(|date| date.starts_with(year)))
            })
            .or_else(|| results.first())
            .ok_or_else(|| "No TMDB match was found for this title.".to_owned())?;
        let id = selected
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .map(|id| id.to_string())
            .ok_or_else(|| "The metadata gateway result had no TMDB id.".to_owned())?;
        self.cache_tmdb(raw, &id);
        Ok(id)
    }

    fn cache_tmdb(&self, raw: &str, id: &str) {
        let mut cache = self
            .inner
            .tmdb_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(raw.to_owned(), (Instant::now(), id.to_owned()));
    }

    async fn run_details_query(
        &self,
        query: DetailsQuery,
        generation: u64,
        _ui: slint::Weak<MainWindow>,
    ) {
        let providers = self.enabled_providers(&query.media_type);
        if providers.is_empty() {
            self.set_message("No enabled plugin providers support this media type.", false);
            return;
        }
        self.set_message("Resolving metadata for plugin providers…", false);
        let tmdb_id = match self.resolve_tmdb_id(&query).await {
            Ok(id) => id,
            Err(error) => {
                for provider in &providers {
                    self.set_status(&provider.key, "No TMDB id", true);
                }
                // Name the failing stage: "plugin failed" was unhelpful when
                // the real cause was the shared metadata lookup.
                let message = if error.starts_with("No TMDB match") {
                    error
                } else {
                    format!("Metadata lookup failed: {error}")
                };
                self.set_message(&message, true);
                self.project();
                return;
            }
        };
        if self.inner.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let cache_key = format!(
            "{}|{}|{}|{}",
            tmdb_id,
            query.media_type,
            query.season.unwrap_or_default(),
            query.episode.unwrap_or_default()
        );
        let cached = self
            .inner
            .results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&cache_key)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(300))
            .map(|(_, streams)| streams.clone());
        if let Some(streams) = cached {
            let count = streams.len();
            self.apply_plugin_results(generation, streams, query.video_id.clone());
            self.set_message(&format!("Plugin search complete: {count} cached source(s)"), false);
            return;
        }

        // Every enabled provider runs at once. Eight at a time left later
        // providers waiting behind slow scripts; sixteen keeps a cold load
        // close to the slowest provider's own response time.
        let semaphore = Arc::new(Semaphore::new(16));
        let mut tasks = FuturesUnordered::new();
        // The semaphore bounds concurrent script fetches and QuickJS workers.
        for provider in providers {
            let manager = self.clone();
            let query = query.clone();
            let tmdb_id = tmdb_id.clone();
            let semaphore = semaphore.clone();
            tasks.push(async move {
                let _permit = semaphore.acquire_owned().await.ok()?;
                manager.set_status(&provider.key, "Searching…", false);
                manager.project();
                let script = match manager.script_for(&provider).await {
                    Ok(script) => script,
                    Err(error) => {
                        manager.set_status(&provider.key, &error, true);
                        manager.project();
                        return None;
                    }
                };
                let settings = if serde_json::from_str::<serde_json::Value>(&provider.settings)
                    .is_ok_and(|value| value.is_object())
                {
                    provider.settings.clone()
                } else {
                    "{}".to_owned()
                };
                let key = provider.key.clone();
                let name = provider.name.clone();
                let request = ProviderQuery {
                    tmdb_id,
                    media_type: query.media_type.clone(),
                    season: query.season,
                    episode: query.episode,
                };
                let outcome = tokio::task::spawn_blocking(move || {
                    plugin_runtime::execute(&key, &name, &script, &settings, &request)
                })
                .await
                .ok()?;
                Some((provider, outcome))
            });
        }

        let mut streams = Vec::new();
        let mut seen = HashSet::new();
        let mut errors = 0usize;
        let mut completed = 0usize;
        while let Some(result) = tasks.next().await {
            let Some((provider, outcome)) = result else {
                errors += 1;
                continue;
            };
            completed += 1;
            if let Some(error) = outcome.error {
                errors += 1;
                self.set_status(&provider.key, truncate_message(&error, 90), true);
            } else {
                let candidates = outcome
                    .streams
                    .into_iter()
                    .filter(PluginStream::is_playable)
                    .collect::<Vec<_>>();
                // Show the provider's answer immediately. The old probe pass
                // opened an HTTP request per direct URL (8s timeout, eight at a
                // time) and held the whole list back, while the phone client
                // renders every source as soon as the script returns.
                let mut count = 0usize;
                for stream in candidates {
                    let dedupe = if stream.is_torrent() {
                        stream.info_hash.clone().unwrap_or_default()
                    } else {
                        stream
                            .url
                            .clone()
                            .unwrap_or_else(|| format!("{}:{}", stream.provider_key, stream.name))
                    };
                    if !seen.insert(dedupe) {
                        continue;
                    }
                    count += 1;
                    streams.push(stream);
                    if streams.len() >= 160 {
                        break;
                    }
                }
                self.set_status(
                    &provider.key,
                    if count == 0 {
                        "No sources".to_owned()
                    } else {
                        format!("{count} source(s)")
                    },
                    false,
                );
            }
            self.project();
            if streams.len() >= 160 {
                break;
            }
        }

        if self.inner.generation.load(Ordering::Acquire) != generation {
            return;
        }
        {
            let mut cache = self
                .inner
                .results
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if cache.len() >= 64 {
                cache.clear();
            }
            cache.insert(cache_key, (Instant::now(), streams.clone()));
        }
        self.apply_plugin_results(generation, streams.clone(), query.video_id.clone());
        let message = if streams.is_empty() && errors > 0 {
            format!(
                "No plugin sources. {errors} provider(s) failed; check this device's connection, then retry."
            )
        } else if errors > 0 {
            format!(
                "Plugin search complete: {} source(s) from {completed} provider(s); {errors} failed",
                streams.len()
            )
        } else {
            format!(
                "Plugin search complete: {} source(s) from {completed} provider(s)",
                streams.len()
            )
        };
        self.set_message(&message, streams.is_empty() && errors > 0);
        self.project();
    }
}

fn truncate_message(value: &str, limit: usize) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if output.len() + character.len_utf8() > limit {
            break;
        }
        output.push(character);
    }
    output
}


pub fn setup(
    ui: &MainWindow,
    config: &PluginsConfig,
    runtime: Arc<stremio_core::runtime::Runtime<core_env::DesktopEnv, AppModel>>,
    playback: Arc<PlaybackSelections>,
) {
    let manager = NuvioPlugins {
        inner: Arc::new(Inner {
            ui: ui.as_weak(),
            config: RwLock::new(config.clone()),
            status: RwLock::new(HashMap::new()),
            busy: RwLock::new(false),
            provider_filter: RwLock::new("All".to_owned()),
            provider_query: RwLock::new(String::new()),
            repository_filter: RwLock::new(String::new()),
            runtime,
            playback,
            generation: AtomicU64::new(0),
            last_query: Mutex::new(None),
            scripts: Mutex::new(HashMap::new()),
            tmdb_ids: Mutex::new(HashMap::new()),
            results: Mutex::new(HashMap::new()),
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
    ui.on_plugin_provider_filter_changed({
        let manager = manager.clone();
        move |filter| manager.set_provider_filter(filter.to_string())
    });
    ui.on_plugin_provider_query_changed({
        let manager = manager.clone();
        move |query| manager.set_provider_query(query.to_string())
    });
    ui.on_plugin_repository_filter_changed({
        let manager = manager.clone();
        move |filter| {
            let filter = filter.to_string();
            manager.set_repository_filter(if filter == ALL_REPOSITORIES {
                String::new()
            } else {
                filter
            });
        }
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
    ui.on_plugin_repository_enabled({
        let manager = manager.clone();
        move |url, enabled| manager.set_repository_enabled(url.as_str(), enabled)
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
    ui.on_plugin_provider_settings_saved({
        let manager = manager.clone();
        move |key, settings| manager.update_provider_settings(key.as_str(), settings.as_str())
    });
    ui.on_plugin_refresh_all({
        let manager = manager.clone();
        move || {
            let manager = manager.clone();
            let repositories = manager.config().repositories;
            tokio::spawn(async move {
                // Refreshing repositories one by one turned "Refresh all" into
                // a queue of network round-trips. Providers are independent,
                // so refresh a few repositories at a time instead.
                stream::iter(repositories)
                    .for_each_concurrent(REFRESH_CONCURRENCY, |repository| {
                        let manager = manager.clone();
                        async move {
                            manager
                                .refresh_repository(&repository.manifest_url)
                                .await;
                        }
                    })
                    .await;
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

/// One pooled HTTP client for every plugin manifest, script and gateway call.
///
/// Building a client per request (the previous behaviour) paid for a fresh DNS
/// lookup, TCP handshake and TLS session every time, so a slow or flaky link
/// turned into failures a kept-alive connection would have absorbed.
static PLUGIN_HTTP: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
const FETCH_ATTEMPTS: u32 = 3;
/// Plugin lists of one repository are fetched this many at a time.
const LOAD_CONCURRENCY: usize = 6;
/// Repositories refreshed together by "Refresh all".
const REFRESH_CONCURRENCY: usize = 4;

fn plugin_http_client() -> Result<&'static reqwest::Client, String> {
    PLUGIN_HTTP
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 5 || !is_public_url(attempt.url()) {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                }))
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .pool_idle_timeout(Duration::from_secs(90))
                .tcp_keepalive(Duration::from_secs(30))
                .user_agent("SENZO-PluginManager/1.0 (+https://github.com/Sensei002/senzo)")
                .build()
                .map_err(|error| format!("Could not create plugin HTTP client: {error}"))
        })
        .as_ref()
        .map_err(|error| error.clone())
}

struct FetchFailure {
    message: String,
    retryable: bool,
}

impl FetchFailure {
    fn permanent(message: impl Into<String>) -> Self {
        Self { message: message.into(), retryable: false }
    }

    fn transient(message: impl Into<String>) -> Self {
        Self { message: message.into(), retryable: true }
    }
}

async fn fetch_text(url: &str, limit: usize) -> Result<String, String> {
    let parsed = Url::parse(url).map_err(|_| "Invalid plugin URL.".to_owned())?;
    if parsed.scheme() != "https" {
        return Err("Plugin repositories must use HTTPS.".to_owned());
    }
    match fetch_with_retries(parsed.clone(), limit).await {
        Ok(text) => Ok(text),
        // raw.githubusercontent.com is unreachable on some networks (it is
        // blocked in several countries). jsDelivr mirrors GitHub content, so
        // one fallback attempt keeps add-ons installable there.
        Err(error) => match github_mirror(&parsed) {
            Some(mirror) => fetch_with_retries(mirror.clone(), limit)
                .await
                .map_err(|mirror_error| format!("{error} Mirror {mirror}: {mirror_error}")),
            None => Err(error),
        },
    }
}

fn github_mirror(url: &Url) -> Option<Url> {
    if url.host_str()? != "raw.githubusercontent.com" {
        return None;
    }
    let segments = url
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if segments.len() < 4 {
        return None;
    }
    let rest = segments[3..].join("/");
    let path = format!(
        "gh/{}/{}@{}/{rest}",
        segments[0], segments[1], segments[2]
    );
    let mut mirror = Url::parse(&format!("https://cdn.jsdelivr.net/{path}")).ok()?;
    mirror.set_query(url.query());
    Some(mirror)
}

async fn fetch_with_retries(parsed: Url, limit: usize) -> Result<String, String> {
    let mut last_error = "Plugin request failed.".to_owned();
    for attempt in 0..FETCH_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400 * u64::from(attempt))).await;
        }
        match fetch_text_once(parsed.clone(), limit).await {
            Ok(text) => return Ok(text),
            Err(failure) if failure.retryable => last_error = failure.message,
            Err(failure) => return Err(failure.message),
        }
    }
    Err(format!(
        "{last_error} (retried {FETCH_ATTEMPTS} times; check this device's connection, DNS, or VPN)"
    ))
}

async fn fetch_text_once(parsed: Url, limit: usize) -> Result<String, FetchFailure> {
    // Resolution is blocking; keep it off the async workers so one slow DNS
    // server cannot stall every provider request in the batch.
    let lookup = parsed.clone();
    let host = tokio::task::spawn_blocking(move || classify_public_host(&lookup))
        .await
        .unwrap_or(PublicHost::Unresolved);
    match host {
        PublicHost::Blocked => {
            return Err(FetchFailure::permanent(
                "Plugin hosts must resolve to a public address.",
            ));
        }
        PublicHost::Unresolved => {
            return Err(FetchFailure::transient(format!(
                "Could not resolve {}.",
                parsed.host_str().unwrap_or("the plugin host")
            )));
        }
        PublicHost::Public => {}
    }
    let client = match plugin_http_client() {
        Ok(client) => client,
        Err(message) => return Err(FetchFailure::permanent(message)),
    };
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| {
            let message = if error.is_timeout() {
                "Plugin request timed out.".to_owned()
            } else if error.is_connect() {
                format!("Could not reach the plugin server: {error}")
            } else {
                format!("Plugin request failed: {error}")
            };
            if error.is_timeout() || error.is_connect() || error.is_request() || error.is_body() {
                FetchFailure::transient(message)
            } else {
                FetchFailure::permanent(message)
            }
        })?;
    let status = response.status();
    if !status.is_success() {
        let message = format!("Plugin server returned HTTP {status}.");
        return Err(if status.is_server_error()
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            FetchFailure::transient(message)
        } else {
            FetchFailure::permanent(message)
        });
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| FetchFailure::transient(format!("Could not read plugin response: {error}")))?;
    if bytes.len() > limit {
        return Err(FetchFailure::permanent(
            "Plugin response is larger than the allowed limit.",
        ));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| FetchFailure::permanent("Plugin response is not valid UTF-8."))
}

async fn fetch_json(url: &str) -> Result<serde_json::Value, String> {
    let text = fetch_text(url, MAX_MANIFEST_BYTES).await?;
    serde_json::from_str(&text).map_err(|error| format!("Invalid plugin manifest: {error}"))
}

/// Load a repository in any of the shapes community projects publish.
///
/// A manifest with `pluginLists` is an index: each listed document is fetched
/// (one level of indirection is enough in practice, two are allowed) and its
/// providers are resolved against that document's own URL, so relative script
/// names keep working no matter where the list is hosted.
/// Plugin lists of one repository document, or no lists for a leaf document.
fn merge_repository_document(
    content: &mut RepositoryContent,
    depth: usize,
    base_url: &str,
    value: serde_json::Value,
) -> Vec<String> {
    match value {
        serde_json::Value::Array(entries) => {
            content.scrapers.extend(
                entries
                    .into_iter()
                    .filter_map(parse_scraper)
                    .map(|scraper| (base_url.to_owned(), scraper)),
            );
            Vec::new()
        }
        serde_json::Value::Object(_) => {
            let manifest: RawManifest = serde_json::from_value(value).unwrap_or_default();
            if content.name.trim().is_empty() {
                content.name = manifest.name.unwrap_or_default();
            }
            if content.version.is_empty() {
                content.version = manifest.version.as_ref().map(json_text).unwrap_or_default();
            }
            if content.description.is_empty() {
                content.description = manifest.description.unwrap_or_default();
            }
            content.scrapers.extend(
                manifest
                    .scrapers
                    .into_iter()
                    .filter_map(RawScraper::normalize)
                    .map(|scraper| (base_url.to_owned(), scraper)),
            );
            if depth >= 2 {
                Vec::new()
            } else {
                manifest.plugin_lists
            }
        }
        _ => Vec::new(),
    }
}

async fn load_repository(manifest_url: &str) -> Result<RepositoryContent, String> {
    let root = fetch_json(manifest_url).await?;
    let mut content = RepositoryContent::default();
    let mut errors = Vec::new();
    let mut level = vec![(0usize, manifest_url.to_owned(), root)];
    while !level.is_empty() {
        let mut list_requests = Vec::new();
        for (depth, base_url, value) in std::mem::take(&mut level) {
            for list in merge_repository_document(&mut content, depth, &base_url, value) {
                let list_url = Url::parse(&base_url)
                    .and_then(|base| base.join(&list))
                    .ok()
                    .filter(|url| url.scheme() == "https" && !is_blocked_host(url));
                match list_url {
                    Some(list_url) => list_requests.push((depth + 1, list_url.to_string())),
                    None => errors.push(format!(
                        "{list}: plugin lists must use HTTPS on a public host"
                    )),
                }
            }
        }
        if list_requests.is_empty() {
            continue;
        }
        // Sibling plugin lists are independent documents. Fetching them one
        // after another made a repository that publishes a dozen lists wait
        // for a dozen round-trips (with retries) before one provider appeared.
        let mut fetched = stream::iter(list_requests.into_iter().map(|(depth, url)| async move {
            let value = fetch_json(url.as_str()).await;
            (depth, url, value)
        }))
        .buffer_unordered(LOAD_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        fetched.sort_by(|left, right| left.1.cmp(&right.1));
        for (depth, url, value) in fetched {
            match value {
                Ok(value) => level.push((depth, url, value)),
                Err(error) => errors.push(format!("{url}: {error}")),
            }
        }
    }
    if content.scrapers.is_empty() {
        return Err(if errors.is_empty() {
            "This repository declares no plugin providers.".to_owned()
        } else {
            format!("Could not load plugin providers: {}", errors.join("; "))
        });
    }
    if content.name.trim().is_empty() {
        content.name = manifest_fallback_name(manifest_url);
    }
    Ok(content)
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

    #[test]
    fn cloudstream_build_entries_are_normalized() {
        let entry = serde_json::json!({
            "url": "https://example.com/builds/Movie.cs3",
            "status": 1,
            "version": 11,
            "name": "Movie",
            "internalName": "MovieProvider",
            "description": "Bangla & Hindi movies",
            "language": "hi",
            "tvTypes": ["Movie", "TvSeries"],
            "iconUrl": "https://example.com/icon.png"
        });
        let scraper = parse_scraper(entry).expect("entry should normalize");
        assert_eq!(scraper.id, "MovieProvider");
        assert_eq!(scraper.name, "Movie");
        assert_eq!(scraper.version, "11");
        assert_eq!(scraper.filename, "https://example.com/builds/Movie.cs3");
        assert_eq!(scraper.supported_types, vec!["movie", "tv"]);
        assert_eq!(scraper.logo.as_deref(), Some("https://example.com/icon.png"));
        assert_eq!(scraper.content_language, Some(vec!["hi".to_owned()]));
        assert!(scraper.enabled);
    }

    #[test]
    fn repository_index_lists_plugin_lists() {
        let index: RawManifest = serde_json::from_str(
            r#"{
                "name": "Redowan's BDIX repository",
                "description": "BDIX builds",
                "manifestVersion": 1,
                "pluginLists": [
                    "https://raw.githubusercontent.com/redowan99/Redowan-CloudStream/builds/plugins.json"
                ]
            }"#,
        )
        .expect("index should parse");
        assert_eq!(index.name.as_deref(), Some("Redowan's BDIX repository"));
        assert_eq!(index.plugin_lists.len(), 1);
        assert!(index.scrapers.is_empty());
    }

    #[test]
    fn provider_filters_match_content_types() {
        let provider = PluginProvider {
            supported_types: vec!["movie".to_owned(), "tv".to_owned()],
            name: "Movie Hub".to_owned(),
            languages: vec!["bn".to_owned()],
            ..PluginProvider::default()
        };
        assert!(provider_matches_filter(&provider, "All"));
        assert!(provider_matches_filter(&provider, "Movies"));
        assert!(provider_matches_filter(&provider, "Series"));
        assert!(!provider_matches_filter(&provider, "Live TV"));
        assert!(provider_matches_query(&provider, "movie bn"));
        assert!(!provider_matches_query(&provider, "anime"));
    }

    #[test]
    fn repository_filter_labels_stay_unique() {
        let repositories = vec![
            PluginRepository {
                manifest_url: "https://a.example.com/manifest.json".to_owned(),
                name: "Community".to_owned(),
                ..PluginRepository::default()
            },
            PluginRepository {
                manifest_url: "https://b.example.com/manifest.json".to_owned(),
                name: "Community".to_owned(),
                ..PluginRepository::default()
            },
            PluginRepository {
                manifest_url: "https://c.example.com/repo.json".to_owned(),
                name: "   ".to_owned(),
                ..PluginRepository::default()
            },
        ];
        let labels = repository_filter_labels(&repositories);
        assert_eq!(labels[0], ("Community".to_owned(), repositories[0].manifest_url.clone()));
        assert_eq!(
            labels[1],
            ("Community (2)".to_owned(), repositories[1].manifest_url.clone())
        );
        assert_eq!(labels[2].0, "repo.json");
    }
}