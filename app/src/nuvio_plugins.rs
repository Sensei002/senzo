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

use futures::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use tokio::sync::Semaphore;
use url::Url;

use crate::{
    MainWindow,
    app_model::AppModel,
    direct_sources::net::is_blocked_host,
    playback::{PlaybackSelections, PluginSelection},
    plugin_runtime::{self, PluginStream, ProviderQuery, net::is_public_url},
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
                    settings: provider.settings.clone().into(),
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
                        self.set_status(key, &truncate_message(&error, 90), true);
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
            .map(|info| (Some(u32::from(info.season)), Some(u32::from(info.episode))))
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

    fn apply_plugin_results(&self, generation: u64, streams: Vec<PluginStream>) {
        if self.inner.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let selections = streams
            .into_iter()
            .enumerate()
            .map(|(index, stream)| PluginSelection {
                id: format!("plugin:{}:{index}", stream.provider_key),
                provider_name: stream.provider_name,
                title: stream.name,
                description: stream.description,
                url: stream.url,
                info_hash: stream.info_hash,
                file_idx: stream.file_idx,
                headers: stream.headers,
                subtitles: stream.subtitles,
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
        if let Some(id) = raw.strip_prefix("tmdb:") {
            if id.chars().all(|character| character.is_ascii_digit()) && !id.is_empty() {
                return Ok(id.to_owned());
            }
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
                self.set_message(&error, true);
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
            self.apply_plugin_results(generation, streams);
            self.set_message(&format!("Plugin search complete: {count} cached source(s)"), false);
            return;
        }

        let semaphore = Arc::new(Semaphore::new(8));
        let mut tasks = FuturesUnordered::new();
        for provider in providers.into_iter().take(12) {
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
                self.set_status(&provider.key, &truncate_message(&error, 90), true);
            } else {
                let count = outcome.streams.len();
                for stream in outcome.streams {
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
        self.apply_plugin_results(generation, streams.clone());
        self.set_message(
            &format!(
                "Plugin search complete: {} source(s) from {completed} provider(s)",
                streams.len()
            ),
            errors > 0 && streams.is_empty(),
        );
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
    if parsed.scheme() != "https" || !is_public_url(&parsed) {
        return Err("Plugin repositories must use HTTPS on a public host.".to_owned());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !is_public_url(attempt.url()) {
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
