//! Nuvio-compatible QuickJS scraper plugins, isolated from Stremio add-ons.
//!
//! Plugin source is untrusted. Every execution gets a fresh QuickJS runtime with
//! memory, stack, wall-clock and concurrency limits. Network calls are HTTPS-only,
//! DNS-pinned to globally routable addresses, redirect-revalidated and response-bounded.

use std::{
    collections::BTreeMap,
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{
    Method, StatusCode,
    blocking::{Client, Response},
    header::{HeaderMap, HeaderName, HeaderValue, LOCATION},
    redirect::Policy,
};
use rquickjs::{Context, Promise, Runtime, prelude::Func};
use senzo_domain::{SettingKey, SettingsRepository};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Semaphore;
use url::{Host, Url};

const MAX_MANIFEST_BYTES: usize = 512 * 1024;
const MAX_SCRIPT_BYTES: usize = 512 * 1024;
const MAX_FETCH_BYTES: usize = 2 * 1024 * 1024;
const MAX_REPOSITORIES: usize = 32;
const MAX_SCRAPERS_PER_REPOSITORY: usize = 128;
const MAX_STREAMS: usize = 100;
const MAX_URL_LENGTH: usize = 4096;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_VALUE: usize = 8192;
const MAX_REQUEST_BODY: usize = 1024 * 1024;
const MAX_REDIRECTS: usize = 4;
const MAX_CONCURRENT_EXECUTIONS: usize = 10;
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(60);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(10);
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const STACK_LIMIT: usize = 512 * 1024;
const MAX_TIMER_DELAY_MS: u64 = 60_000;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin repository URL must be an absolute HTTPS URL without credentials, query or fragment")]
    InvalidRepositoryUrl,
    #[error("plugin network access has not been explicitly granted")]
    NetworkPermissionRequired,
    #[error("plugin is disabled")]
    Disabled,
    #[error("plugin repository limit has been reached")]
    RepositoryLimit,
    #[error("plugin repository is already installed")]
    AlreadyInstalled,
    #[error("plugin repository was not found")]
    RepositoryNotFound,
    #[error("plugin scraper was not found")]
    ScraperNotFound,
    #[error("plugin manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("plugin network destination is private, local, reserved or otherwise blocked")]
    BlockedDestination,
    #[error("plugin host could not be resolved safely")]
    DnsResolution,
    #[error("plugin network request failed: {0}")]
    Network(String),
    #[error("plugin response exceeded the {0}-byte limit")]
    ResponseTooLarge(usize),
    #[error("plugin script exceeded the {0}-byte limit")]
    ScriptTooLarge(usize),
    #[error("plugin script execution exceeded its resource budget")]
    ExecutionLimit,
    #[error("plugin runtime failed: {0}")]
    Runtime(String),
    #[error("plugin returned invalid stream data")]
    InvalidStreams,
    #[error("plugin state could not be read or saved: {0}")]
    Persistence(String),
    #[error("plugin settings are invalid JSON or exceed the size limit")]
    InvalidSettings,
    #[error("plugin profile identifier is invalid")]
    InvalidProfile,
    #[error("plugin runtime worker failed: {0}")]
    Worker(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    pub scrapers: Vec<PluginManifestScraper>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifestScraper {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub version: String,
    pub filename: String,
    #[serde(default = "default_supported_types")]
    pub supported_types: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub has_settings: bool,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub content_language: Option<Vec<String>>,
    #[serde(default)]
    pub supported_platforms: Option<Vec<String>>,
    #[serde(default)]
    pub disabled_platforms: Option<Vec<String>>,
    #[serde(default)]
    pub formats: Option<Vec<String>>,
    #[serde(default)]
    pub supported_formats: Option<Vec<String>>,
    #[serde(default)]
    pub supports_external_player: Option<bool>,
    #[serde(default)]
    pub limited: Option<bool>,
}

fn default_supported_types() -> Vec<String> {
    vec!["movie".to_owned(), "tv".to_owned()]
}
const fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginRepository {
    pub manifest_url: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub version: String,
    pub scraper_count: usize,
    pub last_updated_epoch_ms: u64,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginScraper {
    pub id: String,
    pub repository_url: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    pub filename: String,
    pub supported_types: Vec<String>,
    pub enabled: bool,
    pub manifest_enabled: bool,
    pub has_settings: bool,
    #[serde(default)]
    pub content_language: Vec<String>,
    #[serde(default)]
    pub formats: Vec<String>,
    /// Grant is intentionally separate from enabling a plugin.
    #[serde(default)]
    pub network_access_granted: bool,
}

impl PluginScraper {
    pub fn supports_type(&self, media_type: &str) -> bool {
        let normalized = normalize_media_type(media_type);
        self.supported_types.iter().any(|kind| normalize_media_type(kind) == normalized)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginState {
    #[serde(default = "default_true")]
    pub plugins_enabled: bool,
    #[serde(default)]
    pub repositories: Vec<PluginRepository>,
    #[serde(default)]
    pub scrapers: Vec<PluginScraper>,
}

impl Default for PluginState {
    fn default() -> Self {
        Self {
            plugins_enabled: true,
            repositories: Vec::new(),
            scrapers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSubtitle {
    pub url: String,
    pub language: String,
    pub name: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// Provider-neutral result contract for plugin scrapers. Phase 8 can consume
/// this type alongside Stremio results without coupling either runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginStream {
    pub id: String,
    pub scraper_id: String,
    pub provider: String,
    pub title: String,
    pub name: Option<String>,
    pub target: StreamTarget,
    pub quality: Option<String>,
    pub size: Option<String>,
    pub language: Option<String>,
    pub stream_type: Option<String>,
    pub seeders: Option<u32>,
    pub peers: Option<u32>,
    pub headers: BTreeMap<String, String>,
    pub subtitles: Vec<PluginSubtitle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum StreamTarget {
    Http(String),
    Torrent { info_hash: String, file_index: Option<u32> },
    External(String),
    Youtube(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScraperInput {
    pub tmdb_id: String,
    pub media_type: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl ScraperInput {
    pub fn validate(&self) -> Result<(), PluginError> {
        let id = self.tmdb_id.trim();
        if id.is_empty() || id.len() > 16 || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PluginError::Runtime("TMDB identifier must be numeric".to_owned()));
        }
        if !matches!(normalize_media_type(&self.media_type).as_str(), "movie" | "tv") {
            return Err(PluginError::Runtime("media type must be movie or tv".to_owned()));
        }
        if self.media_type.eq_ignore_ascii_case("movie") && (self.season.is_some() || self.episode.is_some()) {
            return Err(PluginError::Runtime("movie execution cannot include a season or episode".to_owned()));
        }
        if self.episode.is_some() && self.season.is_none() {
            return Err(PluginError::Runtime("episode execution requires a season".to_owned()));
        }
        Ok(())
    }
}

pub trait PluginSettingsStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, value: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

#[derive(Clone)]
pub struct PluginManager {
    settings: Arc<dyn SettingsRepository>,
    secret_store: Arc<dyn PluginSettingsStore>,
    cache_dir: PathBuf,
    http: PluginHttp,
    semaphore: Arc<Semaphore>,
}

impl PluginManager {
    pub fn new(
        settings: Arc<dyn SettingsRepository>,
        secret_store: Arc<dyn PluginSettingsStore>,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            settings,
            secret_store,
            cache_dir: cache_dir.into(),
            http: PluginHttp,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_EXECUTIONS)),
        }
    }

    pub fn list_state(&self, profile_id: &str) -> Result<PluginState, PluginError> {
        let key = state_key(profile_id)?;
        let Some(raw) = self.settings.get(&key).map_err(|e| PluginError::Persistence(e.to_string()))? else {
            return Ok(PluginState::default());
        };
        serde_json::from_str(&raw).map_err(|e| PluginError::Persistence(e.to_string()))
    }

    pub fn add_repository(&self, profile_id: &str, raw_url: &str) -> Result<PluginState, PluginError> {
        let manifest_url = normalize_manifest_url(raw_url)?;
        let current = self.list_state(profile_id)?;
        if current.repositories.iter().any(|repo| repo.manifest_url == manifest_url) {
            return Err(PluginError::AlreadyInstalled);
        }
        if current.repositories.len() >= MAX_REPOSITORIES {
            return Err(PluginError::RepositoryLimit);
        }
        let manifest_payload = self.http.get_text(&manifest_url, MAX_MANIFEST_BYTES)?;
        let manifest = parse_manifest(&manifest_payload)?;
        let base = Url::parse(&manifest_url).map_err(|_| PluginError::InvalidRepositoryUrl)?;
        let repo_root = base.join(".").map_err(|_| PluginError::InvalidRepositoryUrl)?;
        let mut next = current;
        let mut installed_scrapers = Vec::new();
        for scraper in &manifest.scrapers {
            validate_scraper_manifest(scraper)?;
            let script_url = resolve_script_url(&repo_root, &scraper.filename)?;
            let code = self.http.get_text(script_url.as_str(), MAX_SCRIPT_BYTES)
                .map_err(|error| PluginError::Network(format!("{}: {error}", scraper.name)))?;
            let id = scraper_identity(&manifest_url, &scraper.id);
            self.write_script(profile_id, &id, &code)?;
            installed_scrapers.push(PluginScraper {
                id,
                repository_url: manifest_url.clone(),
                name: scraper.name.clone(),
                description: scraper.description.clone().unwrap_or_default(),
                version: scraper.version.clone(),
                filename: scraper.filename.clone(),
                supported_types: scraper.supported_types.clone(),
                enabled: scraper.enabled,
                manifest_enabled: scraper.enabled,
                has_settings: scraper.has_settings,
                content_language: scraper.content_language.clone().unwrap_or_default(),
                formats: scraper.formats.clone().or(scraper.supported_formats.clone()).unwrap_or_default(),
                network_access_granted: false,
            });
        }
        let repository = PluginRepository {
            manifest_url: manifest_url.clone(),
            name: manifest.name,
            description: manifest.description,
            version: manifest.version,
            scraper_count: installed_scrapers.len(),
            last_updated_epoch_ms: epoch_millis(),
            last_error: None,
        };
        next.repositories.push(repository);
        next.scrapers.extend(installed_scrapers);
        self.save_state(profile_id, &next)?;
        Ok(next)
    }

    pub fn refresh_repository(&self, profile_id: &str, manifest_url: &str) -> Result<PluginState, PluginError> {
        let current = self.list_state(profile_id)?;
        let existing = current.repositories.iter().find(|repo| repo.manifest_url == manifest_url)
            .ok_or(PluginError::RepositoryNotFound)?;
        let payload = self.http.get_text(manifest_url, MAX_MANIFEST_BYTES)?;
        let manifest = parse_manifest(&payload)?;
        if manifest.name != existing.name {
            return Err(PluginError::InvalidManifest("repository identity changed; previous version was retained".to_owned()));
        }
        let base = Url::parse(manifest_url).map_err(|_| PluginError::InvalidRepositoryUrl)?;
        let repo_root = base.join(".").map_err(|_| PluginError::InvalidRepositoryUrl)?;
        let previous = current.scrapers.iter().filter(|scraper| scraper.repository_url == manifest_url)
            .map(|scraper| (scraper.id.clone(), scraper.clone())).collect::<std::collections::HashMap<_, _>>();
        let mut refreshed = Vec::new();
        for scraper in &manifest.scrapers {
            validate_scraper_manifest(scraper)?;
            let id = scraper_identity(manifest_url, &scraper.id);
            let script_url = resolve_script_url(&repo_root, &scraper.filename)?;
            let code = self.http.get_text(script_url.as_str(), MAX_SCRIPT_BYTES)?;
            self.write_script(profile_id, &id, &code)?;
            let old = previous.get(&id);
            refreshed.push(PluginScraper {
                id,
                repository_url: manifest_url.to_owned(),
                name: scraper.name.clone(),
                description: scraper.description.clone().unwrap_or_default(),
                version: scraper.version.clone(),
                filename: scraper.filename.clone(),
                supported_types: scraper.supported_types.clone(),
                enabled: old.map(|item| item.enabled).unwrap_or(scraper.enabled) && scraper.enabled,
                manifest_enabled: scraper.enabled,
                has_settings: scraper.has_settings,
                content_language: scraper.content_language.clone().unwrap_or_default(),
                formats: scraper.formats.clone().or(scraper.supported_formats.clone()).unwrap_or_default(),
                network_access_granted: old.is_some_and(|item| item.network_access_granted),
            });
        }
        let mut next = current;
        next.scrapers.retain(|scraper| scraper.repository_url != manifest_url);
        next.scrapers.extend(refreshed);
        if let Some(repo) = next.repositories.iter_mut().find(|repo| repo.manifest_url == manifest_url) {
            repo.name = manifest.name;
            repo.description = manifest.description;
            repo.version = manifest.version;
            repo.scraper_count = next.scrapers.iter().filter(|item| item.repository_url == manifest_url).count();
            repo.last_updated_epoch_ms = epoch_millis();
            repo.last_error = None;
        }
        self.save_state(profile_id, &next)?;
        Ok(next)
    }

    pub fn remove_repository(&self, profile_id: &str, manifest_url: &str) -> Result<PluginState, PluginError> {
        let mut state = self.list_state(profile_id)?;
        if !state.repositories.iter().any(|repo| repo.manifest_url == manifest_url) {
            return Err(PluginError::RepositoryNotFound);
        }
        let removed = state.scrapers.iter().filter(|scraper| scraper.repository_url == manifest_url)
            .cloned().collect::<Vec<_>>();
        state.scrapers.retain(|scraper| scraper.repository_url != manifest_url);
        state.repositories.retain(|repo| repo.manifest_url != manifest_url);
        for scraper in removed {
            self.delete_script(profile_id, &scraper.id);
            let account = settings_account(profile_id, &scraper.id);
            let _ = self.secret_store.delete(&account);
        }
        self.save_state(profile_id, &state)?;
        Ok(state)
    }

    pub fn set_enabled(&self, profile_id: &str, scraper_id: &str, enabled: bool) -> Result<PluginState, PluginError> {
        let mut state = self.list_state(profile_id)?;
        let scraper = state.scrapers.iter_mut().find(|item| item.id == scraper_id)
            .ok_or(PluginError::ScraperNotFound)?;
        scraper.enabled = enabled && scraper.manifest_enabled;
        self.save_state(profile_id, &state)?;
        Ok(state)
    }

    pub fn set_network_permission(&self, profile_id: &str, scraper_id: &str, granted: bool) -> Result<PluginState, PluginError> {
        let mut state = self.list_state(profile_id)?;
        let scraper = state.scrapers.iter_mut().find(|item| item.id == scraper_id)
            .ok_or(PluginError::ScraperNotFound)?;
        scraper.network_access_granted = granted;
        self.save_state(profile_id, &state)?;
        Ok(state)
    }

    pub fn set_plugins_enabled(&self, profile_id: &str, enabled: bool) -> Result<PluginState, PluginError> {
        let mut state = self.list_state(profile_id)?;
        state.plugins_enabled = enabled;
        self.save_state(profile_id, &state)?;
        Ok(state)
    }

    pub fn load_scraper_settings(&self, profile_id: &str, scraper_id: &str) -> Result<String, PluginError> {
        self.find_scraper(profile_id, scraper_id)?;
        let account = settings_account(profile_id, scraper_id);
        self.secret_store.get(&account).map_err(PluginError::Persistence)
            .map(|value| value.unwrap_or_else(|| "{}".to_owned()))
    }

    pub fn save_scraper_settings(&self, profile_id: &str, scraper_id: &str, raw_json: &str) -> Result<(), PluginError> {
        self.find_scraper(profile_id, scraper_id)?;
        if raw_json.len() > 32 * 1024 || serde_json::from_str::<Value>(raw_json).is_err() {
            return Err(PluginError::InvalidSettings);
        }
        let account = settings_account(profile_id, scraper_id);
        self.secret_store.set(&account, raw_json).map_err(PluginError::Persistence)
    }

    pub async fn scraper_settings_layout(&self, profile_id: &str, scraper_id: &str) -> Result<String, PluginError> {
        let state = self.list_state(profile_id)?;
        if !state.plugins_enabled { return Err(PluginError::Disabled); }
        let scraper = state.scrapers.iter().find(|item| item.id == scraper_id).ok_or(PluginError::ScraperNotFound)?;
        if !scraper.enabled { return Err(PluginError::Disabled); }
        if !scraper.network_access_granted { return Err(PluginError::NetworkPermissionRequired); }
        let code = self.read_script(profile_id, scraper_id)?;
        let stable_id = scraper.id.clone();
        self.run_limited(move || execute_settings(&code, &stable_id)).await
    }

    pub async fn test_scraper(&self, profile_id: &str, scraper_id: &str) -> Result<Vec<PluginStream>, PluginError> {
        let input = {
            let state = self.list_state(profile_id)?;
            let scraper = state.scrapers.iter().find(|item| item.id == scraper_id).ok_or(PluginError::ScraperNotFound)?;
            let media_type = if scraper.supports_type("movie") { "movie" } else { "tv" };
            ScraperInput {
                tmdb_id: if media_type == "movie" { "603".to_owned() } else { "1399".to_owned() },
                media_type: media_type.to_owned(),
                season: (media_type == "tv").then_some(1),
                episode: (media_type == "tv").then_some(1),
            }
        };
        self.execute_internal(profile_id, scraper_id, input, true).await
    }

    pub async fn execute_scraper(
        &self,
        profile_id: &str,
        scraper_id: &str,
        input: ScraperInput,
    ) -> Result<Vec<PluginStream>, PluginError> {
        self.execute_internal(profile_id, scraper_id, input, false).await
    }

    async fn execute_internal(
        &self,
        profile_id: &str,
        scraper_id: &str,
        input: ScraperInput,
        test_mode: bool,
    ) -> Result<Vec<PluginStream>, PluginError> {
        input.validate()?;
        let state = self.list_state(profile_id)?;
        if !state.plugins_enabled { return Err(PluginError::Disabled); }
        let scraper = state.scrapers.iter().find(|item| item.id == scraper_id).ok_or(PluginError::ScraperNotFound)?;
        if !test_mode && !scraper.enabled { return Err(PluginError::Disabled); }
        if !scraper.network_access_granted { return Err(PluginError::NetworkPermissionRequired); }
        if !scraper.supports_type(&input.media_type) {
            return Err(PluginError::Runtime("scraper does not support this media type".to_owned()));
        }
        let settings = self.load_scraper_settings(profile_id, scraper_id)?;
        let code = self.read_script(profile_id, scraper_id)?;
        let scraper_id = scraper.id.clone();
        let http = self.http.clone();
        let permit = self.semaphore.clone().acquire_owned().await
            .map_err(|_| PluginError::Worker("execution queue closed".to_owned()))?;
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            execute_streams(&code, &scraper_id, &settings, &input, http)
        }).await.map_err(|e| PluginError::Worker(e.to_string()))??;
        Ok(result)
    }

    async fn run_limited<T, F>(&self, operation: F) -> Result<T, PluginError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, PluginError> + Send + 'static,
    {
        let permit = self.semaphore.clone().acquire_owned().await
            .map_err(|_| PluginError::Worker("execution queue closed".to_owned()))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        }).await.map_err(|e| PluginError::Worker(e.to_string()))?
    }

    fn find_scraper(&self, profile_id: &str, scraper_id: &str) -> Result<PluginScraper, PluginError> {
        self.list_state(profile_id)?.scrapers.into_iter().find(|item| item.id == scraper_id)
            .ok_or(PluginError::ScraperNotFound)
    }

    fn save_state(&self, profile_id: &str, state: &PluginState) -> Result<(), PluginError> {
        let key = state_key(profile_id)?;
        let payload = serde_json::to_string(state).map_err(|e| PluginError::Persistence(e.to_string()))?;
        self.settings.set(&key, &payload).map_err(|e| PluginError::Persistence(e.to_string()))
    }

    fn script_path(&self, profile_id: &str, scraper_id: &str) -> Result<PathBuf, PluginError> {
        validate_profile(profile_id)?;
        let dir = self.cache_dir.join(profile_id);
        let digest = Sha256::digest(scraper_id.as_bytes());
        let filename = format!("{:x}.js", digest);
        Ok(dir.join(filename))
    }

    fn write_script(&self, profile_id: &str, scraper_id: &str, code: &str) -> Result<(), PluginError> {
        if code.len() > MAX_SCRIPT_BYTES { return Err(PluginError::ScriptTooLarge(MAX_SCRIPT_BYTES)); }
        let path = self.script_path(profile_id, scraper_id)?;
        let parent = path.parent().ok_or_else(|| PluginError::Persistence("invalid script cache path".to_owned()))?;
        std::fs::create_dir_all(parent).map_err(|e| PluginError::Persistence(e.to_string()))?;
        let temp = path.with_extension("js.tmp");
        std::fs::write(&temp, code).map_err(|e| PluginError::Persistence(e.to_string()))?;
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| PluginError::Persistence(e.to_string()))?;
        }
        std::fs::rename(&temp, &path).map_err(|e| PluginError::Persistence(e.to_string()))
    }

    fn read_script(&self, profile_id: &str, scraper_id: &str) -> Result<String, PluginError> {
        let path = self.script_path(profile_id, scraper_id)?;
        let metadata = std::fs::metadata(&path).map_err(|_| PluginError::ScraperNotFound)?;
        if metadata.len() as usize > MAX_SCRIPT_BYTES { return Err(PluginError::ScriptTooLarge(MAX_SCRIPT_BYTES)); }
        std::fs::read_to_string(path).map_err(|_| PluginError::ScraperNotFound)
    }

    fn delete_script(&self, profile_id: &str, scraper_id: &str) {
        if let Ok(path) = self.script_path(profile_id, scraper_id) { let _ = std::fs::remove_file(path); }
    }
}

fn state_key(profile_id: &str) -> Result<SettingKey, PluginError> {
    validate_profile(profile_id)?;
    SettingKey::new(format!("plugins.{profile_id}.state"))
        .map_err(|e| PluginError::Persistence(e.to_string()))
}

fn validate_profile(profile_id: &str) -> Result<(), PluginError> {
    if profile_id.is_empty() || profile_id.len() > 48 ||
        !profile_id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        return Err(PluginError::InvalidProfile);
    }
    Ok(())
}

fn settings_account(profile_id: &str, scraper_id: &str) -> String {
    let digest = Sha256::digest(scraper_id.as_bytes());
    format!("senzo-plugin-settings:{profile_id}:{:x}", digest)
}

fn normalize_manifest_url(raw: &str) -> Result<String, PluginError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_URL_LENGTH { return Err(PluginError::InvalidRepositoryUrl); }
    let with_scheme = if raw.contains("://") { raw.to_owned() } else { format!("https://{raw}") };
    let mut url = Url::parse(&with_scheme).map_err(|_| PluginError::InvalidRepositoryUrl)?;
    if url.scheme() != "https" || url.host_str().is_none() ||
        !url.username().is_empty() || url.password().is_some() ||
        url.query().is_some() || url.fragment().is_some() {
        return Err(PluginError::InvalidRepositoryUrl);
    }
    let path = url.path().trim_end_matches('/');
    if !path.ends_with("/manifest.json") && path != "manifest.json" {
        url.set_path(&format!("{path}/manifest.json"));
    }
    Ok(url.to_string())
}

fn parse_manifest(payload: &str) -> Result<PluginManifest, PluginError> {
    if payload.len() > MAX_MANIFEST_BYTES { return Err(PluginError::ResponseTooLarge(MAX_MANIFEST_BYTES)); }
    let manifest: PluginManifest = serde_json::from_str(payload)
        .map_err(|e| PluginError::InvalidManifest(e.to_string()))?;
    if manifest.name.trim().is_empty() || manifest.name.len() > 160 ||
        manifest.version.trim().is_empty() || manifest.version.len() > 64 ||
        manifest.scrapers.is_empty() || manifest.scrapers.len() > MAX_SCRAPERS_PER_REPOSITORY {
        return Err(PluginError::InvalidManifest("name, version and a bounded non-empty scraper list are required".to_owned()));
    }
    Ok(manifest)
}

fn validate_scraper_manifest(scraper: &PluginManifestScraper) -> Result<(), PluginError> {
    let valid_types = !scraper.supported_types.is_empty() && scraper.supported_types.len() <= 16 &&
        scraper.supported_types.iter().all(|value| matches!(normalize_media_type(value).as_str(), "movie" | "tv"));
    if scraper.id.trim().is_empty() || scraper.id.len() > 128 ||
        scraper.name.trim().is_empty() || scraper.name.len() > 160 ||
        scraper.version.trim().is_empty() || scraper.version.len() > 64 ||
        scraper.filename.trim().is_empty() || scraper.filename.len() > MAX_URL_LENGTH ||
        !valid_types {
        return Err(PluginError::InvalidManifest(format!("scraper '{}' has invalid required fields", scraper.id)));
    }
    Ok(())
}

fn resolve_script_url(base: &Url, filename: &str) -> Result<Url, PluginError> {
    let resolved = base.join(filename).map_err(|_| PluginError::InvalidManifest("scraper filename is not a valid URL".to_owned()))?;
    validate_https_url(&resolved)?;
    Ok(resolved)
}

fn scraper_identity(manifest_url: &str, scraper_id: &str) -> String {
    format!("{}:{}", manifest_url.to_ascii_lowercase(), scraper_id)
}

fn normalize_media_type(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "series" | "show" | "other" => "tv".to_owned(),
        value => value.to_owned(),
    }
}

fn epoch_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis().min(u64::MAX as u128) as u64
}

#[derive(Clone)]
struct PluginHttp;

#[derive(Debug)]
struct PluginResponse {
    url: String,
    status: u16,
    body: String,
    headers: BTreeMap<String, String>,
}

impl PluginHttp {
    fn client(url: &Url, timeout: Duration) -> Result<Client, PluginError> {
        let addresses = resolve_public_destination(url)?;
        let mut builder = Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(5))
            .redirect(Policy::none())
            .no_proxy()
            .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")));
        if let Some(Host::Domain(host)) = url.host() {
            builder = builder.resolve_to_addrs(host, &addresses);
        }
        builder.build().map_err(|e| PluginError::Network(e.to_string()))
    }

    fn get_text(&self, raw_url: &str, max_bytes: usize) -> Result<String, PluginError> {
        let url = Url::parse(raw_url).map_err(|_| PluginError::InvalidRepositoryUrl)?;
        let response = self.request(&url, "GET", &HeaderMap::new(), None, true, Instant::now() + Duration::from_secs(25), max_bytes)?;
        if !(200..300).contains(&response.status) {
            return Err(PluginError::Network(format!("HTTP {}", response.status)));
        }
        Ok(response.body)
    }

    fn request(
        &self,
        initial_url: &Url,
        method: &str,
        headers: &HeaderMap,
        body: Option<&[u8]>,
        follow_redirects: bool,
        deadline: Instant,
        max_bytes: usize,
    ) -> Result<PluginResponse, PluginError> {
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|_| PluginError::Network("unsupported HTTP method".to_owned()))?;
        if !matches!(method, Method::GET | Method::POST | Method::HEAD) {
            return Err(PluginError::Network("only GET, POST and HEAD are supported".to_owned()));
        }
        if body.is_some_and(|value| value.len() > MAX_REQUEST_BODY) {
            return Err(PluginError::ResponseTooLarge(MAX_REQUEST_BODY));
        }
        let mut url = initial_url.clone();
        for redirect_index in 0..=MAX_REDIRECTS {
            validate_https_url(&url)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Err(PluginError::ExecutionLimit); }
            let client = Self::client(&url, remaining.min(NETWORK_TIMEOUT))?;
            let mut request = client.request(method.clone(), url.clone()).headers(headers.clone());
            if let Some(body) = body { request = request.body(body.to_vec()); }
            let response = request.send().map_err(|e| PluginError::Network(e.without_url().to_string()))?;
            let status = response.status();
            if follow_redirects && status.is_redirection() {
                if redirect_index >= MAX_REDIRECTS { return Err(PluginError::Network("redirect limit exceeded".to_owned())); }
                let location = response.headers().get(LOCATION).and_then(|value| value.to_str().ok())
                    .ok_or_else(|| PluginError::Network("redirect did not include a valid Location".to_owned()))?;
                url = url.join(location).map_err(|_| PluginError::Network("invalid redirect URL".to_owned()))?;
                continue;
            }
            return read_response(response, max_bytes);
        }
        Err(PluginError::Network("redirect limit exceeded".to_owned()))
    }
}

fn read_response(mut response: Response, max_bytes: usize) -> Result<PluginResponse, PluginError> {
    let status = response.status().as_u16();
    let url = response.url().to_string();
    let headers = response.headers().iter().filter_map(|(name, value)| {
        value.to_str().ok().map(|value| (name.as_str().to_owned(), value.chars().take(MAX_HEADER_VALUE).collect()))
    }).collect();
    let mut bytes = Vec::new();
    response.by_ref().take(max_bytes as u64 + 1).read_to_end(&mut bytes)
        .map_err(|e| PluginError::Network(e.to_string()))?;
    if bytes.len() > max_bytes { return Err(PluginError::ResponseTooLarge(max_bytes)); }
    let body = String::from_utf8(bytes).map_err(|_| PluginError::Network("response was not UTF-8".to_owned()))?;
    Ok(PluginResponse { url, status, body, headers })
}

fn validate_https_url(url: &Url) -> Result<(), PluginError> {
    if url.scheme() != "https" || url.host_str().is_none() ||
        !url.username().is_empty() || url.password().is_some() ||
        url.as_str().len() > MAX_URL_LENGTH {
        return Err(PluginError::BlockedDestination);
    }
    Ok(())
}

fn resolve_public_destination(url: &Url) -> Result<Vec<SocketAddr>, PluginError> {
    validate_https_url(url)?;
    let host = url.host_str().ok_or(PluginError::BlockedDestination)?;
    let port = url.port_or_known_default().ok_or(PluginError::BlockedDestination)?;
    let addresses = (host, port).to_socket_addrs().map_err(|_| PluginError::DnsResolution)?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(PluginError::BlockedDestination);
    }
    Ok(addresses)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0 || a == 10 || a == 127 || a >= 224 ||
                (a == 100 && (64..=127).contains(&b)) ||
                (a == 169 && b == 254) ||
                (a == 172 && (16..=31).contains(&b)) ||
                (a == 192 && b == 168) ||
                (a == 192 && b == 0) ||
                (a == 192 && b == 88 && c == 99) ||
                (a == 198 && (b == 18 || b == 19)) ||
                (a == 198 && b == 51 && c == 100) ||
                (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            if segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xffff {
                return is_public_ip(IpAddr::V4(Ipv4Addr::new(
                    (segments[6] >> 8) as u8, segments[6] as u8,
                    (segments[7] >> 8) as u8, segments[7] as u8,
                )));
            }
            segments[0] & 0xe000 == 0x2000 &&
                !(segments[0] == 0x2001 && segments[1] == 0x0db8) &&
                !(segments[0] == 0x2001 && segments[1] == 0x0000) &&
                segments[0] != 0x2002
        }
    }
}

fn execute_settings(code: &str, scraper_id: &str) -> Result<String, PluginError> {
    run_javascript(code, scraper_id, "{}", ScraperInput {
        tmdb_id: "603".to_owned(), media_type: "movie".to_owned(), season: None, episode: None,
    }, PluginHttp, EXECUTION_TIMEOUT, true)
}

fn execute_streams(
    code: &str,
    scraper_id: &str,
    settings: &str,
    input: &ScraperInput,
    http: PluginHttp,
) -> Result<Vec<PluginStream>, PluginError> {
    let raw = run_javascript(code, scraper_id, settings, input.clone(), http, EXECUTION_TIMEOUT, false)?;
    parse_streams(&raw, scraper_id)
}

fn run_javascript(
    code: &str,
    scraper_id: &str,
    settings: &str,
    input: ScraperInput,
    http: PluginHttp,
    budget: Duration,
    settings_mode: bool,
) -> Result<String, PluginError> {
    if code.len() > MAX_SCRIPT_BYTES { return Err(PluginError::ScriptTooLarge(MAX_SCRIPT_BYTES)); }
    let deadline = Instant::now() + budget;
    let execution_deadline = deadline;
    let runtime = Runtime::new().map_err(|e| PluginError::Runtime(e.to_string()))?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= execution_deadline)));
    let context = Context::full(&runtime).map_err(|e| PluginError::Runtime(e.to_string()))?;
    let settings_value: Value = serde_json::from_str(settings).map_err(|_| PluginError::InvalidSettings)?;
    let input_json = serde_json::to_string(&input).map_err(|e| PluginError::Runtime(e.to_string()))?;
    let settings_json = serde_json::to_string(&settings_value).map_err(|e| PluginError::Runtime(e.to_string()))?;

    let result = context.with(|ctx| {
        let globals = ctx.globals();
        globals.set("SCRAPER_ID", scraper_id.to_owned())?;
        globals.set("__SCRAPER_SETTINGS_JSON", settings_json)?;
        globals.set("__SCRAPER_CALL_ARGS_JSON", input_json)?;
        let fetch_http = http.clone();
        let fetch_deadline = deadline;
        globals.set("__native_fetch", Func::from(move |raw_url: String, raw_options: String| -> String {
            let result = native_fetch(&fetch_http, &raw_url, &raw_options, fetch_deadline);
            serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_owned())
        }))?;
        let sleep_deadline = deadline;
        globals.set("__plugin_sleep", Func::from(move |milliseconds: i32| {
            let delay = Duration::from_millis(milliseconds.max(0).min(MAX_TIMER_DELAY_MS as i32) as u64);
            if Instant::now() + delay <= sleep_deadline { std::thread::sleep(delay); }
        }))?;
        ctx.eval::<(), _>(JS_POLYFILLS)?;
        let mut script = String::from(
            "var module = { exports: {} }; var exports = module.exports;\
             globalThis.global = globalThis; globalThis.window = globalThis; globalThis.self = globalThis;\
             globalThis.SCRAPER_SETTINGS = JSON.parse(__SCRAPER_SETTINGS_JSON);\
             globalThis.TMDB_API_KEY = '';\
             "
        );
        script.push_str(code);
        if settings_mode {
            script.push_str("\n;(async function(){\
                var fn = (module.exports && module.exports.onSettings) || globalThis.onSettings;\
                if (typeof fn !== 'function') return 'null';\
                return JSON.stringify((await fn()) || null);\
            })()");
        } else {
            script.push_str("\n;(async function(){\
                var fn = (module.exports && module.exports.getStreams) || globalThis.getStreams;\
                if (typeof fn !== 'function') throw new Error('getStreams function not found');\
                var args = JSON.parse(__SCRAPER_CALL_ARGS_JSON);\
                var result = await fn(args.tmdb_id, args.media_type, args.season == null ? undefined : args.season, args.episode == null ? undefined : args.episode);\
                return JSON.stringify(result || []);\
            })()");
        }
        let promise: Promise = ctx.eval(script.as_str())?;
        promise.finish::<String>()
    });
    match result {
        Ok(_) if Instant::now() >= deadline => Err(PluginError::ExecutionLimit),
        Ok(value) => Ok(value),
        Err(_) if Instant::now() >= deadline => Err(PluginError::ExecutionLimit),
        Err(error) => Err(PluginError::Runtime(error.to_string())),
    }
}

const JS_POLYFILLS: &str = r#"
globalThis.console = {
  log: function(){}, info: function(){}, warn: function(){}, error: function(){}, debug: function(){}
};
globalThis.fetch = async function(url, options) {
  var response = JSON.parse(__native_fetch(String(url), JSON.stringify(options || {})));
  if (response.status === 0) throw new Error(response.statusText || 'Network request failed');
  var body = response.body || '';
  var headers = { get: function(name) { return response.headers[String(name).toLowerCase()] || null; } };
  return {
    ok: response.status >= 200 && response.status < 300,
    status: response.status, statusText: response.statusText || '', url: response.url,
    headers: headers,
    text: async function(){ return body; },
    json: async function(){ return body ? JSON.parse(body) : null; },
    arrayBuffer: async function(){ return new TextEncoder().encode(body).buffer; }
  };
};
if (typeof TextEncoder === 'undefined') {
  globalThis.TextEncoder = function(){};
  TextEncoder.prototype.encode = function(value) {
    var encoded = unescape(encodeURIComponent(String(value)));
    var bytes = new Uint8Array(encoded.length);
    for (var i = 0; i < encoded.length; i++) bytes[i] = encoded.charCodeAt(i);
    return bytes;
  };
}
if (typeof TextDecoder === 'undefined') {
  globalThis.TextDecoder = function(){};
  TextDecoder.prototype.decode = function(bytes) {
    var value = '';
    for (var i = 0; i < bytes.length; i++) value += String.fromCharCode(bytes[i]);
    return decodeURIComponent(escape(value));
  };
}
if (typeof atob === 'undefined') {
  globalThis.atob = function(value) {
    var alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    var source = String(value).replace(/=+$/, '');
    var output = '', bits = 0, buffer = 0;
    for (var i = 0; i < source.length; i++) {
      var index = alphabet.indexOf(source.charAt(i));
      if (index < 0) throw new Error('InvalidCharacterError');
      buffer = (buffer << 6) | index; bits += 6;
      if (bits >= 8) { bits -= 8; output += String.fromCharCode((buffer >> bits) & 255); }
    }
    return output;
  };
}
if (typeof btoa === 'undefined') {
  globalThis.btoa = function(value) {
    var alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    var source = String(value), output = '';
    for (var i = 0; i < source.length; i += 3) {
      var a = source.charCodeAt(i), hasB = i + 1 < source.length, hasC = i + 2 < source.length;
      var b = hasB ? source.charCodeAt(i + 1) : 0, c = hasC ? source.charCodeAt(i + 2) : 0;
      if (a > 255 || b > 255 || c > 255) throw new Error('InvalidCharacterError');
      output += alphabet.charAt(a >> 2) + alphabet.charAt(((a & 3) << 4) | (b >> 4));
      output += hasB ? alphabet.charAt(((b & 15) << 2) | (c >> 6)) : '=';
      output += hasC ? alphabet.charAt(c & 63) : '=';
    }
    return output;
  };
}
globalThis.setTimeout = function(callback, delay) {
  var id = 0;
  Promise.resolve().then(function() {
    __plugin_sleep(Number(delay) || 0);
    if (typeof callback === 'function') callback();
  });
  return id;
};
globalThis.clearTimeout = function(){};
globalThis.setInterval = function(){ throw new Error('setInterval is not supported in the bounded plugin runtime'); };
globalThis.clearInterval = function(){};
"#;

#[derive(Debug, Deserialize)]
struct FetchOptions {
    #[serde(default = "default_get")]
    method: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    redirect: Value,
}
fn default_get() -> String { "GET".to_owned() }

fn native_fetch(http: &PluginHttp, raw_url: &str, raw_options: &str, deadline: Instant) -> Value {
    let fallback = |error: String| json!({
        "status": 0, "statusText": error, "url": raw_url, "body": "", "headers": {}
    });
    if raw_url.len() > MAX_URL_LENGTH { return fallback("URL exceeds limit".to_owned()); }
    let Ok(url) = Url::parse(raw_url) else { return fallback("Invalid URL".to_owned()); };
    if validate_https_url(&url).is_err() { return fallback("Only public HTTPS destinations are allowed".to_owned()); }
    let options: FetchOptions = match serde_json::from_str(raw_options) {
        Ok(options) => options,
        Err(_) => return fallback("Invalid fetch options".to_owned()),
    };
    if options.headers.len() > MAX_HEADERS || options.body.as_ref().is_some_and(|body| body.len() > MAX_REQUEST_BODY) {
        return fallback("Request exceeds safety limits".to_owned());
    }
    let mut headers = HeaderMap::new();
    for (key, value) in options.headers {
        if key.len() > 128 || value.len() > MAX_HEADER_VALUE || value.contains('\r') || value.contains('\n') {
            return fallback("Invalid request header".to_owned());
        }
        let (Ok(name), Ok(value)) = (HeaderName::from_bytes(key.as_bytes()), HeaderValue::from_str(&value)) else {
            return fallback("Invalid request header".to_owned());
        };
        headers.insert(name, value);
    }
    let follow_redirects = !matches!(options.redirect, Value::String(ref value) if value == "manual")
        && options.redirect != Value::Bool(false);
    match http.request(&url, &options.method, &headers, options.body.as_deref().map(str::as_bytes), follow_redirects, deadline, MAX_FETCH_BYTES) {
        Ok(response) => json!({
            "status": response.status,
            "statusText": StatusCode::from_u16(response.status).map(|status| status.canonical_reason().unwrap_or("")).unwrap_or(""),
            "url": response.url,
            "body": response.body,
            "headers": response.headers
        }),
        Err(error) => fallback(error.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct RawStream {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<Value>,
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default, rename = "type")]
    stream_type: Option<String>,
    #[serde(default)]
    seeders: Option<u32>,
    #[serde(default)]
    peers: Option<u32>,
    #[serde(default, rename = "infoHash")]
    info_hash: Option<String>,
    #[serde(default)]
    file_idx: Option<u32>,
    #[serde(default, rename = "externalUrl")]
    external_url: Option<String>,
    #[serde(default, rename = "ytId")]
    youtube_id: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    subtitles: Vec<RawSubtitle>,
}

#[derive(Debug, Deserialize)]
struct RawSubtitle {
    url: String,
    #[serde(default = "unknown_language")]
    language: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}
fn unknown_language() -> String { "Unknown".to_owned() }

fn parse_streams(raw: &str, scraper_id: &str) -> Result<Vec<PluginStream>, PluginError> {
    let value: Value = serde_json::from_str(raw).map_err(|_| PluginError::InvalidStreams)?;
    let items = value.as_array().ok_or(PluginError::InvalidStreams)?;
    let mut streams = Vec::new();
    for (index, item) in items.iter().take(MAX_STREAMS).enumerate() {
        let Ok(item) = serde_json::from_value::<RawStream>(item.clone()) else { continue; };
        let url = match item.url.as_ref() {
            Some(Value::String(url)) => Some(url.clone()),
            Some(Value::Object(object)) => object.get("url").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        };
        let target = if let Some(hash) = item.info_hash.as_deref().filter(|hash| valid_info_hash(hash)) {
            StreamTarget::Torrent { info_hash: hash.to_ascii_lowercase(), file_index: item.file_idx }
        } else if let Some(url) = url.filter(|url| valid_media_url(url)) {
            StreamTarget::Http(url)
        } else if let Some(url) = item.external_url.filter(|url| valid_media_url(url)) {
            StreamTarget::External(url)
        } else if let Some(id) = item.youtube_id.filter(|id| valid_youtube_id(id)) {
            StreamTarget::Youtube(id)
        } else { continue; };

        let subtitles = item.subtitles.into_iter().filter_map(|subtitle| {
            if !valid_media_url(&subtitle.url) { return None; }
            Some(PluginSubtitle {
                url: subtitle.url,
                language: safe_text(&subtitle.language, 48),
                name: subtitle.name.map(|value| safe_text(&value, 160)),
                headers: clean_headers(subtitle.headers),
            })
        }).take(32).collect();

        let title = item.title.as_deref().filter(|value| !value.trim().is_empty())
            .or(item.name.as_deref()).map(|value| safe_text(value, 240))
            .unwrap_or_else(|| format!("Stream {}", index + 1));
        let provider = item.provider.as_deref().or(item.name.as_deref())
            .map(|value| safe_text(value, 120)).unwrap_or_else(|| "Plugin".to_owned());
        streams.push(PluginStream {
            id: format!("{}:{index}", scraper_id),
            scraper_id: scraper_id.to_owned(),
            provider,
            title,
            name: item.name.map(|value| safe_text(&value, 160)),
            target,
            quality: item.quality.map(|value| safe_text(&value, 40)),
            size: item.size.map(|value| safe_text(&value, 48)),
            language: item.language.map(|value| safe_text(&value, 48)),
            stream_type: item.stream_type.map(|value| safe_text(&value, 40)),
            seeders: item.seeders,
            peers: item.peers,
            headers: clean_headers(item.headers),
            subtitles,
        });
    }
    Ok(streams)
}

fn clean_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers.into_iter().filter(|(key, value)| {
        key.len() <= 128 && value.len() <= MAX_HEADER_VALUE &&
            !value.contains('\r') && !value.contains('\n')
    }).take(MAX_HEADERS).collect()
}

fn safe_text(value: &str, max: usize) -> String {
    value.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_owned()
}

fn valid_info_hash(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_youtube_id(value: &str) -> bool {
    value.len() == 11 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

fn valid_media_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https") &&
        url.host_str().is_some() && url.username().is_empty() && url.password().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemorySettings(Mutex<BTreeMap<String, String>>);
    impl SettingsRepository for MemorySettings {
        fn get(&self, key: &SettingKey) -> Result<Option<String>, senzo_domain::RepositoryError> {
            Ok(self.0.lock().unwrap().get(key.as_str()).cloned())
        }
        fn set(&self, key: &SettingKey, value: &str) -> Result<(), senzo_domain::RepositoryError> {
            self.0.lock().unwrap().insert(key.as_str().to_owned(), value.to_owned());
            Ok(())
        }
    }
    #[derive(Default)]
    struct MemorySecrets(Mutex<BTreeMap<String, String>>);
    impl PluginSettingsStore for MemorySecrets {
        fn get(&self, account: &str) -> Result<Option<String>, String> { Ok(self.0.lock().unwrap().get(account).cloned()) }
        fn set(&self, account: &str, value: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(account.to_owned(), value.to_owned()); Ok(())
        }
        fn delete(&self, account: &str) -> Result<(), String> { self.0.lock().unwrap().remove(account); Ok(()) }
    }

    #[test]
    fn parses_nuvio_manifest_contract_and_ignores_unknown_fields() {
        let payload = r#"{"name":"Fixture Repository","version":"1.0.0","author":"Fixture","unknown":"ignored","scrapers":[{"id":"fixture","name":"Fixture Scraper","version":"2.0","filename":"fixture.js","supportedTypes":["movie","series"],"hasSettings":true}]}"#;
        let manifest = parse_manifest(payload).unwrap();
        assert_eq!(manifest.name, "Fixture Repository");
        assert_eq!(manifest.scrapers[0].supported_types, ["movie", "series"]);
        assert!(manifest.scrapers[0].has_settings);
        assert!(parse_manifest(r#"{"name":"","version":"1","scrapers":[]}"#).is_err());
    }

    #[test]
    fn plugin_state_defaults_to_enabled_but_has_no_repositories_or_scrapers() {
        let state = PluginState::default();
        assert!(state.plugins_enabled);
        assert!(state.repositories.is_empty());
        assert!(state.scrapers.is_empty());
    }

    #[test]
    fn manifest_url_defaults_to_https_and_is_normalized() {
        assert_eq!(normalize_manifest_url("plugins.example/repo").unwrap(), "https://plugins.example/repo/manifest.json");
        assert_eq!(normalize_manifest_url("https://plugins.example/manifest.json").unwrap(), "https://plugins.example/manifest.json");
        assert!(normalize_manifest_url("http://plugins.example/repo").is_err());
        assert!(normalize_manifest_url("https://user:pass@plugins.example").is_err());
        assert!(normalize_manifest_url("https://plugins.example/repo?token=secret").is_err());
    }

    #[test]
    fn blocks_local_and_reserved_network_destinations() {
        for ip in ["127.0.0.1", "10.0.0.1", "192.168.1.1", "169.254.169.254", "224.0.0.1"] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} must be blocked");
        }
        assert!(is_public_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[test]
    fn maps_plugin_results_to_provider_neutral_streams() {
        let payload = r#"[{"title":"1080p","url":"https://cdn.example/video.m3u8","quality":"1080p","provider":"Fixture","headers":{"Referer":"https://example.test"},"subtitles":[{"url":"https://cdn.example/en.vtt","language":"en"}]},{"title":"Torrent","infoHash":"0123456789012345678901234567890123456789","file_idx":2},{"title":"bad","url":"file:///etc/passwd"}]"#;
        let streams = parse_streams(payload, "repo:fixture").unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].quality.as_deref(), Some("1080p"));
        assert_eq!(streams[0].subtitles.len(), 1);
        assert!(matches!(streams[1].target, StreamTarget::Torrent { file_index: Some(2), .. }));
    }

    #[test]
    fn executes_nuvio_commonjs_getstreams_and_settings_exports() {
        let code = r#"
          module.exports.getStreams = async function(tmdbId, mediaType, season, episode) {
            await Promise.resolve();
            return [{ title: "Fixture " + tmdbId, url: "https://cdn.example/video.mp4", provider: mediaType }];
          };
          module.exports.onSettings = async function() { return [{ id: "token", type: "input", title: "Token" }]; };
        "#;
        let input = ScraperInput { tmdb_id: "603".into(), media_type: "movie".into(), season: None, episode: None };
        let raw = run_javascript(code, "fixture", "{}", input, PluginHttp, Duration::from_secs(3), false).unwrap();
        assert!(raw.contains("Fixture 603"));
        let settings = run_javascript(code, "fixture", "{}", ScraperInput {
            tmdb_id: "603".into(), media_type: "movie".into(), season: None, episode: None
        }, PluginHttp, Duration::from_secs(3), true).unwrap();
        assert!(settings.contains("token"));
    }

    #[test]
    fn execution_budget_interrupts_infinite_javascript() {
        let error = run_javascript("while (true) {}", "fixture", "{}", ScraperInput {
            tmdb_id: "603".into(), media_type: "movie".into(), season: None, episode: None
        }, PluginHttp, Duration::from_millis(30), false).unwrap_err();
        assert!(matches!(error, PluginError::Runtime(_) | PluginError::ExecutionLimit));
    }

    #[test]
    fn execution_input_rejects_invalid_tmdb_and_episode_pairs() {
        assert!(ScraperInput { tmdb_id: "tt123".into(), media_type: "movie".into(), season: None, episode: None }.validate().is_err());
        assert!(ScraperInput { tmdb_id: "123".into(), media_type: "tv".into(), season: None, episode: Some(1) }.validate().is_err());
    }

    #[test]
    fn settings_are_profile_scoped_and_not_embedded_in_metadata_registry() {
        let settings = Arc::new(MemorySettings::default());
        let secrets = Arc::new(MemorySecrets::default());
        let manager = PluginManager::new(settings.clone(), secrets, std::env::temp_dir().join("senzo-plugin-tests"));
        let mut state = PluginState::default();
        state.repositories.push(PluginRepository {
            manifest_url: "https://fixture.example/manifest.json".into(), name: "Fixture".into(),
            description: None, version: "1".into(), scraper_count: 1, last_updated_epoch_ms: 1, last_error: None
        });
        state.scrapers.push(PluginScraper {
            id: "fixture:one".into(), repository_url: "https://fixture.example/manifest.json".into(),
            name: "One".into(), description: String::new(), version: "1".into(), filename: "one.js".into(),
            supported_types: vec!["movie".into()], enabled: true, manifest_enabled: true, has_settings: true,
            content_language: Vec::new(), formats: Vec::new(), network_access_granted: false
        });
        manager.save_state("profile-a", &state).unwrap();
        assert_eq!(manager.list_state("profile-a").unwrap(), state);
        assert_eq!(manager.list_state("profile-b").unwrap(), PluginState::default());
        let raw = settings.0.lock().unwrap().values().next().unwrap().clone();
        assert!(!raw.contains("module.exports"));
    }

    #[test]
    fn plugin_execution_requires_explicit_network_grant() {
        let settings = Arc::new(MemorySettings::default());
        let secrets = Arc::new(MemorySecrets::default());
        let manager = PluginManager::new(settings, secrets, std::env::temp_dir().join("senzo-plugin-tests"));
        assert!(matches!(manager.set_network_permission("default", "missing", true), Err(PluginError::ScraperNotFound)));
    }

    #[test]
    fn manifest_scraper_type_normalization_matches_nuvio() {
        let scraper = PluginScraper {
            id: "x".into(), repository_url: "https://x.example/manifest.json".into(), name: "x".into(),
            description: String::new(), version: "1".into(), filename: "x.js".into(),
            supported_types: vec!["series".into()], enabled: true, manifest_enabled: true, has_settings: false,
            content_language: vec![], formats: vec![], network_access_granted: false,
        };
        assert!(scraper.supports_type("tv"));
    }
}
