//! Provider-neutral source discovery, normalization, grouping and policy.
//! Add-ons, Nuvio scrapers and media servers remain isolated adapters.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use senzo_addons::{AddonManager, AddonStream};
use senzo_debrid::{DebridFileHint, DebridManager, DebridProviderId, ResolvedDebridSource};
use senzo_domain::ProfileScope;
use senzo_integrations::MediaServerManager;
use senzo_plugins::{PluginManager, PluginStream, ScraperInput, StreamTarget as PluginTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    sync::{RwLock, Semaphore},
    task::JoinSet,
    time::timeout,
};
use url::Url;

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(28);
const CACHE_TTL: Duration = Duration::from_secs(45);
const STALE_TTL: Duration = Duration::from_secs(600);
const CACHE_CAPACITY: usize = 128;
const MAX_PROVIDERS: usize = 48;
const MAX_STREAMS_PER_PROVIDER: usize = 100;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_VALUE_BYTES: usize = 8192;
const MAX_SUBTITLES: usize = 32;
const MAX_CONCURRENT_PROVIDERS: usize = 12;
const MAX_URL_LENGTH: usize = 8 * 1024;

#[derive(Debug, Error, Clone)]
pub enum SourceError {
    #[error("provider request was invalid: {0}")]
    InvalidRequest(String),
    #[error("provider is unavailable: {0}")]
    Provider(String),
    #[error("source response was invalid")]
    InvalidResponse,
    #[error("provider request timed out")]
    Timeout,
    #[error("source worker was cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTarget {
    Http(String),
    Torrent {
        info_hash: String,
        magnet_uri: Option<String>,
        file_index: Option<u32>,
    },
    External(String),
    Youtube(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSubtitle {
    pub url: String,
    pub language: String,
    pub name: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBadge {
    pub label: String,
    pub tone: BadgeTone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeTone {
    Neutral,
    Accent,
    Positive,
    Warning,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    #[default]
    Unknown,
    Checking,
    Cached,
    NotCached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub title: String,
    pub description: Option<String>,
    #[serde(default)]
    pub filename_hint: Option<String>,
    /// Stremio behaviorHints.bingeGroup, used to prefer the same release group across episodes.
    #[serde(default)]
    pub binge_group: Option<String>,
    pub target: SourceTarget,
    pub quality: Option<String>,
    pub size_bytes: Option<u64>,
    pub language: Option<String>,
    pub codec: Option<String>,
    pub stream_type: Option<String>,
    pub seeders: Option<u32>,
    pub peers: Option<u32>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub subtitles: Vec<SourceSubtitle>,
    #[serde(default)]
    pub badges: Vec<SourceBadge>,
    #[serde(default)]
    pub cache_state: CacheState,
}

impl Source {
    pub fn is_external(&self) -> bool {
        matches!(
            &self.target,
            SourceTarget::External(_) | SourceTarget::Youtube(_)
        )
    }
    pub fn is_torrent(&self) -> bool {
        matches!(&self.target, SourceTarget::Torrent { .. })
    }
    pub fn is_direct(&self) -> bool {
        matches!(&self.target, SourceTarget::Http(_))
    }
    pub fn playable_url(&self) -> Option<&str> {
        match &self.target {
            SourceTarget::Http(url) => Some(url),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceRequest {
    pub media_type: String,
    /// Provider-facing identifier, typically tmdb:<id> or an IMDb ID.
    pub media_id: String,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl SourceRequest {
    pub fn validate(&self) -> Result<(), SourceError> {
        if !matches!(self.media_type.as_str(), "movie" | "tv" | "series") {
            return Err(SourceError::InvalidRequest(
                "media type must be movie or tv".into(),
            ));
        }
        if self.media_id.trim().is_empty()
            || self.media_id.len() > 256
            || self.media_id.chars().any(char::is_control)
        {
            return Err(SourceError::InvalidRequest(
                "media identifier is invalid".into(),
            ));
        }
        if self.title.trim().is_empty()
            || self.title.len() > 512
            || self.title.chars().any(char::is_control)
        {
            return Err(SourceError::InvalidRequest("media title is invalid".into()));
        }
        if self.episode.is_some() && self.season.is_none() {
            return Err(SourceError::InvalidRequest(
                "an episode requires a season number".into(),
            ));
        }
        if self.media_type == "movie" && (self.season.is_some() || self.episode.is_some()) {
            return Err(SourceError::InvalidRequest(
                "movie requests cannot contain episode coordinates".into(),
            ));
        }
        Ok(())
    }

    pub fn plugin_input(&self) -> Result<ScraperInput, SourceError> {
        let tmdb_id = self
            .tmdb_id
            .clone()
            .unwrap_or_else(|| self.media_id.trim_start_matches("tmdb:").to_owned());
        Ok(ScraperInput {
            tmdb_id,
            media_type: if self.media_type == "movie" {
                "movie".into()
            } else {
                "tv".into()
            },
            season: self.season,
            episode: self.episode,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourceFilters {
    pub provider_ids: HashSet<String>,
    pub minimum_seeders: Option<u32>,
    pub quality: Option<String>,
    pub include_external: bool,
    pub include_torrents: bool,
    pub include_direct: bool,
}

impl SourceFilters {
    pub fn all() -> Self {
        Self {
            include_external: true,
            include_torrents: true,
            include_direct: true,
            ..Self::default()
        }
    }

    pub fn apply(&self, sources: &[Source]) -> Vec<Source> {
        sources
            .iter()
            .filter(|source| {
                (self.provider_ids.is_empty() || self.provider_ids.contains(&source.provider_id))
                    && (self
                        .minimum_seeders
                        .is_none_or(|min| source.seeders.unwrap_or(0) >= min))
                    && (self.quality.as_ref().is_none_or(|q| {
                        source
                            .quality
                            .as_ref()
                            .is_some_and(|v| v.eq_ignore_ascii_case(q))
                    }))
                    && ((self.include_external && source.is_external())
                        || (self.include_torrents && source.is_torrent())
                        || (self.include_direct && source.is_direct()))
            })
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSort {
    Recommended,
    Quality,
    Seeders,
    Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoSelectPolicy {
    Disabled,
    Recommended,
    HighestQuality,
    CachedFirst,
    HighestSeeders,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceGroup {
    pub provider_id: String,
    pub provider_name: String,
    pub sources: Vec<Source>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDiagnostic {
    pub provider_id: String,
    pub provider_name: String,
    pub duration_ms: u64,
    pub returned_sources: usize,
    pub error: Option<String>,
    pub from_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDiscoveryResult {
    pub groups: Vec<SourceGroup>,
    pub diagnostics: Vec<ProviderDiagnostic>,
    pub total_sources: usize,
    pub used_stale_cache: bool,
    pub recommended_source_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AdapterOutput {
    pub sources: Vec<Source>,
}

#[async_trait]
pub trait SourceAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    async fn discover(&self, request: &SourceRequest) -> Result<AdapterOutput, SourceError>;
}

#[derive(Clone)]
pub struct AddonSourceAdapter {
    manager: AddonManager,
    profile_id: String,
    profile_scope: Option<ProfileScope>,
}

impl AddonSourceAdapter {
    pub fn new(manager: AddonManager, profile_id: impl Into<String>) -> Self {
        Self {
            manager,
            profile_id: profile_id.into(),
            profile_scope: None,
        }
    }

    pub fn new_scoped(manager: AddonManager, profile_scope: ProfileScope) -> Self {
        Self {
            manager,
            profile_id: "default".to_owned(),
            profile_scope: Some(profile_scope),
        }
    }

    fn active_profile_id(&self) -> String {
        self.profile_scope
            .as_ref()
            .map(ProfileScope::current)
            .unwrap_or_else(|| self.profile_id.clone())
    }
}

#[async_trait]
impl SourceAdapter for AddonSourceAdapter {
    fn id(&self) -> &'static str {
        "stremio-addons"
    }
    fn name(&self) -> &'static str {
        "Stremio add-ons"
    }

    async fn discover(&self, request: &SourceRequest) -> Result<AdapterOutput, SourceError> {
        let active_profile_id = self.active_profile_id();
        let addons = self
            .manager
            .list_addons(&active_profile_id)
            .map_err(|error| SourceError::Provider(error.to_string()))?;
        let mut jobs = JoinSet::new();
        for addon in addons.into_iter().filter(|addon| addon.enabled) {
            let Some(manifest) = addon.manifest.clone() else {
                continue;
            };
            let media_type = if request.media_type == "movie" {
                "movie"
            } else {
                "series"
            };
            if !manifest.supports_resource("stream", media_type, &request.media_id) {
                continue;
            }
            let manager = self.manager.clone();
            let profile = active_profile_id.clone();
            let request = request.clone();
            jobs.spawn(async move {
                let result = manager
                    .fetch_streams(&profile, &addon.id, media_type, &request.media_id)
                    .await;
                (addon.id, manifest.name, result)
            });
        }
        let mut sources = Vec::new();
        while let Some(joined) = jobs.join_next().await {
            let (id, name, result) =
                joined.map_err(|error| SourceError::Provider(error.to_string()))?;
            match result {
                Ok(result) => {
                    for (index, stream) in result
                        .streams
                        .into_iter()
                        .take(MAX_STREAMS_PER_PROVIDER)
                        .enumerate()
                    {
                        if let Some(source) = addon_stream_to_source(&id, &name, index, stream) {
                            sources.push(source);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(provider_id=%id, error=%error, "Stremio add-on source request failed")
                }
            }
        }
        Ok(AdapterOutput { sources })
    }
}

#[derive(Clone)]
pub struct PluginSourceAdapter {
    manager: PluginManager,
    profile_id: String,
    profile_scope: Option<ProfileScope>,
}

impl PluginSourceAdapter {
    pub fn new(manager: PluginManager, profile_id: impl Into<String>) -> Self {
        Self {
            manager,
            profile_id: profile_id.into(),
            profile_scope: None,
        }
    }

    pub fn new_scoped(manager: PluginManager, profile_scope: ProfileScope) -> Self {
        Self {
            manager,
            profile_id: "default".to_owned(),
            profile_scope: Some(profile_scope),
        }
    }

    fn active_profile_id(&self) -> String {
        self.profile_scope
            .as_ref()
            .map(ProfileScope::current)
            .unwrap_or_else(|| self.profile_id.clone())
    }
}

#[async_trait]
impl SourceAdapter for PluginSourceAdapter {
    fn id(&self) -> &'static str {
        "nuvio-scrapers"
    }
    fn name(&self) -> &'static str {
        "Nuvio scrapers"
    }

    async fn discover(&self, request: &SourceRequest) -> Result<AdapterOutput, SourceError> {
        let active_profile_id = self.active_profile_id();
        let state = self
            .manager
            .list_state(&active_profile_id)
            .map_err(|error| SourceError::Provider(error.to_string()))?;
        if !state.plugins_enabled {
            return Ok(AdapterOutput {
                sources: Vec::new(),
            });
        }
        let input = request.plugin_input()?;
        let mut jobs = JoinSet::new();
        for scraper in state
            .scrapers
            .into_iter()
            .filter(|s| s.enabled && s.supports_type(&input.media_type) && s.network_access_granted)
        {
            let manager = self.manager.clone();
            let profile = active_profile_id.clone();
            let input = input.clone();
            jobs.spawn(async move {
                let id = scraper.id.clone();
                let name = scraper.name.clone();
                (
                    id.clone(),
                    name,
                    manager.execute_scraper(&profile, &id, input).await,
                )
            });
        }
        let mut sources = Vec::new();
        while let Some(joined) = jobs.join_next().await {
            let (id, name, result) =
                joined.map_err(|error| SourceError::Provider(error.to_string()))?;
            match result {
                Ok(streams) => {
                    for (index, stream) in streams
                        .into_iter()
                        .take(MAX_STREAMS_PER_PROVIDER)
                        .enumerate()
                    {
                        if let Some(source) = plugin_stream_to_source(&id, &name, index, stream) {
                            sources.push(source);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(provider_id=%id, error=%error, "Nuvio scraper source request failed")
                }
            }
        }
        Ok(AdapterOutput { sources })
    }
}

#[derive(Clone)]
pub struct MediaServerSourceAdapter {
    manager: MediaServerManager,
    profile_id: String,
    profile_scope: Option<ProfileScope>,
}

impl MediaServerSourceAdapter {
    pub fn new(manager: MediaServerManager, profile_id: impl Into<String>) -> Self {
        Self {
            manager,
            profile_id: profile_id.into(),
            profile_scope: None,
        }
    }

    pub fn new_scoped(manager: MediaServerManager, profile_scope: ProfileScope) -> Self {
        Self {
            manager,
            profile_id: "default".to_owned(),
            profile_scope: Some(profile_scope),
        }
    }

    fn active_profile_id(&self) -> String {
        self.profile_scope
            .as_ref()
            .map(ProfileScope::current)
            .unwrap_or_else(|| self.profile_id.clone())
    }
}

#[async_trait]
impl SourceAdapter for MediaServerSourceAdapter {
    fn id(&self) -> &'static str {
        "media-servers"
    }
    fn name(&self) -> &'static str {
        "Media servers"
    }

    async fn discover(&self, request: &SourceRequest) -> Result<AdapterOutput, SourceError> {
        let active_profile_id = self.active_profile_id();
        let matches = self
            .manager
            .find_and_resolve(
                &active_profile_id,
                &request.media_type,
                &request.title,
                request.year,
                request.tmdb_id.as_deref(),
                request.imdb_id.as_deref(),
                request.season,
                request.episode,
            )
            .await
            .map_err(|error| SourceError::Provider(error.to_string()))?;
        let sources = matches
            .into_iter()
            .map(|resolved| Source {
                id: resolved.id,
                provider_id: resolved.provider_id,
                provider_name: resolved.provider_name,
                title: resolved.title,
                description: Some("Resolved from your connected media server".into()),
                filename_hint: None,
                binge_group: None,
                target: SourceTarget::Http(resolved.url),
                quality: resolved.quality,
                size_bytes: resolved.size_bytes,
                language: None,
                codec: None,
                stream_type: Some("media_server".into()),
                seeders: None,
                peers: None,
                headers: resolved.headers,
                subtitles: resolved
                    .subtitles
                    .into_iter()
                    .map(|subtitle| SourceSubtitle {
                        url: subtitle.url,
                        language: subtitle.language,
                        name: subtitle.name,
                        headers: subtitle.headers,
                    })
                    .collect(),
                badges: vec![SourceBadge {
                    label: "MEDIA SERVER".into(),
                    tone: BadgeTone::Positive,
                }],
                cache_state: CacheState::Unknown,
            })
            .collect();
        Ok(AdapterOutput { sources })
    }
}

#[derive(Clone)]
struct CacheEntry {
    stored_at: Instant,
    result: SourceDiscoveryResult,
}

#[derive(Clone)]
pub struct SourceAggregator {
    adapters: Arc<Vec<Arc<dyn SourceAdapter>>>,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    semaphore: Arc<Semaphore>,
    debrid: Option<DebridManager>,
}

impl SourceAggregator {
    pub fn new(adapters: Vec<Arc<dyn SourceAdapter>>) -> Self {
        Self {
            adapters: Arc::new(adapters),
            cache: Arc::new(RwLock::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_PROVIDERS)),
            debrid: None,
        }
    }

    pub fn with_debrid(mut self, manager: DebridManager) -> Self {
        self.debrid = Some(manager);
        self
    }

    pub async fn resolve_torrent(
        &self,
        profile_id: &str,
        source: &Source,
        provider: DebridProviderId,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> Result<ResolvedDebridSource, SourceError> {
        let manager = self
            .debrid
            .as_ref()
            .ok_or_else(|| SourceError::Provider("debrid services are not configured".into()))?;
        let (magnet, file_index) = match &source.target {
            SourceTarget::Torrent {
                info_hash,
                magnet_uri,
                file_index,
            } => (
                magnet_uri
                    .clone()
                    .unwrap_or_else(|| format!("magnet:?xt=urn:btih:{info_hash}")),
                *file_index,
            ),
            _ => {
                return Err(SourceError::InvalidRequest(
                    "selected source is not a torrent".into(),
                ));
            }
        };
        let file_hint = DebridFileHint {
            filename: source.filename_hint.clone(),
            file_index,
            season,
            episode,
        };
        manager
            .resolve_magnet(profile_id, provider, &magnet, &file_hint)
            .await
            .map_err(|error| SourceError::Provider(error.to_string()))
    }

    pub async fn check_torrent_cache(
        &self,
        profile_id: &str,
        provider: DebridProviderId,
        source: &Source,
    ) -> Result<senzo_debrid::CacheAvailability, SourceError> {
        let manager = self
            .debrid
            .as_ref()
            .ok_or_else(|| SourceError::Provider("debrid services are not configured".into()))?;
        let SourceTarget::Torrent { info_hash, .. } = &source.target else {
            return Err(SourceError::InvalidRequest(
                "selected source is not a torrent".into(),
            ));
        };
        manager
            .check_cached(profile_id, provider, info_hash)
            .await
            .map_err(|error| SourceError::Provider(error.to_string()))
    }

    pub async fn discover(
        &self,
        request: SourceRequest,
        force_refresh: bool,
    ) -> Result<SourceDiscoveryResult, SourceError> {
        request.validate()?;
        let key = cache_key(&request);
        if !force_refresh
            && let Some(entry) = self.cache.read().await.get(&key)
            && entry.stored_at.elapsed() <= CACHE_TTL
        {
            return Ok(mark_cached(entry.result.clone()));
        }
        let mut jobs = JoinSet::new();
        for adapter in self.adapters.iter().take(MAX_PROVIDERS) {
            let adapter = adapter.clone();
            let request = request.clone();
            let semaphore = self.semaphore.clone();
            jobs.spawn(async move {
                let permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|_| SourceError::Cancelled)?;
                let started = Instant::now();
                let first = timeout(PROVIDER_TIMEOUT, adapter.discover(&request)).await;
                let result = match first {
                    Ok(Ok(output)) => Ok(output),
                    Ok(Err(_)) | Err(_) => {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        timeout(PROVIDER_TIMEOUT, adapter.discover(&request))
                            .await
                            .map_err(|_| SourceError::Timeout)?
                    }
                };
                drop(permit);
                Ok::<_, SourceError>((
                    adapter.id().to_owned(),
                    adapter.name().to_owned(),
                    started.elapsed(),
                    result,
                ))
            });
        }
        let mut streams = Vec::new();
        let mut diagnostics = Vec::new();
        while let Some(joined) = jobs.join_next().await {
            match joined {
                Ok(Ok((id, name, duration, Ok(output)))) => {
                    let returned = output.sources.len();
                    let validated: Vec<Source> = output
                        .sources
                        .into_iter()
                        .filter(validate_source)
                        .take(MAX_STREAMS_PER_PROVIDER.saturating_mul(16))
                        .map(enrich_badges)
                        .collect();
                    streams.extend(validated);
                    diagnostics.push(ProviderDiagnostic {
                        provider_id: id,
                        provider_name: name,
                        duration_ms: duration.as_millis() as u64,
                        returned_sources: returned,
                        error: None,
                        from_cache: false,
                    });
                }
                Ok(Ok((id, name, duration, Err(error)))) => {
                    diagnostics.push(ProviderDiagnostic {
                        provider_id: id.clone(),
                        provider_name: name.clone(),
                        duration_ms: duration.as_millis() as u64,
                        returned_sources: 0,
                        error: Some(error.to_string()),
                        from_cache: false,
                    });
                    tracing::warn!(provider_id=%id, error=%error, "Source provider isolated after failure");
                }
                Ok(Err(error)) => tracing::warn!(error=%error, "Source provider task failed"),
                Err(error) => tracing::warn!(error=%error, "Source provider task panicked"),
            }
        }
        streams.sort_by(source_order);
        streams.dedup_by(|a, b| a.target == b.target && a.headers == b.headers);
        let groups = group_sources(&streams, &diagnostics);
        let recommended_source_id =
            select_source(&streams, AutoSelectPolicy::Recommended).map(|s| s.id.clone());
        let result = SourceDiscoveryResult {
            groups,
            diagnostics: diagnostics.clone(),
            total_sources: streams.len(),
            used_stale_cache: false,
            recommended_source_id,
        };
        if streams.is_empty()
            && !diagnostics.is_empty()
            && diagnostics
                .iter()
                .all(|diagnostic| diagnostic.error.is_some())
            && let Some(stale) = self
                .cache
                .read()
                .await
                .get(&key)
                .filter(|entry| entry.stored_at.elapsed() <= STALE_TTL)
        {
            let mut fallback = stale.result.clone();
            fallback.used_stale_cache = true;
            return Ok(fallback);
        }
        let mut cache = self.cache.write().await;
        if cache.len() >= CACHE_CAPACITY
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.stored_at)
                .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            key,
            CacheEntry {
                stored_at: Instant::now(),
                result: result.clone(),
            },
        );
        Ok(result)
    }

    pub async fn retry(
        &self,
        request: SourceRequest,
    ) -> Result<SourceDiscoveryResult, SourceError> {
        self.discover(request, true).await
    }

    pub async fn clear_cache(&self) {
        self.cache.write().await.clear();
    }

    pub async fn cached_fallback(&self, request: &SourceRequest) -> Option<SourceDiscoveryResult> {
        self.cache
            .read()
            .await
            .get(&cache_key(request))
            .filter(|entry| entry.stored_at.elapsed() <= STALE_TTL)
            .map(|entry| {
                let mut result = entry.result.clone();
                result.used_stale_cache = true;
                result
            })
    }
}

fn mark_cached(mut result: SourceDiscoveryResult) -> SourceDiscoveryResult {
    for diagnostic in &mut result.diagnostics {
        diagnostic.from_cache = true;
    }
    result
}

fn cache_key(request: &SourceRequest) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        request.media_type,
        request.media_id,
        request.tmdb_id.as_deref().unwrap_or(""),
        request.imdb_id.as_deref().unwrap_or(""),
        request.season.unwrap_or(0),
        request.episode.unwrap_or(0),
        request.title.to_lowercase()
    )
}

pub fn group_sources(sources: &[Source], diagnostics: &[ProviderDiagnostic]) -> Vec<SourceGroup> {
    let mut groups: BTreeMap<(String, String), Vec<Source>> = BTreeMap::new();
    for source in sources {
        groups
            .entry((source.provider_id.clone(), source.provider_name.clone()))
            .or_default()
            .push(source.clone());
    }
    for diagnostic in diagnostics {
        if diagnostic.error.is_some() {
            groups
                .entry((
                    diagnostic.provider_id.clone(),
                    diagnostic.provider_name.clone(),
                ))
                .or_default();
        }
    }
    groups
        .into_iter()
        .map(|((provider_id, provider_name), mut sources)| {
            sources.sort_by(source_order);
            let error = diagnostics
                .iter()
                .find(|d| d.provider_id == provider_id)
                .and_then(|d| d.error.clone());
            SourceGroup {
                provider_id,
                provider_name,
                loading: false,
                error,
                sources,
            }
        })
        .collect()
}

pub fn select_source(sources: &[Source], policy: AutoSelectPolicy) -> Option<&Source> {
    if policy == AutoSelectPolicy::Disabled {
        return None;
    }
    sources.iter().max_by(|a, b| {
        let score = |source: &Source| match policy {
            AutoSelectPolicy::Disabled => 0_i64,
            AutoSelectPolicy::HighestQuality => quality_rank(source.quality.as_deref()) as i64,
            AutoSelectPolicy::CachedFirst => {
                i64::from(source.cache_state == CacheState::Cached) * 100
                    + quality_rank(source.quality.as_deref()) as i64
            }
            AutoSelectPolicy::HighestSeeders => source.seeders.unwrap_or(0) as i64,
            AutoSelectPolicy::Recommended => recommendation_score(source),
        };
        score(a).cmp(&score(b))
    })
}

fn recommendation_score(source: &Source) -> i64 {
    quality_rank(source.quality.as_deref()) as i64 * 10
        + i64::from(source.cache_state == CacheState::Cached) * 100
        + i64::from(source.is_direct()) * 20
        + (source.seeders.unwrap_or(0).min(1000) as i64 / 10)
        - i64::from(source.is_external()) * 1000
}

fn source_order(a: &Source, b: &Source) -> std::cmp::Ordering {
    recommendation_score(b).cmp(&recommendation_score(a))
}

pub fn sort_sources(sources: &mut [Source], sort: SourceSort) {
    sources.sort_by(|a, b| match sort {
        SourceSort::Recommended => source_order(a, b),
        SourceSort::Quality => {
            quality_rank(b.quality.as_deref()).cmp(&quality_rank(a.quality.as_deref()))
        }
        SourceSort::Seeders => b.seeders.unwrap_or(0).cmp(&a.seeders.unwrap_or(0)),
        SourceSort::Size => b.size_bytes.unwrap_or(0).cmp(&a.size_bytes.unwrap_or(0)),
    });
}

fn quality_rank(value: Option<&str>) -> u8 {
    let value = value.unwrap_or("").to_ascii_lowercase();
    if value.contains("2160") || value.contains("4k") || value.contains("uhd") {
        4
    } else if value.contains("1440") {
        3
    } else if value.contains("1080") || value.contains("fhd") {
        2
    } else if value.contains("720") || value.contains("hd") {
        1
    } else {
        0
    }
}

fn enrich_badges(mut source: Source) -> Source {
    let quality = source.quality.as_deref().unwrap_or("").to_ascii_lowercase();
    if quality.contains("2160") || quality.contains("4k") {
        source.badges.push(SourceBadge {
            label: "4K".into(),
            tone: BadgeTone::Accent,
        });
    } else if quality.contains("1080") {
        source.badges.push(SourceBadge {
            label: "1080p".into(),
            tone: BadgeTone::Accent,
        });
    } else if quality.contains("720") {
        source.badges.push(SourceBadge {
            label: "720p".into(),
            tone: BadgeTone::Neutral,
        });
    }
    if source.cache_state == CacheState::Cached {
        source.badges.push(SourceBadge {
            label: "CACHED".into(),
            tone: BadgeTone::Positive,
        });
    }
    if source
        .stream_type
        .as_deref()
        .is_some_and(|kind| kind.to_ascii_lowercase().contains("hdr"))
    {
        source.badges.push(SourceBadge {
            label: "HDR".into(),
            tone: BadgeTone::Accent,
        });
    }
    if source.codec.as_deref().is_some_and(|codec| {
        codec.to_ascii_lowercase().contains("hevc") || codec.to_ascii_lowercase().contains("h265")
    }) {
        source.badges.push(SourceBadge {
            label: "HEVC".into(),
            tone: BadgeTone::Neutral,
        });
    }
    source
}

fn addon_stream_to_source(
    provider_id: &str,
    provider_name: &str,
    index: usize,
    stream: AddonStream,
) -> Option<Source> {
    let extra = stream.extra;
    // Stremio behaviorHints is a first-class field, not part of the flattened extras.
    let behavior = stream.behavior_hints.as_ref();
    let headers = parse_headers(extra.get("headers").or_else(|| {
        behavior
            .and_then(|v| v.get("proxyHeaders"))
            .and_then(|v| v.get("request"))
    }));
    let raw = stream.url.as_deref().or(stream.external_url.as_deref());
    let target = if let Some(hash) = stream.info_hash.as_deref().and_then(normalize_hash) {
        SourceTarget::Torrent {
            info_hash: hash,
            magnet_uri: raw.filter(|v| v.starts_with("magnet:")).map(str::to_owned),
            file_index: stream.file_idx,
        }
    } else if let Some(url) = raw.filter(|v| v.starts_with("magnet:")) {
        SourceTarget::Torrent {
            info_hash: magnet_hash(url)?,
            magnet_uri: Some(url.to_owned()),
            file_index: stream.file_idx,
        }
    } else if let Some(url) = stream.url.as_deref().and_then(valid_http_url) {
        SourceTarget::Http(url)
    } else if let Some(url) = stream.external_url.as_deref().and_then(valid_http_url) {
        SourceTarget::External(url)
    } else if let Some(id) = stream.yt_id.as_deref().filter(|v| valid_youtube_id(v)) {
        SourceTarget::Youtube(id.to_owned())
    } else {
        return None;
    };
    let title = stream
        .name
        .or(stream.title)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| format!("Source {}", index + 1));
    let title_lower = title.to_ascii_lowercase();
    let quality = extract_quality(&title);
    let subtitles = parse_subtitles(extra.get("subtitles"));
    let size = extra.get("size").and_then(parse_size_value);
    Some(Source {
        id: format!("{provider_id}:{index}"),
        provider_id: provider_id.to_owned(),
        provider_name: provider_name.to_owned(),
        title,
        description: stream.description,
        filename_hint: behavior
            .and_then(|value| value.get("filename"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        binge_group: behavior
            .and_then(|value| value.get("bingeGroup"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        target,
        quality,
        size_bytes: size,
        language: extra
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_owned),
        codec: ["hevc", "h265", "h264", "av1"]
            .iter()
            .find(|needle| title_lower.contains(**needle))
            .map(|v| (*v).to_owned()),
        stream_type: extra.get("type").and_then(Value::as_str).map(str::to_owned),
        seeders: extra.get("seeders").and_then(value_u32),
        peers: extra.get("peers").and_then(value_u32),
        headers,
        subtitles,
        badges: Vec::new(),
        cache_state: CacheState::Unknown,
    })
}

fn plugin_stream_to_source(
    provider_id: &str,
    provider_name: &str,
    index: usize,
    stream: PluginStream,
) -> Option<Source> {
    let target = match stream.target {
        PluginTarget::Http(url) => SourceTarget::Http(valid_http_url(&url)?),
        PluginTarget::Torrent {
            info_hash,
            file_index,
        } => SourceTarget::Torrent {
            info_hash: normalize_hash(&info_hash)?,
            magnet_uri: None,
            file_index,
        },
        PluginTarget::External(url) => SourceTarget::External(valid_http_url(&url)?),
        PluginTarget::Youtube(id) if valid_youtube_id(&id) => SourceTarget::Youtube(id),
        PluginTarget::Youtube(_) => return None,
    };
    let quality = stream
        .quality
        .clone()
        .or_else(|| extract_quality(&stream.title));
    let subtitles = stream
        .subtitles
        .into_iter()
        .take(MAX_SUBTITLES)
        .filter_map(|sub| {
            Some(SourceSubtitle {
                url: valid_http_url(&sub.url)?,
                language: sub.language,
                name: sub.name,
                headers: parse_header_map(sub.headers),
            })
        })
        .collect();
    Some(Source {
        id: format!("{provider_id}:{index}:{}", stream.id),
        provider_id: provider_id.to_owned(),
        provider_name: provider_name.to_owned(),
        title: stream.title,
        description: stream.name,
        filename_hint: None,
        binge_group: None,
        target,
        quality,
        size_bytes: parse_size_text(stream.size.as_deref()),
        language: stream.language,
        codec: None,
        stream_type: stream.stream_type,
        seeders: stream.seeders,
        peers: stream.peers,
        headers: parse_header_map(stream.headers),
        subtitles,
        badges: Vec::new(),
        cache_state: CacheState::Unknown,
    })
}

fn valid_http_url(raw: &str) -> Option<String> {
    if raw.len() > MAX_URL_LENGTH {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.to_string())
}

fn normalize_hash(raw: &str) -> Option<String> {
    let value = raw.trim().to_ascii_lowercase();
    (value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())).then_some(value)
}

fn magnet_hash(raw: &str) -> Option<String> {
    let query = Url::parse(raw).ok()?;
    query
        .query_pairs()
        .find(|(key, value)| key == "xt" && value.to_ascii_lowercase().starts_with("urn:btih:"))
        .and_then(|(_, value)| normalize_hash(value.trim_start_matches("urn:btih:")))
}

fn valid_youtube_id(id: &str) -> bool {
    (id.len() == 11)
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn parse_headers(value: Option<&Value>) -> BTreeMap<String, String> {
    let Some(object) = value.and_then(Value::as_object) else {
        return BTreeMap::new();
    };
    let mut headers = BTreeMap::new();
    for (name, value) in object.iter().take(MAX_HEADERS) {
        let Some(value) = value.as_str() else {
            continue;
        };
        let name = name.trim();
        if name.is_empty()
            || name.len() > 256
            || value.len() > MAX_HEADER_VALUE_BYTES
            || name
                .bytes()
                .any(|b| !(b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)))
            || value.chars().any(char::is_control)
        {
            continue;
        }
        headers.insert(name.to_owned(), value.to_owned());
    }
    headers
}

fn parse_header_map(values: BTreeMap<String, String>) -> BTreeMap<String, String> {
    parse_headers(Some(&serde_json::to_value(values).unwrap_or(Value::Null)))
}

fn parse_subtitles(value: Option<&Value>) -> Vec<SourceSubtitle> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_SUBTITLES)
        .filter_map(|item| {
            let object = item.as_object()?;
            Some(SourceSubtitle {
                url: valid_http_url(object.get("url")?.as_str()?)?,
                language: object
                    .get("lang")
                    .or_else(|| object.get("language"))
                    .and_then(Value::as_str)
                    .unwrap_or("und")
                    .to_owned(),
                name: object
                    .get("label")
                    .or_else(|| object.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                headers: parse_headers(object.get("headers")),
            })
        })
        .collect()
}

fn extract_quality(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for value in ["2160p", "4k", "1440p", "1080p", "720p", "480p"] {
        if lower.contains(value) {
            return Some(value.to_owned());
        }
    }
    None
}

fn parse_size_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| parse_size_text(Some(text))))
}

fn parse_size_text(value: Option<&str>) -> Option<u64> {
    let value = value?.trim().to_ascii_uppercase();
    let (num, unit) = value.split_once(' ')?;
    let number: f64 = num.parse().ok()?;
    let multiplier = match unit.trim() {
        "B" => 1.0,
        "KB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        "KIB" => 1024.0,
        "MIB" => 1_048_576.0,
        "GIB" => 1_073_741_824.0,
        _ => return None,
    };
    (number.is_finite() && number >= 0.0)
        .then_some((number * multiplier).min(u64::MAX as f64) as u64)
}

fn value_u32(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|v| u32::try_from(v).ok())
}

fn validate_source(source: &Source) -> bool {
    if source.id.is_empty() || source.provider_id.is_empty() || source.title.trim().is_empty() {
        return false;
    }
    if source.headers.len() > MAX_HEADERS || source.subtitles.len() > MAX_SUBTITLES {
        return false;
    }
    match &source.target {
        SourceTarget::Http(url) | SourceTarget::External(url) => valid_http_url(url).is_some(),
        SourceTarget::Torrent { info_hash, .. } => normalize_hash(info_hash).is_some(),
        SourceTarget::Youtube(id) => valid_youtube_id(id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(target: SourceTarget, quality: Option<&str>, seeders: u32) -> Source {
        Source {
            id: "id".into(),
            provider_id: "provider".into(),
            provider_name: "Provider".into(),
            title: "Title".into(),
            description: None,
            filename_hint: None,
            binge_group: None,
            target,
            quality: quality.map(str::to_owned),
            size_bytes: None,
            language: None,
            codec: None,
            stream_type: None,
            seeders: Some(seeders),
            peers: None,
            headers: BTreeMap::new(),
            subtitles: Vec::new(),
            badges: Vec::new(),
            cache_state: CacheState::Unknown,
        }
    }

    #[test]
    fn classifies_only_supported_target_types() {
        assert!(
            addon_stream_to_source(
                "a",
                "A",
                0,
                AddonStream {
                    name: Some("4K".into()),
                    title: None,
                    description: None,
                    url: Some("https://cdn.example/movie.mkv".into()),
                    info_hash: None,
                    file_idx: None,
                    external_url: None,
                    yt_id: None,
                    behavior_hints: None,
                    extra: BTreeMap::new()
                }
            )
            .is_some()
        );
        assert!(valid_youtube_id("dQw4w9WgXcQ"));
        assert!(valid_http_url("file:///etc/passwd").is_none());
    }

    #[test]
    fn stremio_behavior_hints_and_file_index_survive_source_normalization() {
        let stream = AddonStream {
            name: Some("Release name".into()),
            title: None,
            description: None,
            url: None,
            info_hash: Some("0123456789012345678901234567890123456789".into()),
            file_idx: Some(4),
            external_url: None,
            yt_id: None,
            behavior_hints: Some(serde_json::json!({
                "filename": "Show.S02E03.1080p.mkv",
                "bingeGroup": "season-pack",
                "proxyHeaders": { "request": { "Referer": "https://provider.example/" } }
            })),
            extra: BTreeMap::new(),
        };
        let source = addon_stream_to_source("test", "Test", 0, stream).unwrap();
        assert_eq!(
            source.filename_hint.as_deref(),
            Some("Show.S02E03.1080p.mkv")
        );
        assert_eq!(
            source.headers.get("Referer").map(String::as_str),
            Some("https://provider.example/")
        );
        assert_eq!(source.binge_group.as_deref(), Some("season-pack"));
        assert!(matches!(
            source.target,
            SourceTarget::Torrent {
                file_index: Some(4),
                ..
            }
        ));
    }

    #[test]
    fn groups_sources_by_independent_provider() {
        let a = source(
            SourceTarget::Http("https://a.example/a".into()),
            Some("1080p"),
            0,
        );
        let mut b = source(
            SourceTarget::Http("https://b.example/b".into()),
            Some("720p"),
            0,
        );
        b.provider_id = "other".into();
        b.provider_name = "Other".into();
        assert_eq!(group_sources(&[a, b], &[]).len(), 2);
    }

    #[test]
    fn sorting_filters_and_auto_selection_are_deterministic() {
        let low = source(
            SourceTarget::Http("https://a.example/a".into()),
            Some("720p"),
            2,
        );
        let high = source(
            SourceTarget::Http("https://b.example/b".into()),
            Some("2160p"),
            4,
        );
        let mut list = vec![low, high];
        sort_sources(&mut list, SourceSort::Quality);
        assert_eq!(list[0].quality.as_deref(), Some("2160p"));
        assert_eq!(
            select_source(&list, AutoSelectPolicy::HighestQuality)
                .unwrap()
                .quality
                .as_deref(),
            Some("2160p")
        );
        assert_eq!(
            SourceFilters {
                minimum_seeders: Some(3),
                include_direct: true,
                ..SourceFilters::default()
            }
            .apply(&list)
            .len(),
            1
        );
    }

    #[test]
    fn parses_magnet_infohash_and_header_bounds() {
        let magnet = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";
        assert_eq!(
            magnet_hash(magnet).as_deref(),
            Some("0123456789012345678901234567890123456789")
        );
        assert!(parse_headers(Some(&serde_json::json!({"Authorization":"Bearer abc","bad header":"x","X-Large":"x".repeat(9000)}))).contains_key("Authorization"));
        assert!(
            !parse_headers(Some(&serde_json::json!({"bad header":"x"}))).contains_key("bad header")
        );
    }

    #[test]
    fn episode_request_requires_season_and_rejects_movie_coordinates() {
        let base = SourceRequest {
            media_type: "tv".into(),
            media_id: "tmdb:1:1:1".into(),
            tmdb_id: Some("1".into()),
            imdb_id: None,
            title: "Show".into(),
            year: None,
            season: None,
            episode: Some(1),
        };
        assert!(base.validate().is_err());
        assert!(
            SourceRequest {
                media_type: "movie".into(),
                season: Some(1),
                ..base
            }
            .validate()
            .is_err()
        );
    }

    struct StaticAdapter {
        id: &'static str,
        name: &'static str,
        output: Result<AdapterOutput, SourceError>,
    }

    #[async_trait]
    impl SourceAdapter for StaticAdapter {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.name
        }
        async fn discover(&self, _request: &SourceRequest) -> Result<AdapterOutput, SourceError> {
            self.output.clone()
        }
    }

    #[tokio::test]
    async fn aggregator_collects_independent_providers_and_isolates_failures() {
        let direct = source(
            SourceTarget::Http("https://one.example/video.mkv".into()),
            Some("1080p"),
            0,
        );
        let mut torrent = source(
            SourceTarget::Torrent {
                info_hash: "0123456789012345678901234567890123456789".into(),
                magnet_uri: None,
                file_index: Some(0),
            },
            Some("2160p"),
            40,
        );
        torrent.provider_id = "second".into();
        torrent.provider_name = "Second provider".into();
        let aggregator = SourceAggregator::new(vec![
            Arc::new(StaticAdapter {
                id: "first",
                name: "First provider",
                output: Ok(AdapterOutput {
                    sources: vec![direct],
                }),
            }),
            Arc::new(StaticAdapter {
                id: "second",
                name: "Second provider",
                output: Ok(AdapterOutput {
                    sources: vec![torrent],
                }),
            }),
            Arc::new(StaticAdapter {
                id: "offline",
                name: "Offline provider",
                output: Err(SourceError::Provider("offline".into())),
            }),
        ]);
        let result = aggregator
            .discover(
                SourceRequest {
                    media_type: "movie".into(),
                    media_id: "tt0133093".into(),
                    tmdb_id: Some("603".into()),
                    imdb_id: Some("tt0133093".into()),
                    title: "The Matrix".into(),
                    year: Some(1999),
                    season: None,
                    episode: None,
                },
                false,
            )
            .await
            .unwrap();
        assert_eq!(result.total_sources, 2);
        assert_eq!(
            result
                .groups
                .iter()
                .filter(|group| !group.sources.is_empty())
                .count(),
            2
        );
        assert!(result.diagnostics.iter().any(|diagnostic| diagnostic.provider_id == "offline" && diagnostic.error.is_some()));
        assert!(result.recommended_source_id.is_some());
    }

    #[test]
    fn stream_badges_are_metadata_not_guesses() {
        let mut s = source(
            SourceTarget::Http("https://a.example".into()),
            Some("2160p"),
            0,
        );
        s.cache_state = CacheState::Cached;
        let decorated = enrich_badges(s);
        assert!(decorated.badges.iter().any(|b| b.label == "4K"));
        assert!(decorated.badges.iter().any(|b| b.label == "CACHED"));
    }
}
