//! Direct Sources manager and Slint bridge.
//!
//! This module owns the independent provider subsystem. It does not install or
//! read Stremio add-ons, and Torrentio is never imported as a Direct Source.

pub mod archive;
pub mod commons;
pub mod custom;
pub mod discover;
pub mod matching;
pub mod net;
pub mod types;

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Datelike;
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use stremio_core::{
    models::meta_details::MetaDetails, runtime::Runtime, types::resource::MetaItem,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    DirectDiscoveryResult, DirectProviderCard, MainWindow,
    app_model::AppModel,
    playback::{DirectSelection, PlaybackSelections},
};
use core_env::DesktopEnv;

use self::{
    net::{NetError, SafeHttp, display_url},
    types::{
        CustomSourceConfig, CustomSourceKind, DirectRequest, DirectSourcesConfig, DirectStream,
        DiscoveryCandidate, ProviderHealth,
    },
};

const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_LIMIT: usize = 24;
const MAX_PROVIDERS: usize = 4;

static DIRECT: OnceLock<DirectSources> = OnceLock::new();

#[derive(Clone)]
pub struct DirectSources {
    inner: Arc<Inner>,
}

struct Inner {
    client: SafeHttp,
    config: RwLock<DirectSourcesConfig>,
    health: RwLock<HashMap<String, ProviderHealth>>,
    cache: RwLock<HashMap<String, (Instant, Vec<DirectStream>)>>,
    discoveries: RwLock<Vec<DiscoveryCandidate>>,
    last_request: RwLock<Option<DirectRequest>>,
    last_key: RwLock<String>,
    cancel: RwLock<Option<CancellationToken>>,
    generation: AtomicU64,
    discovery_generation: AtomicU64,
    runtime: Arc<Runtime<DesktopEnv, AppModel>>,
    playback: Arc<PlaybackSelections>,
    ui: slint::Weak<MainWindow>,
}

impl DirectSources {
    fn new(
        ui: slint::Weak<MainWindow>,
        runtime: Arc<Runtime<DesktopEnv, AppModel>>,
        playback: Arc<PlaybackSelections>,
        config: DirectSourcesConfig,
    ) -> Self {
        let mut health = HashMap::new();
        for definition in types::builtin_definitions() {
            health.insert(
                definition.id.to_owned(),
                ProviderHealth {
                    status: "Not checked".to_owned(),
                    checked_at: None,
                },
            );
        }
        for source in &config.custom {
            health.insert(
                source.id.clone(),
                ProviderHealth {
                    status: "Not checked".to_owned(),
                    checked_at: None,
                },
            );
        }
        Self {
            inner: Arc::new(Inner {
                client: SafeHttp::new(),
                config: RwLock::new(config),
                health: RwLock::new(health),
                cache: RwLock::new(HashMap::new()),
                discoveries: RwLock::new(Vec::new()),
                last_request: RwLock::new(None),
                last_key: RwLock::new(String::new()),
                cancel: RwLock::new(None),
                generation: AtomicU64::new(0),
                discovery_generation: AtomicU64::new(0),
                runtime,
                playback,
                ui,
            }),
        }
    }

    fn config(&self) -> DirectSourcesConfig {
        self.inner
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn health(&self) -> HashMap<String, ProviderHealth> {
        self.inner
            .health
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn discoveries(&self) -> Vec<DiscoveryCandidate> {
        self.inner
            .discoveries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn project(&self) {
        let config = self.config();
        let health = self.health();
        let discoveries = self.discoveries();
        let ui = self.inner.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                project_page(&ui, &config, &health, &discoveries);
            }
        });
    }

    fn set_status(&self, message: impl Into<String>, error: bool) {
        let message = message.into();
        let ui = self.inner.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_direct_status(message.into());
                ui.set_direct_status_error(error);
                ui.set_direct_busy(false);
            }
        });
    }

    fn set_busy(&self, busy: bool) {
        let ui = self.inner.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_direct_busy(busy);
            }
        });
    }

    fn close_form(&self) {
        let ui = self.inner.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_direct_form_open(false);
                ui.set_direct_form_error("".into());
            }
        });
    }

    fn set_form_error(&self, message: &str) {
        let message = message.to_owned();
        let ui = self.inner.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_direct_form_error(message.into());
            }
        });
    }

    fn save(&self) {
        self.inner
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let config = self.config();
        let mut app = crate::config::load_config();
        if app.direct_sources != config {
            app.direct_sources = config;
            crate::config::save_config(&app);
        }
    }

    fn set_master(&self, enabled: bool) {
        {
            let mut config = self
                .inner
                .config
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if config.enabled == enabled {
                return;
            }
            config.enabled = enabled;
        }
        self.save();
        self.project();
        if enabled {
            self.refresh_last();
        } else {
            self.cancel_current();
            self.clear_direct_playback();
            self.set_status("Direct Sources is off", false);
        }
    }

    fn toggle_provider(&self, id: &str, enabled: bool) {
        let changed = {
            let mut config = self
                .inner
                .config
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if config.set_builtin_enabled(id, enabled) {
                true
            } else if let Some(source) = config.custom.iter_mut().find(|source| source.id == id) {
                source.enabled = enabled;
                true
            } else {
                false
            }
        };
        if !changed {
            self.set_status("That provider no longer exists", true);
            return;
        }
        self.save();
        self.project();
        self.refresh_last();
    }

    fn validate_provider(&self, id: &str) {
        let config = self.config();
        let Some(source) = config.custom_source(id).cloned() else {
            let Some(definition) = types::builtin_definition(id) else {
                self.set_status("That provider no longer exists", true);
                return;
            };
            self.set_provider_status(id, "Checking…");
            self.set_busy(true);
            let manager = self.clone();
            let provider_id = definition.id.to_owned();
            let client = self.inner.client.clone();
            tokio::spawn(async move {
                let result = match provider_id.as_str() {
                    types::INTERNET_ARCHIVE => archive::validate(&client).await,
                    types::WIKIMEDIA_COMMONS => commons::validate(&client).await,
                    _ => Err(NetError::Request),
                };
                manager.finish_validation(&provider_id, result);
            });
            return;
        };

        self.set_provider_status(id, "Checking…");
        self.set_busy(true);
        let manager = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move {
            let result = custom::validate(&manager.inner.client, &source)
                .await
                .map_err(|error| error.to_string());
            manager.finish_custom_validation(&id, result);
        });
    }

    fn finish_validation(&self, id: &str, result: Result<String, NetError>) {
        match result {
            Ok(summary) => {
                self.set_provider_status(id, &format!("Ready · {summary}"));
                self.set_status(format!("{} is reachable", provider_name(id)), false);
            }
            Err(error) => {
                self.set_provider_status(id, &format!("Error · {}", net_error_label(&error)));
                self.set_status(
                    format!("{} could not be validated", provider_name(id)),
                    true,
                );
            }
        }
        self.project();
    }

    fn finish_custom_validation(&self, id: &str, result: Result<String, String>) {
        match result {
            Ok(summary) => {
                self.set_provider_status(id, &format!("Ready · {summary}"));
                self.set_status("Custom source is reachable", false);
            }
            Err(error) => {
                self.set_provider_status(id, &format!("Error · {error}"));
                self.set_status("Custom source could not be validated", true);
            }
        }
        self.project();
    }

    fn set_provider_status(&self, id: &str, status: &str) {
        let mut health = self
            .inner
            .health
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.insert(
            id.to_owned(),
            ProviderHealth {
                status: status.to_owned(),
                checked_at: Some(chrono::Utc::now().timestamp()),
            },
        );
    }

    fn remove_source(&self, id: &str) {
        let removed = {
            let mut config = self
                .inner
                .config
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let before = config.custom.len();
            config.custom.retain(|source| source.id != id);
            before != config.custom.len()
        };
        if !removed {
            self.set_status("That source no longer exists", true);
            return;
        }
        self.inner
            .health
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
        self.save();
        self.project();
        self.set_status("Source removed", false);
        self.refresh_last();
    }

    fn save_source(
        &self,
        id: &str,
        name: &str,
        url: &str,
        kind: &str,
        content_types: &str,
        formats: &str,
        enabled: bool,
    ) {
        let name = name.trim();
        let url = url.trim();
        if name.is_empty() {
            self.set_form_error("Give the source a name");
            self.set_status("Give the source a name", true);
            return;
        }
        if let Err(error) = SafeHttp::validate_url(url) {
            self.set_form_error(net_error_label(&error));
            self.set_status(net_error_label(&error), true);
            return;
        }
        let Some(kind) = CustomSourceKind::from_short(kind) else {
            self.set_form_error("Choose a supported source type");
            self.set_status("Choose a supported source type", true);
            return;
        };
        let content_types = split_labels(content_types, &["movie", "series"]);
        let formats = split_labels(formats, &["m3u8", "mp4"]);
        let id = if id.trim().is_empty() {
            format!("custom-{}", Uuid::new_v4())
        } else {
            id.trim().to_owned()
        };
        let is_new = {
            let mut config = self
                .inner
                .config
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let is_new = !config.custom.iter().any(|source| source.id == id);
            if let Some(source) = config.custom.iter_mut().find(|source| source.id == id) {
                source.name = name.to_owned();
                source.url = url.to_owned();
                source.kind = kind;
                source.content_types = content_types;
                source.formats = formats;
                source.enabled = enabled;
            } else {
                config.custom.push(CustomSourceConfig {
                    id: id.clone(),
                    name: name.to_owned(),
                    url: url.to_owned(),
                    kind,
                    content_types,
                    formats,
                    enabled,
                });
            }
            is_new
        };
        self.save();
        self.project();
        if is_new {
            self.inner
                .health
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    id.clone(),
                    ProviderHealth {
                        status: "Not checked".to_owned(),
                        checked_at: None,
                    },
                );
        }
        self.close_form();
        self.set_status("Source saved", false);
        self.validate_provider(&id);
        self.refresh_last();
    }

    fn open_source_form(&self, id: Option<&str>) {
        let config = self.config();
        let source = id.and_then(|id| config.custom_source(id));
        if id.is_some() && source.is_none() {
            self.set_status("That source no longer exists", true);
            return;
        }
        let ui = self.inner.ui.clone();
        let source = source.cloned();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                if let Some(source) = source {
                    ui.set_direct_form_id(source.id.into());
                    ui.set_direct_form_name(source.name.into());
                    ui.set_direct_form_url(source.url.into());
                    ui.set_direct_form_type(kind_label_for_ui(source.kind).into());
                    ui.set_direct_form_content_types(source.content_types.join(", ").into());
                    ui.set_direct_form_formats(source.formats.join(", ").into());
                    ui.set_direct_form_enabled(source.enabled);
                } else {
                    ui.set_direct_form_id("".into());
                    ui.set_direct_form_name("".into());
                    ui.set_direct_form_url("".into());
                    ui.set_direct_form_type("M3U playlist".into());
                    ui.set_direct_form_content_types("movie, series".into());
                    ui.set_direct_form_formats("m3u8, mp4".into());
                    ui.set_direct_form_enabled(true);
                }
                ui.set_direct_form_error("".into());
                ui.set_direct_form_open(true);
            }
        });
    }

    fn cancel(&self) {
        self.inner
            .discovery_generation
            .fetch_add(1, Ordering::AcqRel);
        self.cancel_current();
        self.set_busy(false);
        self.set_status("Direct source request cancelled", false);
    }

    fn retry(&self) {
        self.refresh_last();
    }

    fn discover(&self, domain: &str) {
        let generation = self
            .inner
            .discovery_generation
            .fetch_add(1, Ordering::AcqRel)
            + 1;
        self.set_busy(true);
        self.set_status("Checking the domain and its robots.txt…", false);
        let manager = self.clone();
        let domain = domain.trim().to_owned();
        tokio::spawn(async move {
            match discover::discover(&manager.inner.client, &domain).await {
                Ok(candidates) => {
                    if manager.inner.discovery_generation.load(Ordering::Acquire) != generation {
                        return;
                    }
                    *manager
                        .inner
                        .discoveries
                        .write()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = candidates;
                    manager.project();
                    manager.set_status(
                        "Discovery finished. Review each result before adding it.",
                        false,
                    );
                }
                Err(error) => {
                    if manager.inner.discovery_generation.load(Ordering::Acquire) != generation {
                        return;
                    }
                    manager.set_status(
                        format!("Discovery failed: {}", net_error_label(&error)),
                        true,
                    );
                }
            }
        });
    }

    fn add_discovery(&self, id: &str) {
        let candidate = self
            .discoveries()
            .into_iter()
            .find(|candidate| candidate.id == id);
        let Some(candidate) = candidate else {
            self.set_status("That discovery result is no longer available", true);
            return;
        };
        let Some(kind) = candidate.kind else {
            self.set_status("That result is not a supported provider format", true);
            return;
        };
        if SafeHttp::validate_url(&candidate.url).is_err() {
            self.set_status("That result does not use an allowed HTTPS URL", true);
            return;
        }
        let id = format!("custom-{}", Uuid::new_v4());
        let source = CustomSourceConfig {
            id: id.clone(),
            name: candidate.name,
            url: candidate.url,
            kind,
            content_types: candidate
                .content_types
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            formats: candidate
                .formats
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            enabled: true,
        };
        {
            let mut config = self
                .inner
                .config
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            config.custom.push(source);
        }
        self.inner
            .health
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                id.clone(),
                ProviderHealth {
                    status: "Not checked".to_owned(),
                    checked_at: None,
                },
            );
        self.save();
        self.project();
        self.set_status("Source added. Validating it now.", false);
        self.validate_provider(&id);
    }

    fn export(&self) {
        let bundle = ExportBundle {
            format: "senzo-direct-sources".to_owned(),
            version: 1,
            custom_sources: self.config().custom,
        };
        match serde_json::to_string_pretty(&bundle).ok().and_then(|json| {
            arboard::Clipboard::new()
                .ok()
                .and_then(|mut clipboard| clipboard.set_text(json).ok())
        }) {
            Some(()) => self.set_status(
                "Custom sources copied to the clipboard. Treat the export as private.",
                false,
            ),
            None => self.set_status("Could not write the export to the clipboard", true),
        }
    }

    fn import(&self) {
        let Some(text) = arboard::Clipboard::new()
            .ok()
            .and_then(|mut clipboard| clipboard.get_text().ok())
        else {
            self.set_status("Could not read the clipboard", true);
            return;
        };
        let bundle = serde_json::from_str::<ExportBundle>(&text).or_else(|_| {
            serde_json::from_str::<Vec<CustomSourceConfig>>(&text).map(|custom_sources| {
                ExportBundle {
                    format: "senzo-direct-sources".to_owned(),
                    version: 1,
                    custom_sources,
                }
            })
        });
        let Ok(bundle) = bundle else {
            self.set_status(
                "Clipboard does not contain a SENZO Direct Sources export",
                true,
            );
            return;
        };
        if bundle.format != "senzo-direct-sources" || bundle.version != 1 {
            self.set_status("That export version is not supported", true);
            return;
        }
        let mut added = 0usize;
        let mut skipped = 0usize;
        {
            let mut config = self
                .inner
                .config
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for mut source in bundle.custom_sources {
                source.name = source.name.trim().to_owned();
                source.url = source.url.trim().to_owned();
                if source.name.is_empty() || SafeHttp::validate_url(&source.url).is_err() {
                    skipped += 1;
                    continue;
                }
                if source.id.trim().is_empty()
                    || config
                        .custom
                        .iter()
                        .any(|existing| existing.id == source.id)
                {
                    source.id = format!("custom-{}", Uuid::new_v4());
                }
                config.custom.push(source);
                added += 1;
            }
        }
        self.save();
        self.project();
        self.set_status(
            format!("Imported {added} source(s), skipped {skipped}"),
            added == 0,
        );
    }

    fn refresh_last(&self) {
        let request = self
            .inner
            .last_request
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(request) = request {
            self.start_query(request);
        }
    }

    fn cancel_current(&self) {
        if let Some(token) = self
            .inner
            .cancel
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            token.cancel();
        }
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
    }

    fn clear_direct_playback(&self) {
        self.inner.playback.clear_direct_results();
        let playback = self.inner.playback.clone();
        let runtime = self.inner.runtime.clone();
        let ui = self.inner.ui.clone();
        let views = runtime
            .model()
            .ok()
            .map(|model| playback.rebuild(&model.meta_details, &model.ctx.profile.addons))
            .unwrap_or_default();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                let weak = ui.as_weak();
                crate::event_loop::project_stream_selection_views(&ui, &weak, views);
            }
        });
    }

    fn on_details(&self, details: &MetaDetails) {
        let Some(request) = request_from_details(details) else {
            return;
        };
        let key = request.cache_key();
        let same = self
            .inner
            .last_key
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_str()
            == key;
        *self
            .inner
            .last_key
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = key;
        *self
            .inner
            .last_request
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.clone());
        if !self.config().enabled {
            self.inner.playback.clear_direct_results();
            return;
        }
        if !same {
            // The details event loop rebuilds the stream list after this hook,
            // so clearing here removes stale results without re-entering the
            // runtime model lock while it is held by the caller.
            self.inner.playback.clear_direct_results();
            self.start_query(request);
        }
    }

    fn start_query(&self, request: DirectRequest) {
        self.cancel_current();
        let token = CancellationToken::new();
        *self
            .inner
            .cancel
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(token.clone());
        let generation = self.inner.generation.load(Ordering::Acquire);
        let manager = self.clone();
        tokio::spawn(async move {
            let streams = manager.query(&request, token.clone()).await;
            if token.is_cancelled()
                || manager.inner.generation.load(Ordering::Acquire) != generation
            {
                return;
            }
            manager.apply_results(&request, streams);
        });
    }

    async fn query(&self, request: &DirectRequest, token: CancellationToken) -> Vec<DirectStream> {
        let config = self.config();
        if !config.enabled {
            return Vec::new();
        }
        let cache_key = request.cache_key();
        let cached = {
            self.inner
                .cache
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&cache_key)
                .filter(|(at, _)| at.elapsed() < CACHE_TTL)
                .map(|(_, streams)| streams.clone())
        };
        if let Some(streams) = cached {
            return streams;
        }

        let providers = self.providers(&config);
        if providers.is_empty() {
            return Vec::new();
        }
        let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_PROVIDERS));
        let mut tasks = Vec::new();
        for provider in providers {
            let client = self.inner.client.clone();
            let request = request.clone();
            let token = token.clone();
            let semaphore = semaphore.clone();
            tasks.push(tokio::spawn(async move {
                let _permit = semaphore.acquire_owned().await.ok()?;
                let result = tokio::select! {
                    _ = token.cancelled() => return None,
                    result = provider.resolve(&client, &request) => result,
                };
                Some((provider.id().to_owned(), result))
            }));
        }

        let mut streams = Vec::new();
        for task in tasks {
            let Some((id, result)) = task.await.ok().flatten() else {
                continue;
            };
            match result {
                Ok(mut found) => {
                    self.set_provider_status(&id, &format!("Ready · {} result(s)", found.len()));
                    streams.append(&mut found);
                }
                Err(NetError::Timeout) => {
                    self.set_provider_status(&id, "Error · timed out");
                    tracing::warn!(provider = %id, "direct sources provider timed out");
                }
                Err(error) => {
                    self.set_provider_status(&id, &format!("Error · {}", net_error_label(&error)));
                    tracing::warn!(provider = %id, error = %net_error_label(&error), "direct sources provider failed");
                }
            }
        }
        streams.sort_by(|left, right| {
            rank_quality(&right.quality)
                .cmp(&rank_quality(&left.quality))
                .then_with(|| left.provider_name.cmp(&right.provider_name))
        });
        streams.dedup_by(|left, right| left.url == right.url);
        streams.truncate(48);
        self.inner
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(cache_key, (Instant::now(), streams.clone()));
        let mut cache = self
            .inner
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.len() > CACHE_LIMIT {
            let oldest = cache
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                cache.remove(&oldest);
            }
        }
        streams
    }

    fn providers(&self, config: &DirectSourcesConfig) -> Vec<Provider> {
        let mut providers = Vec::new();
        for definition in types::builtin_definitions() {
            if !config.builtin_enabled(definition.id) {
                continue;
            }
            match definition.id {
                types::INTERNET_ARCHIVE => providers.push(Provider::Archive),
                types::WIKIMEDIA_COMMONS => providers.push(Provider::Commons),
                _ => {}
            }
        }
        for source in config.enabled_custom_sources() {
            providers.push(Provider::Custom(source.clone()));
        }
        providers
    }

    fn apply_results(&self, request: &DirectRequest, streams: Vec<DirectStream>) {
        let selections = streams
            .into_iter()
            .enumerate()
            .map(|(index, stream)| {
                let description = direct_description(&stream);
                DirectSelection {
                    id: format!("direct:{}:{index}", stream.provider_id),
                    provider_name: stream.provider_name.clone(),
                    title: stream.title,
                    description,
                    media_type: request.media_type.clone(),
                    video_id: request.video_id.clone(),
                    url: stream.url,
                    headers: stream.headers,
                    subtitles: stream.subtitles,
                }
            })
            .collect::<Vec<_>>();
        self.inner.playback.set_direct_results(selections);
        let playback = self.inner.playback.clone();
        let runtime = self.inner.runtime.clone();
        let ui = self.inner.ui.clone();
        let views = runtime
            .model()
            .ok()
            .map(|model| playback.rebuild(&model.meta_details, &model.ctx.profile.addons))
            .unwrap_or_default();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                let weak = ui.as_weak();
                crate::event_loop::project_stream_selection_views(&ui, &weak, views);
            }
        });
        self.project();
    }
}

#[derive(Clone)]
enum Provider {
    Archive,
    Commons,
    Custom(CustomSourceConfig),
}

impl Provider {
    fn id(&self) -> &str {
        match self {
            Self::Archive => types::INTERNET_ARCHIVE,
            Self::Commons => types::WIKIMEDIA_COMMONS,
            Self::Custom(source) => &source.id,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Archive => "Internet Archive",
            Self::Commons => "Wikimedia Commons",
            Self::Custom(source) => &source.name,
        }
    }

    async fn resolve(
        &self,
        client: &SafeHttp,
        request: &DirectRequest,
    ) -> Result<Vec<DirectStream>, NetError> {
        match self {
            Self::Archive => archive::resolve(client, request).await,
            Self::Commons => commons::resolve(client, request).await,
            Self::Custom(source) => custom::resolve(client, request, source).await,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct ExportBundle {
    format: String,
    version: u32,
    custom_sources: Vec<CustomSourceConfig>,
}

pub fn setup(
    ui: &MainWindow,
    runtime: Arc<Runtime<DesktopEnv, AppModel>>,
    playback: Arc<PlaybackSelections>,
    config: &DirectSourcesConfig,
) {
    let mut config = config.clone();
    config.ensure_builtin_defaults();
    let manager = DirectSources::new(ui.as_weak(), runtime, playback, config);
    let _ = DIRECT.set(manager.clone());
    project_page(
        ui,
        &manager.config(),
        &manager.health(),
        &manager.discoveries(),
    );
    ui.set_direct_master_enabled(manager.config().enabled);

    ui.on_direct_master_changed({
        let manager = manager.clone();
        move |enabled| manager.set_master(enabled)
    });
    ui.on_direct_provider_toggled({
        let manager = manager.clone();
        move |id, enabled| manager.toggle_provider(id.as_str(), enabled)
    });
    ui.on_direct_provider_validate({
        let manager = manager.clone();
        move |id| manager.validate_provider(id.as_str())
    });
    ui.on_direct_source_add({
        let manager = manager.clone();
        move || manager.open_source_form(None)
    });
    ui.on_direct_source_edit({
        let manager = manager.clone();
        move |id| manager.open_source_form(Some(id.as_str()))
    });
    ui.on_direct_source_remove({
        let manager = manager.clone();
        move |id| manager.remove_source(id.as_str())
    });
    ui.on_direct_source_save({
        let manager = manager.clone();
        move |id, name, url, kind, content_types, formats, enabled| {
            manager.save_source(
                id.as_str(),
                name.as_str(),
                url.as_str(),
                kind.as_str(),
                content_types.as_str(),
                formats.as_str(),
                enabled,
            );
        }
    });
    ui.on_direct_source_cancel({
        let manager = manager.clone();
        move || {
            let ui = manager.inner.ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui.upgrade() {
                    ui.set_direct_form_open(false);
                }
            });
        }
    });
    ui.on_direct_discover({
        let manager = manager.clone();
        move |domain| manager.discover(domain.as_str())
    });
    ui.on_direct_discovery_add({
        let manager = manager.clone();
        move |id| manager.add_discovery(id.as_str())
    });
    ui.on_direct_import({
        let manager = manager.clone();
        move || manager.import()
    });
    ui.on_direct_export({
        let manager = manager.clone();
        move || manager.export()
    });
    ui.on_direct_cancel({
        let manager = manager.clone();
        move || manager.cancel()
    });
    ui.on_direct_retry({
        let manager = manager.clone();
        move || manager.retry()
    });
}

pub fn on_details_synced(details: &MetaDetails) {
    if let Some(manager) = DIRECT.get() {
        manager.on_details(details);
    }
}

fn project_page(
    ui: &MainWindow,
    config: &DirectSourcesConfig,
    health: &HashMap<String, ProviderHealth>,
    discoveries: &[DiscoveryCandidate],
) {
    let builtin = types::builtin_definitions()
        .iter()
        .map(|definition| {
            let enabled = config.builtin_enabled(definition.id);
            DirectProviderCard {
                id: definition.id.into(),
                name: definition.name.into(),
                description: definition.description.into(),
                content_types: definition.content_types.join(", ").into(),
                languages: definition.languages.join(", ").into(),
                formats: definition.formats.join(", ").into(),
                status: health
                    .get(definition.id)
                    .map(|health| health.status.clone())
                    .unwrap_or_else(|| "Not checked".to_owned())
                    .into(),
                status_error: health
                    .get(definition.id)
                    .is_some_and(|health| health.status.starts_with("Error")),
                enabled,
                builtin: true,
                configurable: definition.configurable,
            }
        })
        .collect::<Vec<_>>();
    let custom = config
        .custom
        .iter()
        .map(|source| DirectProviderCard {
            id: source.id.clone().into(),
            name: source.name.clone().into(),
            description: format!("{} · {}", source.kind.label(), display_url(&source.url)).into(),
            content_types: source.content_types.join(", ").into(),
            languages: "per source".to_owned().into(),
            formats: source.formats.join(", ").into(),
            status: health
                .get(&source.id)
                .map(|health| health.status.clone())
                .unwrap_or_else(|| "Not checked".to_owned())
                .into(),
            status_error: health
                .get(&source.id)
                .is_some_and(|health| health.status.starts_with("Error")),
            enabled: source.enabled,
            builtin: false,
            configurable: true,
        })
        .collect::<Vec<_>>();
    let discoveries = discoveries
        .iter()
        .map(|candidate| DirectDiscoveryResult {
            id: candidate.id.clone().into(),
            name: candidate.name.clone().into(),
            domain: candidate.domain.clone().into(),
            method: candidate.method.clone().into(),
            content_types: candidate.content_types.clone().into(),
            formats: candidate.formats.clone().into(),
            status: candidate.status.clone().into(),
            required_config: candidate.required_config.clone().into(),
            can_add: candidate.kind.is_some(),
        })
        .collect::<Vec<_>>();

    ui.set_direct_builtin_providers(slint::ModelRc::new(slint::VecModel::from(builtin)));
    ui.set_direct_custom_providers(slint::ModelRc::new(slint::VecModel::from(custom)));
    ui.set_direct_discoveries(slint::ModelRc::new(slint::VecModel::from(discoveries)));
    ui.set_direct_master_enabled(config.enabled);
}

fn request_from_details(details: &MetaDetails) -> Option<DirectRequest> {
    let selected = details.selected.as_ref()?;
    let meta_item = details.meta_items.iter().find_map(|resource| {
        let stremio_core::models::common::Loadable::Ready(meta_item) = resource.content.as_ref()?
        else {
            return None;
        };
        resource
            .request
            .path
            .eq_no_extra(&selected.meta_path)
            .then_some(meta_item)
    })?;
    let media_type = selected.meta_path.r#type.clone();
    let meta_id = selected.meta_path.id.clone();
    let video_id = selected
        .stream_path
        .as_ref()
        .map(|path| path.id.clone())
        .or_else(|| (media_type == "movie").then(|| meta_id.clone()));
    let video = video_id
        .as_deref()
        .and_then(|video_id| meta_item.videos.iter().find(|video| video.id == video_id));
    let imdb_id = imdb_id_for(&meta_item.preview, &meta_id);
    Some(DirectRequest {
        imdb_id,
        meta_id: meta_id.clone(),
        media_type: media_type.clone(),
        title: meta_item.preview.name.clone(),
        year: release_year(meta_item),
        season: video.and_then(|video| {
            video
                .series_info
                .as_ref()
                .and_then(|info| i32::try_from(info.season).ok())
        }),
        episode: video.and_then(|video| {
            video
                .series_info
                .as_ref()
                .and_then(|info| i32::try_from(info.episode).ok())
        }),
        episode_title: video.map(|video| video.title.clone()),
        video_id,
    })
}

fn imdb_id_for(
    preview: &stremio_core::types::resource::MetaItemPreview,
    meta_id: &str,
) -> Option<String> {
    if is_imdb_id(meta_id) {
        return Some(meta_id.to_owned());
    }
    preview.links.iter().find_map(|link| {
        if !link.category.eq_ignore_ascii_case("imdb") {
            return None;
        }
        let path = link.url.path();
        path.split('/')
            .find(|segment| is_imdb_id(segment))
            .map(ToOwned::to_owned)
    })
}

fn is_imdb_id(value: &str) -> bool {
    value.strip_prefix("tt").is_some_and(|rest| {
        (7..=10).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_digit())
    })
}

fn release_year(meta_item: &MetaItem) -> Option<i32> {
    if let Some(released) = meta_item.preview.released {
        return Some(released.year());
    }
    meta_item
        .preview
        .release_info
        .as_deref()
        .and_then(matching::parse_year)
}

fn direct_description(stream: &DirectStream) -> String {
    let mut parts = vec![
        "Direct Sources".to_owned(),
        stream.provider_name.clone(),
        stream.quality.clone(),
        stream.language.clone(),
        stream.video_format.clone(),
    ];
    if !stream.audio.is_empty() && stream.audio != "unknown" {
        parts.push(stream.audio.clone());
    }
    parts.join(" · ")
}

fn rank_quality(quality: &str) -> u8 {
    match quality {
        "4K" => 5,
        "1440p" => 4,
        "1080p" => 3,
        "720p" => 2,
        "576p" => 1,
        _ => 0,
    }
}

fn split_labels(value: &str, defaults: &[&str]) -> Vec<String> {
    let labels = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if labels.is_empty() {
        defaults.iter().map(|value| (*value).to_owned()).collect()
    } else {
        labels
    }
}

fn kind_label_for_ui(kind: CustomSourceKind) -> &'static str {
    match kind {
        CustomSourceKind::M3u => "M3U playlist",
        CustomSourceKind::JsonFeed => "JSON feed",
    }
}

fn net_error_label(error: &NetError) -> &'static str {
    match error {
        NetError::NotHttps => "Only HTTPS sources are allowed",
        NetError::InvalidUrl => "The URL is not valid",
        NetError::PrivateAddress => "Private and loopback addresses are blocked",
        NetError::Timeout => "The source timed out",
        NetError::Status(404) => "The source was not found (HTTP 404)",
        NetError::Status(403) => "The source denied access (HTTP 403)",
        NetError::Status(_) => "The source returned an error",
        NetError::TooLarge => "The source response is too large",
        NetError::ContentType => "The source format is not supported",
        NetError::Request => "The source could not be reached",
        NetError::UnsafeRedirect => "The source redirected to an unsafe address",
    }
}

fn provider_name(id: &str) -> &str {
    types::builtin_definition(id)
        .map(|definition| definition.name)
        .unwrap_or("This provider")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_only_vetted_builtins() {
        let config = DirectSourcesConfig::default();
        assert!(config.enabled);
        assert!(config.builtin_enabled(types::INTERNET_ARCHIVE));
        assert!(config.builtin_enabled(types::WIKIMEDIA_COMMONS));
        assert!(config.custom.is_empty());
    }

    #[test]
    fn migration_does_not_reenable_a_disabled_provider() {
        let mut config = DirectSourcesConfig {
            enabled: true,
            builtin: vec![BuiltinSourceState {
                id: types::INTERNET_ARCHIVE.to_owned(),
                enabled: false,
            }],
            custom: Vec::new(),
        };
        config.ensure_builtin_defaults();
        assert!(!config.builtin_enabled(types::INTERNET_ARCHIVE));
        assert!(config.builtin_enabled(types::WIKIMEDIA_COMMONS));
    }

    #[test]
    fn export_bundle_has_no_builtin_state_or_secret_fields() {
        let bundle = ExportBundle {
            format: "senzo-direct-sources".to_owned(),
            version: 1,
            custom_sources: vec![CustomSourceConfig {
                id: "custom-1".to_owned(),
                name: "Authorized feed".to_owned(),
                url: "https://example.com/feed.json".to_owned(),
                kind: CustomSourceKind::JsonFeed,
                content_types: vec!["movie".to_owned()],
                formats: vec!["mp4".to_owned()],
                enabled: true,
            }],
        };
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(!json.contains("builtin"));
        assert!(!json.contains("api_key"));
    }

    #[test]
    fn direct_descriptions_identify_the_source_system() {
        let stream = DirectStream::new(
            "internet-archive",
            "Internet Archive",
            &DirectRequest {
                imdb_id: None,
                meta_id: "tt1".to_owned(),
                media_type: "movie".to_owned(),
                title: "Test".to_owned(),
                year: None,
                season: None,
                episode: None,
                episode_title: None,
                video_id: None,
            },
            "https://example.com/a.mp4".to_owned(),
            "mp4",
        );
        assert!(direct_description(&stream).starts_with("Direct Sources"));
    }
}
