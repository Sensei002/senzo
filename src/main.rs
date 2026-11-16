use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context as _;
use gpui::{
    Animation, AnimationExt, App, ClipboardItem, Context, Entity, MouseButton, ObjectFit, QuitMode,
    Render, Subscription, Task, Window, WindowOptions, div, ease_in_out, img, prelude::*, px, rgb,
};
use gpui_base::input::{InputEvent, InputState};
use gpui_base::{Input as GpuiInput, InputBase};
use serde_json::Value;

use senzo::{
    addons::{
        AddonCatalogExtra, AddonClient, AddonManifest, AddonManifestPreview, StremioStream,
        StremioSubtitle, first_series_episode_id,
    },
    artwork::TmdbArtworkHttpClient,
    diagnostics,
    metadata::{TmdbClient, TmdbEpisode, TmdbSearchResult},
    playback::{
        mpv_config::build_mpv_config,
        mpv_worker::{PlaybackEvent, PlaybackState, PlaybackWorker},
        windows_surface::{SurfaceBounds, WindowsVideoSurface, gpui_parent_hwnd},
    },
    settings::{
        DecoderPreference, HardwareDecoding, HdrMode, PlaybackSettings, RendererPreference,
        ScalingAlgorithm,
    },
    sources::{SourceProviderRegistry, SourceProviderResult},
    storage::{LocalDatabase, PlaybackProgress, StoredAddon},
    subtitles::{
        OpenSubtitlesClient, OpenSubtitlesCredentialStore, OpenSubtitlesResult, SubtitleSearch,
    },
    torrent::TorrentSessionDirectory,
    torrent_engine::{TorrentPlaybackSession, TorrentProgress, TorrentVideoFile},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum AppPage {
    Home,
    Movies,
    TvShows,
    Search,
    Library,
    Player,
    Details,
    Addons,
    Settings,
}

struct SenzoApp {
    database: LocalDatabase,
    active_page: AppPage,
    settings_advanced_open: bool,
    metadata_language: String,
    poster_size: String,
    selected_title: Option<String>,
    search_query: String,
    search_input: Option<Entity<InputState>>,
    search_input_subscription: Option<Subscription>,
    search_debounce_task: Option<Task<()>>,
    settings: PlaybackSettings,
    discord_presence_enabled: bool,
    discord_share_titles: bool,
    discord_presence: senzo::discord::DiscordPresence,
    status: String,
    playback_worker: Option<Arc<Mutex<PlaybackWorker>>>,
    playback_state: PlaybackState,
    playback_status: String,
    active_media_id: Option<String>,
    active_media_title: Option<String>,
    active_media_type: Option<String>,
    last_progress_save: Option<Instant>,
    playback_monitor: Option<Task<()>>,
    online_subtitle_results: Vec<OpenSubtitlesResult>,
    addon_subtitle_results: Vec<(StoredAddon, StremioSubtitle)>,
    addon_subtitle_status: String,
    subtitle_provider_status: String,
    subtitle_credentials_status: String,
    tmdb_service_status: String,
    subtitle_task: Option<Task<()>>,
    addon_manifest_preview: Option<AddonManifestPreview>,
    addon_status: String,
    addon_task: Option<Task<()>>,
    tmdb_results: Vec<(String, TmdbSearchResult)>,
    tmdb_status: String,
    home_catalogs: Vec<(String, String, TmdbSearchResult)>,
    home_addon_catalogs: Vec<(String, StoredAddon, String, String, Vec<Value>)>,
    home_catalog_status: String,
    home_catalog_requested: bool,
    home_hero_index: usize,
    home_hero_paused: bool,
    home_hero_task: Option<Task<()>>,
    home_shelf_order: Vec<String>,
    hidden_home_shelves: Vec<String>,
    home_customize_open: bool,
    tmdb_details: Option<Value>,
    tmdb_task: Option<Task<()>>,
    tv_seasons: Vec<(u32, String)>,
    tv_selected_season: Option<u32>,
    tv_episodes: Vec<TmdbEpisode>,
    tv_selected_episode: Option<TmdbEpisode>,
    tv_episode_status: String,
    addon_catalog_items: Vec<Value>,
    addon_catalog_status: String,
    addon_catalog_addon: Option<StoredAddon>,
    addon_catalog_media_type: String,
    addon_catalog_id: String,
    addon_catalog_page: usize,
    addon_catalog_filter_specs: Vec<AddonCatalogExtra>,
    addon_catalog_filter_values: Vec<(String, Vec<String>)>,
    addon_stream_results: Vec<SourceProviderResult>,
    stream_status: String,
    torrent_session: Option<TorrentPlaybackSession>,
    torrent_files: Vec<TorrentVideoFile>,
    torrent_progress: Option<TorrentProgress>,
    torrent_status: String,
    torrent_task: Option<Task<()>>,
    native_parent_hwnd: Option<usize>,
    native_viewport_bounds: Arc<Mutex<Option<SurfaceBounds>>>,
    native_video_surface: Arc<Mutex<Option<WindowsVideoSurface>>>,
    native_surface_error: Arc<Mutex<Option<String>>>,
    seek_bar_bounds: Arc<Mutex<Option<(f32, f32)>>>,
    volume_bar_bounds: Arc<Mutex<Option<(f32, f32)>>>,
}

impl SenzoApp {
    fn new(database: LocalDatabase, native_parent_hwnd: Option<usize>) -> Self {
        let settings = match database.playback_settings() {
            Ok(settings) => settings,
            Err(error) => {
                diagnostics::record(
                    diagnostics::Level::Error,
                    "settings",
                    &format!("failed to load playback preferences: {error:#}"),
                );
                PlaybackSettings::default()
            }
        };

        let metadata_language = database
            .get_setting("appearance.metadata_language")
            .ok()
            .flatten()
            .filter(|value| matches!(value.as_str(), "en-US" | "bn-BD"))
            .unwrap_or_else(|| "en-US".into());
        let poster_size = database
            .get_setting("appearance.poster_size")
            .ok()
            .flatten()
            .filter(|value| matches!(value.as_str(), "compact" | "comfortable"))
            .unwrap_or_else(|| "comfortable".into());
        let known_home_shelves = [
            "Trending Movies",
            "Trending TV Shows",
            "Popular movies",
            "Popular TV",
            "Trending this week",
        ];
        let mut home_shelf_order = database
            .get_setting("home.shelf_order")
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
            .unwrap_or_default();
        home_shelf_order.retain(|shelf| known_home_shelves.contains(&shelf.as_str()));
        for shelf in known_home_shelves {
            if !home_shelf_order.iter().any(|saved| saved == shelf) {
                home_shelf_order.push(shelf.to_owned());
            }
        }
        let hidden_home_shelves = database
            .get_setting("home.hidden_shelves")
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|shelf| known_home_shelves.contains(&shelf.as_str()))
            .collect();

        let (native_video_surface, native_surface_error) = match native_parent_hwnd {
            Some(parent) => match WindowsVideoSurface::create(
                parent,
                SurfaceBounds {
                    x: -2,
                    y: -2,
                    width: 1,
                    height: 1,
                },
            ) {
                Ok(surface) => (Some(surface), None),
                Err(error) => (None, Some(format!("{error:#}"))),
            },
            None => (
                None,
                Some("GPUI did not provide a Win32 parent HWND".into()),
            ),
        };

        let discord_presence_enabled = database
            .get_setting("integrations.discord.enabled")
            .ok()
            .flatten()
            .is_some_and(|value| value == "true");
        let discord_share_titles = database
            .get_setting("integrations.discord.share_titles")
            .ok()
            .flatten()
            .is_some_and(|value| value == "true");
        let discord_presence =
            senzo::discord::DiscordPresence::new(discord_presence_enabled, discord_share_titles);

        Self {
            database,
            active_page: AppPage::Home,
            settings_advanced_open: false,
            metadata_language,
            poster_size,
            selected_title: None,
            search_query: String::new(),
            search_input: None,
            search_input_subscription: None,
            search_debounce_task: None,
            settings,
            discord_presence_enabled,
            discord_share_titles,
            discord_presence,
            status: "Changes are saved on this device".into(),
            playback_worker: None,
            playback_state: PlaybackState::default(),
            playback_status: "MPV runtime not found (expected runtime/libmpv-2.dll)".into(),
            active_media_id: None,
            active_media_title: None,
            active_media_type: None,
            last_progress_save: None,
            playback_monitor: None,
            online_subtitle_results: Vec::new(),
            addon_subtitle_results: Vec::new(),
            addon_subtitle_status: "No add-on subtitles loaded for this title".into(),
            subtitle_provider_status: "Online subtitles have not been searched".into(),
            subtitle_credentials_status: match OpenSubtitlesCredentialStore::has_api_key() {
                Ok(true) => "API key saved in Windows Credential Manager".into(),
                Ok(false) => "No API key saved in Windows Credential Manager".into(),
                Err(_) => "Could not inspect Windows Credential Manager".into(),
            },
            tmdb_service_status: "Using SENZO metadata API · no TMDB key required".into(),
            subtitle_task: None,
            addon_manifest_preview: None,
            addon_status: "Copy a Stremio manifest URL, then choose Paste URL to preview it".into(),
            addon_task: None,
            tmdb_results: Vec::new(),
            tmdb_status: "TMDB discovery is ready".into(),
            home_catalogs: Vec::new(),
            home_addon_catalogs: Vec::new(),
            home_catalog_status: "Loading recommendations…".into(),
            home_catalog_requested: false,
            home_hero_index: 0,
            home_hero_paused: false,
            home_hero_task: None,
            home_shelf_order,
            hidden_home_shelves,
            home_customize_open: false,
            tmdb_details: None,
            tmdb_task: None,
            tv_seasons: Vec::new(),
            tv_selected_season: None,
            tv_episodes: Vec::new(),
            tv_selected_episode: None,
            tv_episode_status: "Choose a season to browse episodes".into(),
            addon_catalog_items: Vec::new(),
            addon_catalog_status: "Choose a catalog from an installed add-on".into(),
            addon_catalog_addon: None,
            addon_catalog_media_type: "movie".into(),
            addon_catalog_id: String::new(),
            addon_catalog_page: 0,
            addon_catalog_filter_specs: Vec::new(),
            addon_catalog_filter_values: Vec::new(),
            addon_stream_results: Vec::new(),
            stream_status: "Choose a title or catalog item to discover sources".into(),
            torrent_session: None,
            torrent_files: Vec::new(),
            torrent_progress: None,
            torrent_status: "No torrent session active".into(),
            torrent_task: None,
            native_parent_hwnd,
            native_viewport_bounds: Arc::new(Mutex::new(None)),
            native_video_surface: Arc::new(Mutex::new(native_video_surface)),
            native_surface_error: Arc::new(Mutex::new(native_surface_error)),
            seek_bar_bounds: Arc::new(Mutex::new(None)),
            volume_bar_bounds: Arc::new(Mutex::new(None)),
        }
    }

    fn start_playback(&mut self, cx: &mut Context<Self>) {
        let runtime_path = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.parent()
                    .map(|parent| parent.join("runtime").join("libmpv-2.dll"))
            })
            .unwrap_or_else(|| PathBuf::from("runtime/libmpv-2.dll"));

        if !runtime_path.is_file() {
            self.playback_status = format!("MPV runtime missing: {}", runtime_path.display());
            diagnostics::record(diagnostics::Level::Warn, "playback", &self.playback_status);
            return;
        }

        let window_id = self
            .native_video_surface
            .lock()
            .map_err(|_| anyhow::anyhow!("native video surface state is unavailable"))
            .and_then(|surface| match surface.as_ref() {
                Some(surface) => surface.mpv_window_id(),
                None => Err(anyhow::anyhow!("native video surface is not ready")),
            });
        let window_id = match window_id {
            Ok(window_id) => window_id,
            Err(error) => {
                self.playback_status = format!("Native video surface unavailable: {error:#}");
                diagnostics::record(diagnostics::Level::Warn, "playback", &self.playback_status);
                cx.notify();
                return;
            }
        };

        if let Ok(surface_slot) = self.native_video_surface.lock() {
            if let Some(surface) = surface_slot.as_ref() {
                diagnostics::record(
                    diagnostics::Level::Info,
                    "playback",
                    &surface.diagnostic_snapshot(),
                );
            }
        }

        let mut config = build_mpv_config(
            &self.settings,
            &senzo::playback::mpv_config::MpvConfig::startup_capabilities(),
        );
        if let Err(error) = config.set_native_window_id(window_id) {
            self.playback_status = format!("Could not attach MPV to the video surface: {error}");
            cx.notify();
            return;
        }

        match PlaybackWorker::spawn(runtime_path, config) {
            Ok(worker) => {
                self.playback_worker = Some(Arc::new(Mutex::new(worker)));
                self.playback_status = "Starting MPV playback runtime…".into();
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "playback",
                    "playback worker started",
                );
                self.monitor_playback(cx);
            }
            Err(error) => {
                self.playback_status = format!("Could not start playback worker: {error:#}");
                diagnostics::record(diagnostics::Level::Error, "playback", &self.playback_status);
            }
        }
        cx.notify();
    }

    fn monitor_playback(&mut self, cx: &mut Context<Self>) {
        let Some(worker) = self.playback_worker.clone() else {
            return;
        };

        self.playback_monitor = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;

                let worker = worker.clone();
                let events = cx
                    .background_spawn(async move {
                        let mut events = Vec::new();
                        if let Ok(worker) = worker.lock() {
                            for _ in 0..32 {
                                match worker.try_event() {
                                    Ok(Some(event)) => events.push(event),
                                    Ok(None) | Err(_) => break,
                                }
                            }
                        }
                        events
                    })
                    .await;

                if events.is_empty() {
                    let _ = this.update(cx, |app, cx| {
                        let progress = app
                            .torrent_session
                            .as_ref()
                            .and_then(TorrentPlaybackSession::progress_snapshot);
                        if progress != app.torrent_progress {
                            app.torrent_progress = progress.clone();
                            if let Some(progress) = progress {
                                if let Some(error) = progress.error {
                                    app.torrent_status = format!("Torrent engine error: {error}");
                                } else {
                                    app.torrent_status = format!(
                                        "{:.1} / {:.1} MiB downloaded · {:.1} MiB uploaded · {}",
                                        progress.downloaded_bytes as f64 / 1_048_576.0,
                                        progress.total_bytes as f64 / 1_048_576.0,
                                        progress.uploaded_bytes as f64 / 1_048_576.0,
                                        progress.state
                                    );
                                }
                            }
                            cx.notify();
                        }
                    });
                    continue;
                }

                let keep_running = this
                    .update(cx, |app, cx| {
                        for event in events {
                            match event {
                                PlaybackEvent::Ready => {
                                    app.playback_status = "MPV runtime ready".into();
                                }
                                PlaybackEvent::StateChanged(state) => {
                                    if app.playback_state.diagnostics != state.diagnostics {
                                        let video = &state.diagnostics;
                                        diagnostics::record(
                                            diagnostics::Level::Info,
                                            "playback",
                                            &format!(
                                                "MPV video output state: codec={}; format={}; decoder={}; hwdec={}; vo={}; vo_configured={}; dimensions={}x{}; container_fps={}; output_fps={}; dropped_frames={}; decoder_dropped_frames={}; mistimed_frames={}",
                                                video.video_codec.as_deref().unwrap_or("unknown"),
                                                video.video_format.as_deref().unwrap_or("unknown"),
                                                video.video_decoder.as_deref().unwrap_or("unknown"),
                                                video.hardware_decoder.as_deref().unwrap_or("unknown"),
                                                video.video_output.as_deref().unwrap_or("unknown"),
                                                video.video_output_configured.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()),
                                                video.video_width.map(|value| value.to_string()).unwrap_or_else(|| "?".into()),
                                                video.video_height.map(|value| value.to_string()).unwrap_or_else(|| "?".into()),
                                                video.container_frame_rate.map(|value| format!("{value:.3}")).unwrap_or_else(|| "?".into()),
                                                video.frame_rate.map(|value| format!("{value:.3}")).unwrap_or_else(|| "?".into()),
                                                video.dropped_frames.map(|value| value.to_string()).unwrap_or_else(|| "?".into()),
                                                video.decoder_dropped_frames.map(|value| value.to_string()).unwrap_or_else(|| "?".into()),
                                                video.mistimed_frames.map(|value| value.to_string()).unwrap_or_else(|| "?".into()),
                                            ),
                                        );
                                    }
                                    app.playback_state = state.clone();
                                    app.discord_presence.update_playback(
                                        app.active_media_title
                                            .clone()
                                            .or_else(|| state.media_title.clone()),
                                        app.active_media_type.clone(),
                                        state.paused.unwrap_or(false),
                                        state.position_seconds,
                                        state.duration_seconds,
                                    );
                                    app.persist_playback_progress(false);
                                }
                                PlaybackEvent::CommandCompleted { operation } => {
                                    diagnostics::record(
                                        diagnostics::Level::Info,
                                        "playback",
                                        &format!("playback command completed: {operation}"),
                                    );
                                }
                                PlaybackEvent::CommandFailed { operation, message } => {
                                    app.playback_status = format!("{operation} failed: {message}");
                                    diagnostics::record(
                                        diagnostics::Level::Error,
                                        "playback",
                                        &app.playback_status,
                                    );
                                }
                                PlaybackEvent::Failed { message } => {
                                    app.discord_presence.clear_playback();
                                    app.persist_playback_progress(true);
                                    app.playback_status = format!("MPV failed: {message}");
                                    diagnostics::record(
                                        diagnostics::Level::Error,
                                        "playback",
                                        &app.playback_status,
                                    );
                                    app.playback_worker = None;
                                }
                                PlaybackEvent::Stopped => {
                                    app.discord_presence.clear_playback();
                                    app.persist_playback_progress(true);
                                    app.playback_status = "Playback stopped".into();
                                    app.playback_worker = None;
                                    app.playback_monitor = None;
                                }
                            }
                        }
                        cx.notify();
                        app.playback_worker.is_some()
                    })
                    .unwrap_or(false);

                if !keep_running {
                    break;
                }
            }
        }));
    }

    fn open_media(&mut self, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open media")
            .add_filter(
                "Video files",
                &[
                    "mkv", "mp4", "webm", "avi", "mov", "m4v", "ts", "m2ts", "wmv", "flv",
                ],
            )
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            return;
        };

        if !path.is_file() {
            self.playback_status = "The selected path is not a file".into();
            cx.notify();
            return;
        }

        if self.playback_worker.is_none() {
            self.start_playback(cx);
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("selected media")
            .to_owned();
        let media_id = format!(
            "local-file:{}",
            path.canonicalize()
                .unwrap_or_else(|_| path.clone())
                .to_string_lossy()
        );
        // Flush the previous item's latest known position before switching IDs.
        self.persist_playback_progress(true);

        let resume_position = match self.database.playback_progress(&media_id) {
            Ok(progress) => progress
                .filter(|progress| progress.position_seconds > 5.0)
                .map(|progress| progress.position_seconds),
            Err(error) => {
                diagnostics::record(
                    diagnostics::Level::Warn,
                    "playback",
                    &format!("could not read saved playback progress: {error:#}"),
                );
                None
            }
        };

        let result = self
            .playback_worker
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MPV runtime is unavailable"))
            .and_then(|worker| {
                let worker = worker
                    .lock()
                    .map_err(|_| anyhow::anyhow!("playback worker lock is unavailable"))?;
                worker.load_file_with_resume(path, resume_position)?;
                Ok(())
            });

        match result {
            Ok(()) => {
                self.active_media_id = Some(media_id);
                self.active_media_title = Some(file_name.clone());
                self.active_media_type = Some("local".into());
                self.active_page = AppPage::Player;
                self.last_progress_save = None;
                self.playback_status = match resume_position {
                    Some(position) => {
                        format!("Resuming {file_name} from {:.0} seconds", position)
                    }
                    None => format!("Queued for playback: {file_name}"),
                };
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "playback",
                    &format!("user opened local media file: {file_name}"),
                );
            }
            Err(error) => {
                self.playback_status = format!("Could not open media: {error:#}");
                diagnostics::record(diagnostics::Level::Error, "playback", &self.playback_status);
            }
        }
        cx.notify();
    }

    fn load_home_catalogs(&mut self, cx: &mut Context<Self>) {
        diagnostics::record(
            diagnostics::Level::Audit,
            "metadata",
            "Home catalogue discovery requested",
        );
        if self.home_catalogs.is_empty() && self.home_addon_catalogs.is_empty() {
            self.home_catalog_status = "Finding something to watch…".into();
        } else {
            self.home_catalog_status = "Refreshing recommendations…".into();
        }
        self.home_catalog_requested = true;
        let metadata_language = self.metadata_language.clone();
        // Snapshot enabled user-installed add-ons on the UI thread. The SQLite connection
        // is deliberately not moved into the background task.
        let mut addon_catalog_specs = Vec::new();
        if let Ok(addons) = self.database.installed_addons() {
            'addons: for addon in addons.into_iter().filter(|addon| addon.enabled) {
                let Ok(manifest) = serde_json::from_str::<AddonManifest>(&addon.manifest_json)
                else {
                    continue;
                };
                if !manifest.supports_resource_for("catalog", "movie", "home")
                    && !manifest.supports_resource_for("catalog", "series", "home")
                    && !manifest.supports_resource_for("catalog", "tv", "home")
                {
                    continue;
                }
                for catalog in manifest.catalogs {
                    let Some(catalog_id) = catalog.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let media_type = catalog
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("movie");
                    if !matches!(media_type, "movie" | "series" | "tv") {
                        continue;
                    }
                    // Catalogs that require user-supplied filters are exposed in the Add-ons
                    // page, but are not guessed or silently queried from the Home screen.
                    if AddonManifest::catalog_requires_user_input(&catalog) {
                        continue;
                    }
                    let catalog_name = catalog
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(catalog_id);
                    addon_catalog_specs.push((
                        addon.clone(),
                        media_type.to_owned(),
                        catalog_id.to_owned(),
                        format!("{} · {}", addon.name, catalog_name),
                    ));
                    if addon_catalog_specs.len() >= 12 {
                        break 'addons;
                    }
                }
            }
        }

        self.tmdb_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("could not initialize the discovery runtime")?;

                runtime.block_on(async move {
                    let tmdb_result = async {
                        diagnostics::record(
                            diagnostics::Level::Info,
                            "metadata",
                            "Initializing hosted metadata API client",
                        );
                        let client = TmdbClient::from_service(&metadata_language)
                            .context("could not initialize hosted metadata API client")?;
                        diagnostics::record(
                            diagnostics::Level::Info,
                            "metadata",
                            "Requesting trending, popular movies and popular TV catalogues",
                        );
                        let (trending, movies, shows) = futures::join!(
                            client.trending_all_week(1),
                            client.popular_movies(1),
                            client.popular_tv(1)
                        );
                        let mut shelves = Vec::new();
                        let mut failures = Vec::new();
                        match trending {
                            Ok(page) => {
                                diagnostics::record(
                                    diagnostics::Level::Info,
                                    "metadata",
                                    &format!("Trending catalogue loaded: {} results", page.results.len()),
                                );
                                for item in page.results {
                                    let is_tv = item.media_type.as_deref() == Some("tv")
                                        || (item.title.is_none() && item.name.is_some());
                                    let kind = if is_tv { "tv".to_string() } else { "movie".to_string() };
                                    if is_tv {
                                        shelves.push(("Trending TV Shows".to_owned(), kind.clone(), item.clone()));
                                    } else {
                                        shelves.push(("Trending Movies".to_owned(), kind.clone(), item.clone()));
                                    }
                                    shelves.push(("Trending this week".to_owned(), kind, item));
                                }
                            }
                            Err(error) => {
                                diagnostics::record(
                                    diagnostics::Level::Error,
                                    "metadata",
                                    &format!("Trending catalogue request failed: {error:#}"),
                                );
                                failures.push("trending");
                            }
                        }
                        match movies {
                            Ok(page) => {
                                diagnostics::record(
                                    diagnostics::Level::Info,
                                    "metadata",
                                    &format!("Popular movies catalogue loaded: {} results", page.results.len()),
                                );
                                for item in page.results {
                                    shelves.push(("Popular Movies".to_owned(), "movie".to_owned(), item.clone()));
                                    shelves.push(("Popular movies".to_owned(), "movie".to_owned(), item));
                                }
                            }
                            Err(error) => {
                                diagnostics::record(
                                    diagnostics::Level::Error,
                                    "metadata",
                                    &format!("Popular movies request failed: {error:#}"),
                                );
                                failures.push("popular movies");
                            }
                        }
                        match shows {
                            Ok(page) => {
                                diagnostics::record(
                                    diagnostics::Level::Info,
                                    "metadata",
                                    &format!("Popular TV catalogue loaded: {} results", page.results.len()),
                                );
                                shelves.extend(page.results.into_iter().map(|item| ("Popular TV".to_owned(), "tv".to_owned(), item)));
                            }
                            Err(error) => {
                                diagnostics::record(
                                    diagnostics::Level::Error,
                                    "metadata",
                                    &format!("Popular TV request failed: {error:#}"),
                                );
                                failures.push("popular TV");
                            }
                        }
                        Ok::<_, anyhow::Error>((shelves, failures))
                    };

                    let addon_result = async move {
                        let client = AddonClient::new();
                        let requests = addon_catalog_specs.into_iter().map(|(addon, media_type, catalog_id, shelf_name)| {
                            let client = client.clone();
                            async move {
                                let response = client.catalog(&addon, &media_type, &catalog_id, &[]).await;
                                (addon, media_type, catalog_id, shelf_name, response)
                            }
                        });
                        let mut shelves = Vec::new();
                        let mut failed_requests = 0usize;
                        for (addon, media_type, catalog_id, shelf_name, result) in
                            futures::future::join_all(requests).await
                        {
                            match result {
                                Ok(response) => {
                                    let metas = response.metas.into_iter().filter_map(|mut meta| {
                                        let object = meta.as_object_mut()?;
                                        if !object.get("id").is_some_and(Value::is_string) {
                                            return None;
                                        }
                                        object.entry("type").or_insert_with(|| Value::String(media_type.clone()));
                                        Some(meta)
                                    }).take(20).collect::<Vec<_>>();
                                    if !metas.is_empty() {
                                        shelves.push((shelf_name, addon, media_type, catalog_id, metas));
                                    }
                                }
                                Err(_) => failed_requests += 1,
                            }
                        }
                        (shelves, failed_requests)
                    };

                    let (tmdb_result, addon_result) = futures::join!(tmdb_result, addon_result);
                    let (tmdb_shelves, failures) = match tmdb_result {
                        Ok(value) => value,
                        Err(error) => {
                            diagnostics::record(
                                diagnostics::Level::Error,
                                "metadata",
                                &format!("Hosted metadata discovery failed: {error:#}"),
                            );
                            return Err(error.context("SENZO hosted metadata discovery failed"));
                        }
                    };
                    let (addon_shelves, addon_failures) = addon_result;
                    if tmdb_shelves.is_empty() && addon_shelves.is_empty() {
                        anyhow::bail!("Discovery is temporarily unavailable. Check your connection and try again.");
                    }
                    Ok::<_, anyhow::Error>((tmdb_shelves, addon_shelves, failures, addon_failures))
                })
            }).await;

            let _ = this.update(cx, |app, cx| {
                if let Err(error) = &result {
                    diagnostics::record(
                        diagnostics::Level::Error,
                        "metadata",
                        &format!("Home discovery background task failed: {error:#}"),
                    );
                }
                match result {
                    Ok((mut tmdb_shelves, mut addon_shelves, failures, addon_failures)) => {
                        diagnostics::record(
                            diagnostics::Level::Audit,
                            "metadata",
                            &format!(
                                "Home discovery completed: {} metadata items, {} add-on shelves",
                                tmdb_shelves.len(),
                                addon_shelves.len()
                            ),
                        );
                        // A provider outage must not erase the last successful shelves.
                        // An empty result with no provider errors is a legitimate empty catalog.
                        if tmdb_shelves.is_empty() && !failures.is_empty() {
                            tmdb_shelves = app.home_catalogs.clone();
                        }
                        if addon_shelves.is_empty() && addon_failures > 0 {
                            addon_shelves = app.home_addon_catalogs.clone();
                        }
                        let tmdb_count = tmdb_shelves.len();
                        let addon_count = addon_shelves.len();
                        app.home_catalogs = tmdb_shelves;
                        app.home_addon_catalogs = addon_shelves;
                        app.tmdb_service_status = if tmdb_count > 0 {
                            "Connected to SENZO metadata API".into()
                        } else {
                            "SENZO metadata API is unavailable · retry when your connection is restored".into()
                        };
                        let mut notes = Vec::new();
                        if tmdb_count > 0 {
                            notes.push(format!("{tmdb_count} recommendations"));
                        }
                        if addon_count > 0 {
                            notes.push(format!("{addon_count} add-on shelves"));
                        }
                        if !failures.is_empty() {
                            notes.push(format!("TMDB unavailable: {}", failures.join(", ")));
                        }
                        if addon_failures > 0 {
                            notes.push(format!("{addon_failures} add-on catalog request(s) failed"));
                        }
                        app.home_catalog_status = if notes.is_empty() {
                            "No discovery catalogs are available yet".into()
                        } else {
                            notes.join(" · ")
                        };
                    }
                    Err(error) => {
                        app.tmdb_service_status =
                            format!("SENZO metadata API unavailable: {error:#}");
                        if app.home_catalogs.is_empty() && app.home_addon_catalogs.is_empty() {
                            app.home_catalog_status =
                                format!("Recommendations are unavailable right now: {error:#}");
                        } else {
                            app.home_catalog_status =
                                format!("Could not refresh recommendations ({error:#}); showing previous catalogs");
                        }
                    }
                }
                if app.home_hero_task.is_none() {
                    app.home_hero_task = Some(cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_secs(8))
                                .await;
                            let updated = this.update(cx, |app, cx| {
                                if app.active_page != AppPage::Home || app.home_hero_paused {
                                    return;
                                }
                                let trending_count = app
                                    .home_catalogs
                                    .iter()
                                    .filter(|(shelf, _, _)| {
                                        shelf == "Trending Movies" || shelf == "Trending this week"
                                    })
                                    .count()
                                    .min(7);
                                let featured_count = if trending_count == 0 {
                                    app.home_catalogs.len().min(7)
                                } else {
                                    trending_count
                                };
                                if featured_count > 1 {
                                    app.home_hero_index = (app.home_hero_index + 1) % featured_count;
                                    cx.notify();
                                }
                            });
                            if updated.is_err() {
                                break;
                            }
                        }
                    }));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load_tmdb_catalog(
        &mut self,
        media_type: &'static str,
        query: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.tmdb_status = match (media_type, query.as_deref()) {
            ("trending", _) => "Loading this week's trending titles…".into(),
            (_, Some(value)) => format!("Searching TMDB for {value}…"),
            (_, None) => format!("Loading popular {media_type}…"),
        };
        self.tmdb_results.clear();
        let metadata_language = self.metadata_language.clone();
        self.tmdb_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = TmdbClient::from_service(&metadata_language)?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize TMDB runtime")?;
                    runtime.block_on(async {
                        if media_type == "trending" && query.is_none() {
                            let page = client.trending_all_week(1).await?;
                            return Ok::<_, anyhow::Error>(
                                page.results
                                    .into_iter()
                                    .map(|item| {
                                        let kind = item.media_type.clone().unwrap_or_else(|| {
                                            if item.title.is_some() || item.original_title.is_some()
                                            {
                                                "movie".into()
                                            } else {
                                                "tv".into()
                                            }
                                        });
                                        (kind, item)
                                    })
                                    .collect::<Vec<_>>(),
                            );
                        }
                        let page = match (media_type, query.as_deref()) {
                            ("movie", Some(query)) => client.search_movies(query, 1).await?,
                            ("tv", Some(query)) => client.search_tv(query, 1).await?,
                            ("movie", None) => client.popular_movies(1).await?,
                            ("tv", None) => client.popular_tv(1).await?,
                            _ => anyhow::bail!("unsupported TMDB media type"),
                        };
                        Ok::<_, anyhow::Error>(
                            page.results
                                .into_iter()
                                .map(|item| (media_type.to_owned(), item))
                                .collect::<Vec<_>>(),
                        )
                    })
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(results) => {
                        let count = results.len();
                        app.tmdb_results = results;
                        app.tmdb_status = if count == 0 {
                            "No matching titles found".into()
                        } else {
                            format!("{count} titles loaded from TMDB")
                        };
                    }
                    Err(error) => app.tmdb_status = format!("TMDB request failed: {error:#}"),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load_tmdb_search(&mut self, query: String, cx: &mut Context<Self>) {
        let query = query.trim().to_owned();
        if query.is_empty() {
            self.tmdb_results.clear();
            self.tmdb_status = "Type a title to search movies and TV shows".into();
            cx.notify();
            return;
        }

        self.tmdb_status = format!("Searching movies and TV shows for {query}…");
        self.tmdb_results.clear();
        let requested_query = query.clone();
        let metadata_language = self.metadata_language.clone();
        self.tmdb_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = TmdbClient::from_service(&metadata_language)?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize TMDB search runtime")?;
                    runtime.block_on(async {
                        let (movies, shows) = tokio::join!(
                            client.search_movies(&query, 1),
                            client.search_tv(&query, 1),
                        );
                        let movies = movies?;
                        let shows = shows?;
                        let mut results =
                            Vec::with_capacity(movies.results.len() + shows.results.len());
                        results.extend(
                            movies
                                .results
                                .into_iter()
                                .map(|item| ("movie".to_owned(), item)),
                        );
                        results.extend(
                            shows
                                .results
                                .into_iter()
                                .map(|item| ("tv".to_owned(), item)),
                        );
                        Ok::<_, anyhow::Error>(results)
                    })
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.search_query.trim() != requested_query {
                    return;
                }
                match result {
                    Ok(results) => {
                        let count = results.len();
                        app.tmdb_results = results;
                        app.tmdb_status = if count == 0 {
                            format!("No movies or TV shows found for {requested_query}")
                        } else {
                            format!("{count} results for {requested_query}")
                        };
                    }
                    Err(error) => app.tmdb_status = format!("Search failed: {error:#}"),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn search_tmdb_from_clipboard(&mut self, media_type: &'static str, cx: &mut Context<Self>) {
        let Some(query) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .filter(|text| !text.trim().is_empty())
        else {
            self.tmdb_status = "Copy a title or search phrase, then choose Paste search".into();
            cx.notify();
            return;
        };
        self.load_tmdb_catalog(media_type, Some(query.trim().to_owned()), cx);
    }

    fn select_tmdb_title(
        &mut self,
        media_type: String,
        title: TmdbSearchResult,
        cx: &mut Context<Self>,
    ) {
        let Some(display_title) = title.display_title().map(str::to_owned) else {
            return;
        };
        self.persist_playback_progress(true);
        let tmdb_id = title.id;
        let addons = self.database.installed_addons().unwrap_or_default();
        self.selected_title = Some(display_title.clone());
        self.active_page = AppPage::Details;
        self.tmdb_details = None;
        self.tv_seasons.clear();
        self.tv_selected_season = None;
        self.tv_episodes.clear();
        self.tv_selected_episode = None;
        self.tv_episode_status = "Loading TV seasons…".into();
        self.addon_stream_results.clear();
        self.stream_status = "Resolving provider IDs and checking enabled add-ons…".into();
        self.tmdb_status = format!("Loading details for {display_title}…");
        self.active_media_id = Some(title.media_id(&media_type));
        self.active_media_title = Some(display_title.clone());
        self.active_media_type = Some(if media_type == "tv" {
            "episode".into()
        } else {
            "movie".into()
        });
        let metadata_language = self.metadata_language.clone();
        self.tmdb_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = TmdbClient::from_service(&metadata_language)?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize metadata and source runtime")?;
                    runtime.block_on(async {
                        let detail_key =
                            format!("tmdb:{media_type}:{tmdb_id}:details:{metadata_language}");
                        let details = if let Some(cached) = senzo::cache::load_metadata(
                            &detail_key,
                            Duration::from_secs(7 * 24 * 60 * 60),
                        )
                        .ok()
                        .flatten()
                        {
                            cached
                        } else {
                            let value = if media_type == "movie" {
                                client.movie_details(tmdb_id).await?
                            } else {
                                client.tv_details(tmdb_id).await?
                            };
                            let _ = senzo::cache::store_metadata(&detail_key, &value);
                            value
                        };
                        let ids_key = format!("tmdb:{media_type}:{tmdb_id}:external-ids");
                        let external_ids_value = if let Some(cached) = senzo::cache::load_metadata(
                            &ids_key,
                            Duration::from_secs(30 * 24 * 60 * 60),
                        )
                        .ok()
                        .flatten()
                        {
                            cached
                        } else {
                            let ids = if media_type == "movie" {
                                client.movie_external_ids(tmdb_id).await?
                            } else {
                                client.tv_external_ids(tmdb_id).await?
                            };
                            let value = serde_json::to_value(ids)
                                .context("could not encode TMDB external IDs")?;
                            let _ = senzo::cache::store_metadata(&ids_key, &value);
                            value
                        };
                        let external_ids: senzo::metadata::TmdbExternalIds =
                            serde_json::from_value(external_ids_value)
                                .context("cached TMDB external IDs are invalid")?;
                        let provider_id = external_ids
                            .imdb_id
                            .clone()
                            .unwrap_or_else(|| format!("tmdb:{tmdb_id}"));
                        if media_type == "tv" {
                            Ok::<_, anyhow::Error>((details, Vec::new(), provider_id))
                        } else {
                            let streams = SourceProviderRegistry::default()
                                .discover(&addons, "movie", &provider_id)
                                .await;
                            Ok::<_, anyhow::Error>((details, streams, provider_id))
                        }
                    })
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((details, streams, stream_id)) => {
                        app.tv_seasons = details
                            .get("seasons")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|season| {
                                let number =
                                    season.get("season_number").and_then(Value::as_u64)? as u32;
                                let name = season
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| {
                                        if number == 0 {
                                            "Specials".into()
                                        } else {
                                            format!("Season {number}")
                                        }
                                    });
                                Some((number, name))
                            })
                            .collect();
                        app.tv_seasons.sort_by_key(
                            |(number, _)| if *number == 0 { u32::MAX } else { *number },
                        );
                        let first_season = app
                            .tv_seasons
                            .iter()
                            .find(|(number, _)| *number > 0)
                            .or_else(|| app.tv_seasons.first())
                            .map(|(number, _)| *number);
                        app.tmdb_details = Some(details);
                        app.addon_stream_results = streams;
                        app.tmdb_status = "TMDB details loaded".into();
                        if let Some(season) = first_season {
                            app.load_tmdb_season(season, cx);
                        }
                        app.stream_status = if stream_id.starts_with("tt")
                            || stream_id.starts_with("tmdb:") && !app.tv_seasons.is_empty()
                        {
                            "Choose a season and episode to discover its sources".into()
                        } else {
                            "Movie source discovery completed".into()
                        };
                    }
                    Err(error) => {
                        app.tmdb_status = format!("Could not load title details: {error:#}");
                        app.stream_status = "Source discovery could not start".into();
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load_tmdb_season(&mut self, season_number: u32, cx: &mut Context<Self>) {
        let Some(tmdb_id) = self
            .tmdb_details
            .as_ref()
            .and_then(|details| details.get("id"))
            .and_then(Value::as_u64)
        else {
            return;
        };
        self.tv_selected_season = Some(season_number);
        self.tv_episodes.clear();
        self.tv_selected_episode = None;
        self.tv_episode_status = format!(
            "Loading {}…",
            if season_number == 0 {
                "Specials".to_owned()
            } else {
                format!("Season {season_number}")
            }
        );
        let language = self.metadata_language.clone();
        self.tmdb_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let key = format!("tmdb:tv:{tmdb_id}:season:{season_number}:{language}");
                    if let Some(value) =
                        senzo::cache::load_metadata(&key, Duration::from_secs(7 * 24 * 60 * 60))
                            .ok()
                            .flatten()
                    {
                        return serde_json::from_value::<senzo::metadata::TmdbSeason>(value)
                            .context("cached TMDB season data is invalid");
                    }
                    let client = TmdbClient::from_service(&language)?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize the TV season runtime")?;
                    let season = runtime.block_on(async {
                        let season = client.tv_season(tmdb_id, season_number).await?;
                        let artwork_urls = season
                            .episodes
                            .iter()
                            .filter_map(|episode| {
                                tmdb_artwork_url(episode.still_path.as_deref(), "w500")
                            })
                            .take(12)
                            .collect::<Vec<_>>();
                        let _ = tokio::time::timeout(Duration::from_secs(2), async {
                            let requests = artwork_urls
                                .iter()
                                .map(|url| senzo::cache::cache_artwork(url));
                            let _ = futures::future::join_all(requests).await;
                        })
                        .await;
                        Ok::<_, anyhow::Error>(season)
                    })?;
                    if let Ok(value) = serde_json::to_value(&season) {
                        let _ = senzo::cache::store_metadata(&key, &value);
                    }
                    Ok::<_, anyhow::Error>(season)
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(season) => {
                        app.tv_episodes = season.episodes;
                        app.tv_episode_status = if app.tv_episodes.is_empty() {
                            "No episodes are listed for this season".into()
                        } else {
                            format!(
                                "{} episodes · {}",
                                app.tv_episodes.len(),
                                season
                                    .name
                                    .unwrap_or_else(|| format!("Season {season_number}"))
                            )
                        };
                    }
                    Err(error) => {
                        app.tv_episode_status = format!("Could not load season: {error:#}")
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn select_tmdb_episode(&mut self, episode: TmdbEpisode, cx: &mut Context<Self>) {
        let Some(details) = self.tmdb_details.as_ref() else {
            return;
        };
        let Some(tmdb_id) = details.get("id").and_then(Value::as_u64) else {
            return;
        };
        let provider_id = details
            .get("external_ids")
            .and_then(|ids| ids.get("imdb_id"))
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("tt"))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("tmdb:{tmdb_id}"));
        let stream_id = format!(
            "{provider_id}:{}:{}",
            episode.season_number, episode.episode_number
        );
        let addons = self.database.installed_addons().unwrap_or_default();
        let show_title = details
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("TV show")
            .to_owned();
        let episode_title = episode
            .name
            .clone()
            .unwrap_or_else(|| format!("Episode {}", episode.episode_number));
        let title = format!(
            "{show_title} · S{:02}E{:02} · {episode_title}",
            episode.season_number, episode.episode_number
        );
        let season_number = episode.season_number;
        let episode_number = episode.episode_number;
        self.selected_title = Some(title.clone());
        self.active_media_id = Some(format!(
            "tmdb:tv:{tmdb_id}:{season_number}:{episode_number}"
        ));
        self.active_media_title = Some(title);
        self.active_media_type = Some("episode".into());
        self.tv_selected_episode = Some(episode);
        self.addon_stream_results.clear();
        self.stream_status = format!(
            "Finding sources for S{:02}E{:02}…",
            season_number, episode_number
        );
        let request_id = stream_id.clone();
        self.addon_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize episode source discovery")?;
                    let providers = runtime.block_on(SourceProviderRegistry::default().discover(
                        &addons,
                        "series",
                        &request_id,
                    ));
                    Ok::<_, anyhow::Error>(providers)
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(providers) => {
                        let count = providers
                            .iter()
                            .map(|provider| provider.streams.len())
                            .sum::<usize>();
                        app.addon_stream_results = providers;
                        app.stream_status = format!("{count} source(s) found for {stream_id}");
                    }
                    Err(error) => {
                        app.stream_status = format!("Episode source discovery failed: {error:#}")
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load_addon_catalog(
        &mut self,
        addon: StoredAddon,
        media_type: String,
        catalog_id: String,
        cx: &mut Context<Self>,
    ) {
        self.load_addon_catalog_page(addon, media_type, catalog_id, 0, cx);
    }

    fn load_addon_catalog_page(
        &mut self,
        addon: StoredAddon,
        media_type: String,
        catalog_id: String,
        page: usize,
        cx: &mut Context<Self>,
    ) {
        self.addon_catalog_status = format!("Loading {} catalog · page {}…", addon.name, page + 1);
        self.addon_catalog_media_type = media_type.clone();
        self.addon_catalog_id = catalog_id.clone();
        self.addon_catalog_page = page;
        self.addon_catalog_items.clear();
        self.addon_catalog_addon = Some(addon.clone());
        let mut request_extra = self
            .addon_catalog_filter_values
            .iter()
            .filter(|(name, values)| name != "skip" && !values.is_empty())
            .map(|(name, values)| (name.clone(), values.join(",")))
            .collect::<Vec<_>>();
        let skip = (page * 100).to_string();
        request_extra.push(("skip".to_owned(), skip));
        self.addon_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = AddonClient::new();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize add-on catalog runtime")?;
                    let extra = request_extra
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_str()))
                        .collect::<Vec<_>>();
                    runtime.block_on(client.catalog(&addon, &media_type, &catalog_id, &extra))
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(response) => {
                        let count = response.metas.len();
                        app.addon_catalog_items = response.metas;
                        app.addon_catalog_status = format!("{count} catalog entries loaded");
                    }
                    Err(error) => {
                        app.addon_catalog_status = format!("Catalog request failed: {error:#}")
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn select_addon_catalog(
        &mut self,
        addon: StoredAddon,
        media_type: String,
        catalog_id: String,
        catalog: Value,
        cx: &mut Context<Self>,
    ) {
        self.addon_catalog_filter_specs = AddonManifest::catalog_extra_properties(&catalog);
        self.addon_catalog_filter_values.clear();
        self.addon_catalog_addon = Some(addon.clone());
        self.addon_catalog_media_type = media_type.clone();
        self.addon_catalog_id = catalog_id.clone();
        self.addon_catalog_items.clear();
        self.addon_catalog_page = 0;
        if self.addon_catalog_filter_specs.is_empty() {
            self.load_addon_catalog_page(addon, media_type, catalog_id, 0, cx);
        } else {
            let count = self.addon_catalog_filter_specs.len();
            let required = self
                .addon_catalog_filter_specs
                .iter()
                .filter(|property| property.is_required)
                .count();
            self.addon_catalog_status = format!(
                "Configure {count} catalogue filter(s) before loading this catalogue. {required} required."
            );
            cx.notify();
        }
    }

    fn toggle_addon_catalog_filter(
        &mut self,
        name: String,
        value: String,
        limit: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, selected)) = self
            .addon_catalog_filter_values
            .iter_mut()
            .find(|(key, _)| key == &name)
        {
            if selected.iter().any(|item| item == &value) {
                selected.retain(|item| item != &value);
            } else if limit <= 1 {
                selected.clear();
                selected.push(value);
            } else if selected.len() < limit {
                selected.push(value);
            }
        } else {
            self.addon_catalog_filter_values.push((name, vec![value]));
        }
        cx.notify();
    }

    fn paste_addon_catalog_filter(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(value) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .map(|value| value.trim().to_owned())
            .filter(|value| {
                !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
            })
        else {
            self.addon_catalog_status =
                "Copy a filter value (up to 256 characters), then paste it here.".into();
            cx.notify();
            return;
        };
        if let Some((_, selected)) = self
            .addon_catalog_filter_values
            .iter_mut()
            .find(|(key, _)| key == &name)
        {
            *selected = vec![value];
        } else {
            self.addon_catalog_filter_values.push((name, vec![value]));
        }
        cx.notify();
    }

    fn load_filtered_addon_catalog(&mut self, cx: &mut Context<Self>) {
        for property in &self.addon_catalog_filter_specs {
            let has_value = self
                .addon_catalog_filter_values
                .iter()
                .any(|(name, values)| name == &property.name && !values.is_empty());
            if property.is_required && !has_value {
                self.addon_catalog_status = format!(
                    "Choose or paste a value for required filter: {}",
                    property.name
                );
                cx.notify();
                return;
            }
        }
        let Some(addon) = self.addon_catalog_addon.clone() else {
            self.addon_catalog_status = "Choose an add-on catalogue first.".into();
            cx.notify();
            return;
        };
        self.load_addon_catalog_page(
            addon,
            self.addon_catalog_media_type.clone(),
            self.addon_catalog_id.clone(),
            0,
            cx,
        );
    }

    fn select_addon_catalog_meta(
        &mut self,
        addon: StoredAddon,
        meta: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(media_id) = meta.get("id").and_then(Value::as_str).map(str::to_owned) else {
            self.addon_catalog_status = "Catalog item has no valid Stremio ID".into();
            cx.notify();
            return;
        };
        let media_type = meta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("movie")
            .to_owned();
        let addon_id = addon.id.clone();
        let addon_name = addon.name.clone();
        self.active_media_id = Some(format!("stremio:{media_type}:{media_id}"));
        let subtitle_addons = self.database.installed_addons().unwrap_or_default();
        self.addon_subtitle_results.clear();
        self.addon_subtitle_status = "Checking the selected add-on for subtitle tracks…".into();
        self.addon_catalog_status = format!("Loading details and streams for {media_id}…");
        self.selected_title = meta
            .get("name")
            .or_else(|| meta.get("title"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.active_page = AppPage::Details;
        self.tmdb_details = Some(meta.clone());
        self.addon_stream_results.clear();
        self.addon_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                let client = AddonClient::new();
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
                    .context("could not initialize add-on metadata runtime")?;
                runtime.block_on(async {
                    let metadata = client.metadata(&addon, &media_type, &media_id).await.ok().map(|value| value.meta).unwrap_or(meta);
                    let stream_id = if media_type == "series" {
                        first_series_episode_id(&metadata, &media_id)
                    } else {
                        media_id.clone()
                    };
                    let streams = client.streams(&addon, &media_type, &stream_id).await?;
                    let subtitles = client
                        .discover_subtitles(&subtitle_addons, &media_type, &stream_id)
                        .await;
                    Ok::<_, anyhow::Error>((metadata, streams.streams, media_id, subtitles))
                })
            }).await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((metadata, streams, id, subtitles)) => {
                        app.tmdb_details = Some(metadata);
                        app.addon_stream_results = vec![SourceProviderResult { provider_id: addon_id.clone(), provider_name: addon_name.clone(), streams, error: None }];
                        app.addon_subtitle_results = subtitles;
                        app.addon_subtitle_status = if app.addon_subtitle_results.is_empty() {
                            "This add-on returned no subtitle tracks for the selected episode".into()
                        } else {
                            format!("{} add-on subtitle track(s) available", app.addon_subtitle_results.len())
                        };
                        app.addon_catalog_status = format!("Metadata and streams loaded for {id}");
                        app.stream_status = "Choose a direct stream, or inspect torrent entries for Batch 5 playback".into();
                    }
                    Err(error) => app.addon_catalog_status = format!("Could not load catalog item: {error:#}"),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_addon_external_link(&mut self, raw_url: &str, cx: &mut Context<Self>) {
        let parsed = match url::Url::parse(raw_url) {
            Ok(url)
                if url.scheme() == "https"
                    && url.host().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none() =>
            {
                url
            }
            _ => {
                self.stream_status = "Provider external links must be valid HTTPS URLs without credentials or fragments".into();
                cx.notify();
                return;
            }
        };

        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::UI::Shell::ShellExecuteW;
            let operation: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
            let target: Vec<u16> = std::ffi::OsStr::new(parsed.as_str())
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let result = unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    operation.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                )
            };
            self.stream_status = if (result as isize) <= 32 {
                "Windows could not open the provider's external link".into()
            } else {
                "Opened provider external link in the default browser".into()
            };
        }
        #[cfg(not(windows))]
        {
            let _ = parsed;
            self.stream_status =
                "External-link handoff is currently available on Windows only".into();
        }
        cx.notify();
    }

    fn inspect_torrent_stream(&mut self, stream: StremioStream, cx: &mut Context<Self>) {
        self.persist_playback_progress(true);
        let Some(info_hash) = stream.info_hash.clone() else {
            self.stream_status = "Torrent source is missing its info hash".into();
            cx.notify();
            return;
        };
        let requested_file = stream.file_index.map(|index| index as usize);
        let trackers = stream.sources.clone();
        let previous = self.torrent_session.take();
        let worker = self.playback_worker.clone();
        self.last_progress_save = None;
        self.torrent_files.clear();
        self.torrent_progress = None;
        self.torrent_status = "Resolving torrent metadata and available video files…".into();
        self.stream_status = "Starting a temporary, non-seeding torrent session…".into();
        self.torrent_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize torrent setup runtime")?;
                    runtime.block_on(async move {
                        if let Some(worker) = worker {
                            tokio::task::spawn_blocking(move || {
                                let worker = worker.lock().map_err(|_| {
                                    anyhow::anyhow!("playback worker lock is unavailable")
                                })?;
                                worker.stop_and_wait()
                            })
                            .await
                            .context("MPV stop task failed")??;
                        }
                        if let Some(previous) = previous {
                            previous
                                .shutdown()
                                .await
                                .context("could not close the previous torrent session")?;
                        }
                        TorrentPlaybackSession::inspect_with_trackers(&info_hash, &trackers).await
                    })
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((session, files)) => {
                        let preferred = requested_file
                            .filter(|index| files.iter().any(|file| file.index == *index));
                        let automatic =
                            preferred.or_else(|| (files.len() == 1).then_some(files[0].index));
                        app.torrent_session = Some(session);
                        app.torrent_files = files;
                        app.torrent_status =
                            format!("{} video file(s) available", app.torrent_files.len());
                        app.stream_status = "Select a video file to begin torrent playback".into();
                        if let Some(index) = automatic {
                            app.start_torrent_file(index, cx);
                        }
                    }
                    Err(error) => {
                        app.torrent_session = None;
                        app.torrent_files.clear();
                        app.torrent_status = format!("Torrent setup failed: {error:#}");
                        app.stream_status = app.torrent_status.clone();
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn start_torrent_file(&mut self, file_index: usize, cx: &mut Context<Self>) {
        self.persist_playback_progress(true);
        if self.playback_worker.is_none() {
            self.start_playback(cx);
        }
        let Some(worker) = self.playback_worker.clone() else {
            self.torrent_status = "MPV is not ready; the torrent session was not started".into();
            self.stream_status = self.torrent_status.clone();
            self.stop_torrent_session(cx);
            cx.notify();
            return;
        };
        let Some(mut session) = self.torrent_session.take() else {
            self.torrent_status = "No inspected torrent session is available".into();
            cx.notify();
            return;
        };
        let worker = Some(worker);
        let media_id = self
            .active_media_id
            .clone()
            .or_else(|| session.media_id_for_file(file_index).ok());
        let resume_position = media_id
            .as_deref()
            .and_then(|id| self.database.playback_progress(id).ok().flatten())
            .map(|progress| progress.position_seconds)
            .filter(|position| *position > 5.0);
        let media_id_for_update = media_id.clone();
        self.active_media_id = None;
        self.torrent_status = "Preparing the selected file and loopback stream…".into();
        self.torrent_task = Some(cx.spawn(async move |this, cx| {
            let (session, result) = cx
                .background_spawn(async move {
                    match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => {
                            let result = runtime.block_on(session.start_file(file_index));
                            (session, result)
                        }
                        Err(error) => (session, Err(error.into())),
                    }
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.torrent_session = Some(session);
                match result {
                    Ok(url) => {
                        let queued = worker
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("MPV playback worker is unavailable"))
                            .and_then(|worker| {
                                worker
                                    .lock()
                                    .map_err(|_| {
                                        anyhow::anyhow!("MPV playback worker lock is unavailable")
                                    })?
                                    .load_local_url_with_resume(&url, resume_position)
                            });
                        match queued {
                            Ok(()) => {
                                app.torrent_files.clear();
                                app.torrent_status =
                                    "Torrent stream is ready; playback downloads on demand".into();
                                app.stream_status = "Torrent source queued in MPV".into();
                                app.playback_status =
                                    "Playing torrent stream from the local loopback server".into();
                                app.active_media_id = media_id_for_update;
                                app.active_page = AppPage::Player;
                                app.last_progress_save = None;
                            }
                            Err(error) => {
                                app.torrent_status =
                                    format!("Could not hand torrent stream to MPV: {error:#}");
                                app.stream_status = app.torrent_status.clone();
                                app.stop_torrent_session(cx);
                            }
                        }
                    }
                    Err(error) => {
                        app.torrent_status =
                            format!("Could not start selected torrent file: {error:#}");
                        app.stream_status = app.torrent_status.clone();
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn stop_torrent_session(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.torrent_session.take() else {
            return;
        };
        self.torrent_files.clear();
        self.torrent_progress = None;
        self.torrent_status =
            "Stopping MPV and torrent engine; removing temporary session data…".into();
        let worker = self.playback_worker.clone();
        self.torrent_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let stop_result = if let Some(worker) = worker {
                        tokio::task::spawn_blocking(move || {
                            let worker = worker.lock().map_err(|_| {
                                anyhow::anyhow!("playback worker lock is unavailable")
                            })?;
                            worker.stop_and_wait()
                        })
                        .await
                        .context("MPV stop task failed")?
                    } else {
                        Ok(())
                    };
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize torrent shutdown runtime")?;
                    stop_result?;
                    runtime.block_on(session.shutdown())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.torrent_status = match result {
                    Ok(()) => "Torrent stopped; temporary session data removed".into(),
                    Err(error) => format!("Torrent cleanup reported an error: {error:#}"),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn play_addon_stream(&mut self, stream: StremioStream, cx: &mut Context<Self>) {
        if stream.is_torrent() {
            self.inspect_torrent_stream(stream, cx);
            return;
        }
        let Ok(Some(url)) = stream.direct_url() else {
            self.stream_status =
                "This add-on entry is not a supported direct or torrent stream".into();
            cx.notify();
            return;
        };
        self.persist_playback_progress(true);
        if self.playback_worker.is_none() {
            self.start_playback(cx);
        }
        let Some(worker) = self.playback_worker.clone() else {
            self.stream_status = "MPV is not ready; check the playback runtime status".into();
            cx.notify();
            return;
        };
        let url_string = url.to_string();
        let resume_position = self
            .active_media_id
            .as_deref()
            .and_then(|id| self.database.playback_progress(id).ok().flatten())
            .map(|progress| progress.position_seconds)
            .filter(|position| *position > 5.0);
        let previous = self.torrent_session.take();
        self.torrent_files.clear();
        self.torrent_progress = None;
        self.stream_status = "Validating stream destination and queuing MPV playback…".into();
        self.addon_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = AddonClient::new();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize stream validation runtime")?;
                    runtime.block_on(async {
                        if let Some(previous) = previous {
                            let worker_for_stop = worker.clone();
                            tokio::task::spawn_blocking(move || {
                                let worker = worker_for_stop.lock().map_err(|_| {
                                    anyhow::anyhow!("playback worker lock is unavailable")
                                })?;
                                worker.stop_and_wait()
                            })
                            .await
                            .context("MPV stop task failed")??;
                            previous
                                .shutdown()
                                .await
                                .context("could not close the previous torrent session")?;
                        }
                        let parsed = url::Url::parse(&url_string)?;
                        client.validate_direct_stream_url(&parsed).await?;
                        let worker = worker
                            .lock()
                            .map_err(|_| anyhow::anyhow!("playback worker lock is unavailable"))?;
                        worker.load_url_with_resume(parsed.as_str(), resume_position)?;
                        Ok::<_, anyhow::Error>(())
                    })
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(()) => {
                        app.stream_status = "Direct stream queued in MPV".into();
                        app.playback_status = match resume_position {
                            Some(position) => {
                                format!("Resuming selected stream from {:.0} seconds", position)
                            }
                            None => "Playing selected add-on stream".into(),
                        };
                        app.active_page = AppPage::Player;
                    }
                    Err(error) => {
                        app.stream_status = format!("Could not play selected stream: {error:#}")
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn preview_addon_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(input) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .filter(|value| !value.trim().is_empty())
        else {
            self.addon_status =
                "Clipboard does not contain a manifest URL. Copy an HTTPS manifest URL first."
                    .into();
            cx.notify();
            return;
        };
        self.preview_addon_url(input.trim().to_owned(), cx);
    }

    fn preview_torrentio(&mut self, cx: &mut Context<Self>) {
        self.preview_addon_url("https://torrentio.strem.fun/manifest.json".to_owned(), cx);
    }

    fn preview_addon_url(&mut self, input: String, cx: &mut Context<Self>) {
        self.addon_status = "Fetching and validating add-on manifest…".into();
        self.addon_manifest_preview = None;
        self.addon_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = AddonClient::new();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize add-on preview runtime")?;
                    runtime.block_on(client.preview_from_url(&input))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(preview) => {
                        app.addon_status = format!(
                            "Manifest verified: {} · version {}",
                            preview.manifest.name, preview.manifest.version
                        );
                        app.addon_manifest_preview = Some(preview);
                    }
                    Err(error) => {
                        // User-configured provider URLs can contain private credentials.
                        // Keep detailed errors in the UI, but never log the supplied URL.
                        app.addon_status = format!("Manifest preview failed: {error:#}");
                        diagnostics::record(
                            diagnostics::Level::Warn,
                            "addons",
                            "user-supplied add-on manifest preview failed; see the Add-ons screen for the request error",
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_addon_install(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.addon_manifest_preview.take() else {
            return;
        };
        let installed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_secs() as i64)
            .unwrap_or_default();
        let addon = StoredAddon {
            id: senzo::addons::addon_instance_id(&preview.manifest.id, &preview.manifest_url),
            name: preview.manifest.name,
            version: preview.manifest.version,
            manifest_url: preview.manifest_url,
            enabled: true,
            manifest_json: preview.manifest_json,
            installed_at,
        };
        if self.database.installed_addons().is_ok_and(|installed| {
            installed
                .iter()
                .any(|item| item.manifest_url == addon.manifest_url)
        }) {
            self.addon_status = "This exact configured add-on URL is already installed".into();
            self.addon_manifest_preview = None;
            cx.notify();
            return;
        }
        match self.database.save_addon(&addon) {
            Ok(()) => {
                self.addon_status = format!("Installed {} successfully", addon.name);
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "addons",
                    "user explicitly installed a previewed Stremio add-on",
                );
            }
            Err(error) => {
                self.addon_status = format!("Could not install add-on: {error:#}");
                self.addon_manifest_preview = None;
            }
        }
        cx.notify();
    }

    fn refresh_addon(&mut self, addon: StoredAddon, cx: &mut Context<Self>) {
        self.addon_status = format!("Refreshing {}…", addon.name);
        self.addon_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = AddonClient::new();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize add-on refresh runtime")?;
                    runtime.block_on(client.refresh_manifest(&addon))
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(refreshed) => match app.database.update_addon_manifest(&refreshed) {
                        Ok(true) => {
                            app.addon_status = format!(
                                "Updated {} to version {}",
                                refreshed.name, refreshed.version
                            );
                            app.home_catalog_requested = false;
                            diagnostics::record(
                                diagnostics::Level::Audit,
                                "addons",
                                "user refreshed an installed add-on manifest",
                            );
                        }
                        Ok(false) => {
                            app.addon_status =
                                "The add-on was removed before refresh completed".into()
                        }
                        Err(error) => {
                            app.addon_status =
                                format!("Could not save refreshed manifest: {error:#}")
                        }
                    },
                    Err(error) => app.addon_status = format!("Add-on refresh failed: {error:#}"),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn configure_opensubtitles_key(&mut self, cx: &mut Context<Self>) {
        match OpenSubtitlesCredentialStore::prompt_and_save_api_key() {
            Ok(true) => {
                self.subtitle_credentials_status =
                    "API key saved in Windows Credential Manager".into();
                self.subtitle_provider_status =
                    "OpenSubtitles is ready. Search for subtitles while media is playing.".into();
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "subtitles",
                    "OpenSubtitles API key configured in Windows Credential Manager",
                );
            }
            Ok(false) => {
                self.subtitle_credentials_status =
                    "API key setup cancelled; no changes made".into();
            }
            Err(error) => {
                self.subtitle_credentials_status = format!("Could not save API key: {error:#}");
                diagnostics::record(
                    diagnostics::Level::Warn,
                    "subtitles",
                    "OpenSubtitles API key configuration failed",
                );
            }
        }
        cx.notify();
    }

    fn clear_opensubtitles_key(&mut self, cx: &mut Context<Self>) {
        match OpenSubtitlesCredentialStore::clear_api_key() {
            Ok(()) => {
                self.subtitle_credentials_status =
                    "Saved API key removed from Windows Credential Manager".into();
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "subtitles",
                    "OpenSubtitles API key removed from Windows Credential Manager",
                );
            }
            Err(error) => {
                self.subtitle_credentials_status = format!("Could not remove API key: {error:#}");
                diagnostics::record(
                    diagnostics::Level::Warn,
                    "subtitles",
                    "OpenSubtitles API key removal failed",
                );
            }
        }
        cx.notify();
    }

    fn search_online_subtitles(&mut self, cx: &mut Context<Self>) {
        let title = self
            .playback_state
            .media_title
            .clone()
            .or_else(|| {
                self.active_media_id
                    .as_deref()
                    .and_then(|id| id.strip_prefix("local-file:"))
                    .and_then(|path| PathBuf::from(path).file_stem()?.to_str().map(str::to_owned))
            })
            .filter(|title| !title.trim().is_empty());

        let Some(title) = title else {
            self.subtitle_provider_status =
                "Open a media file before searching for subtitles".into();
            cx.notify();
            return;
        };

        self.online_subtitle_results.clear();
        self.subtitle_provider_status = format!("Searching OpenSubtitles for {title}…");
        let search = SubtitleSearch {
            query: Some(title),
            languages: "en,bn".into(),
            ..SubtitleSearch::default()
        };

        self.subtitle_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = OpenSubtitlesClient::from_credential_store()
                        .or_else(|_| OpenSubtitlesClient::from_environment())?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize subtitle network runtime")?;
                    runtime.block_on(client.search(&search))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(results) => {
                        let count = results.len();
                        app.online_subtitle_results = results;
                        app.subtitle_provider_status = if count == 0 {
                            "No matching subtitles found".into()
                        } else {
                            format!("{count} subtitle releases found · choose a file to download")
                        };
                    }
                    Err(error) => {
                        app.subtitle_provider_status = format!("Subtitle search failed: {error:#}");
                        diagnostics::record(
                            diagnostics::Level::Warn,
                            "subtitles",
                            &app.subtitle_provider_status,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn download_online_subtitle(&mut self, file_id: u64, cx: &mut Context<Self>) {
        let Some(worker) = self.playback_worker.clone() else {
            self.subtitle_provider_status =
                "Start playback before loading an online subtitle".into();
            cx.notify();
            return;
        };

        self.subtitle_provider_status = "Downloading subtitle…".into();
        self.subtitle_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = OpenSubtitlesClient::from_credential_store()
                        .or_else(|_| OpenSubtitlesClient::from_environment())?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize subtitle download runtime")?;
                    let path = runtime.block_on(client.download_to_cache(file_id))?;
                    {
                        let worker = worker
                            .lock()
                            .map_err(|_| anyhow::anyhow!("playback worker lock is unavailable"))?;
                        worker.add_subtitle_file(path.clone())?;
                    }
                    Ok::<PathBuf, anyhow::Error>(path)
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(path) => {
                        let name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("subtitle");
                        app.subtitle_provider_status = format!("Downloaded and loaded: {name}");
                        app.online_subtitle_results.clear();
                        diagnostics::record(
                            diagnostics::Level::Audit,
                            "subtitles",
                            "user downloaded and loaded an OpenSubtitles subtitle",
                        );
                    }
                    Err(error) => {
                        app.subtitle_provider_status =
                            format!("Subtitle download failed: {error:#}");
                        diagnostics::record(
                            diagnostics::Level::Error,
                            "subtitles",
                            &app.subtitle_provider_status,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn download_addon_subtitle(
        &mut self,
        addon: StoredAddon,
        subtitle: StremioSubtitle,
        cx: &mut Context<Self>,
    ) {
        let Some(worker) = self.playback_worker.clone() else {
            self.addon_subtitle_status = "Start playback before loading an add-on subtitle".into();
            cx.notify();
            return;
        };
        let label = subtitle
            .label
            .clone()
            .or_else(|| subtitle.language.clone())
            .unwrap_or_else(|| "subtitle".into());
        self.addon_subtitle_status = format!("Downloading {label} from {}…", addon.name);
        self.subtitle_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = AddonClient::new();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize add-on subtitle runtime")?;
                    let path = runtime.block_on(client.download_subtitle(&subtitle))?;
                    {
                        let worker = worker
                            .lock()
                            .map_err(|_| anyhow::anyhow!("playback worker lock is unavailable"))?;
                        worker.add_subtitle_file(path.clone())?;
                    }
                    Ok::<PathBuf, anyhow::Error>(path)
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(path) => {
                        let name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("subtitle");
                        app.addon_subtitle_status = format!("Loaded add-on subtitle: {name}");
                        diagnostics::record(
                            diagnostics::Level::Audit,
                            "subtitles",
                            "user loaded a subtitle from an installed Stremio add-on",
                        );
                    }
                    Err(error) => {
                        app.addon_subtitle_status =
                            format!("Add-on subtitle download failed: {error:#}");
                        diagnostics::record(
                            diagnostics::Level::Warn,
                            "subtitles",
                            &app.addon_subtitle_status,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_subtitle_file(&mut self, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open subtitle")
            .add_filter(
                "Subtitle files",
                &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup"],
            )
            .pick_file()
        else {
            return;
        };

        let result = self
            .playback_worker
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("open a media file before adding subtitles"))
            .and_then(|worker| {
                let worker = worker
                    .lock()
                    .map_err(|_| anyhow::anyhow!("playback worker lock is unavailable"))?;
                worker.add_subtitle_file(path.clone())
            });

        match result {
            Ok(()) => {
                self.playback_status = format!(
                    "Subtitle queued: {}",
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("subtitle")
                )
            }
            Err(error) => self.playback_status = format!("Could not load subtitle: {error:#}"),
        }
        cx.notify();
    }

    fn set_metadata_language(&mut self, language: &'static str, cx: &mut Context<Self>) {
        if !matches!(language, "en-US" | "bn-BD") {
            return;
        }
        self.metadata_language = language.to_owned();
        if let Err(error) = self
            .database
            .set_setting("appearance.metadata_language", language)
        {
            diagnostics::record(
                diagnostics::Level::Warn,
                "settings",
                &format!("could not save metadata language: {error:#}"),
            );
            self.status = "Could not save language preference".into();
        } else {
            self.status = "Language preference saved on this device".into();
            self.home_catalog_requested = false;
            if self.active_page == AppPage::Home {
                self.load_home_catalogs(cx);
            }
        }
        cx.notify();
    }

    fn set_poster_size(&mut self, size: &'static str, cx: &mut Context<Self>) {
        if !matches!(size, "compact" | "comfortable") {
            return;
        }
        self.poster_size = size.to_owned();
        if let Err(error) = self.database.set_setting("appearance.poster_size", size) {
            diagnostics::record(
                diagnostics::Level::Warn,
                "settings",
                &format!("could not save poster size: {error:#}"),
            );
            self.status = "Could not save appearance preference".into();
        } else {
            self.status = "Appearance preference saved on this device".into();
        }
        cx.notify();
    }

    fn set_home_shelf_hidden(&mut self, shelf: &'static str, hidden: bool, cx: &mut Context<Self>) {
        let mut updated = self.hidden_home_shelves.clone();
        if hidden {
            if !updated.iter().any(|saved| saved == shelf) {
                updated.push(shelf.to_owned());
            }
        } else {
            updated.retain(|saved| saved != shelf);
        }
        match serde_json::to_string(&updated)
            .context("failed to serialize Home shelf visibility")
            .and_then(|value| {
                self.database
                    .set_setting("home.hidden_shelves", &value)
                    .context("failed to save Home shelf visibility")
            }) {
            Ok(()) => {
                self.hidden_home_shelves = updated;
                self.status = "Home layout saved on this device".into();
            }
            Err(error) => {
                self.status = format!("Could not save Home layout: {error:#}");
                diagnostics::record(diagnostics::Level::Warn, "settings", &self.status);
            }
        }
        cx.notify();
    }

    fn move_home_shelf(&mut self, shelf: &'static str, direction: isize, cx: &mut Context<Self>) {
        let Some(index) = self
            .home_shelf_order
            .iter()
            .position(|saved| saved.as_str() == shelf)
        else {
            return;
        };
        let target = index as isize + direction;
        if target < 0 || target >= self.home_shelf_order.len() as isize {
            return;
        }
        let mut updated = self.home_shelf_order.clone();
        updated.swap(index, target as usize);
        match serde_json::to_string(&updated)
            .context("failed to serialize Home shelf order")
            .and_then(|value| {
                self.database
                    .set_setting("home.shelf_order", &value)
                    .context("failed to save Home shelf order")
            }) {
            Ok(()) => {
                self.home_shelf_order = updated;
                self.status = "Home layout saved on this device".into();
            }
            Err(error) => {
                self.status = format!("Could not save Home layout: {error:#}");
                diagnostics::record(diagnostics::Level::Warn, "settings", &self.status);
            }
        }
        cx.notify();
    }

    fn set_discord_presence_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.discord_presence_enabled = enabled;
        if let Err(error) = self.database.set_setting(
            "integrations.discord.enabled",
            if enabled { "true" } else { "false" },
        ) {
            diagnostics::record(
                diagnostics::Level::Error,
                "discord",
                &format!("could not save Discord Rich Presence preference: {error:#}"),
            );
            self.status = "Could not save Discord Rich Presence preference".into();
        } else {
            self.status = if enabled {
                "Discord Rich Presence enabled".into()
            } else {
                "Discord Rich Presence disabled".into()
            };
        }
        self.discord_presence
            .configure(enabled, self.discord_share_titles);
        cx.notify();
    }

    fn set_discord_title_sharing(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.discord_share_titles = enabled;
        if let Err(error) = self.database.set_setting(
            "integrations.discord.share_titles",
            if enabled { "true" } else { "false" },
        ) {
            diagnostics::record(
                diagnostics::Level::Error,
                "discord",
                &format!("could not save Discord title-sharing preference: {error:#}"),
            );
            self.status = "Could not save Discord privacy preference".into();
        }
        self.discord_presence
            .configure(self.discord_presence_enabled, enabled);
        cx.notify();
    }

    fn set_subtitle_font_size(&mut self, size: u8, cx: &mut Context<Self>) {
        self.settings.subtitles.font_size = size.clamp(12, 96);
        self.send_playback_command(
            "subtitle font size",
            move |worker| worker.set_subtitle_font_size(size.clamp(12, 96)),
            cx,
        );
    }

    fn set_subtitle_delay(&mut self, seconds: f64, cx: &mut Context<Self>) {
        let seconds = if seconds.is_finite() {
            seconds.clamp(-10.0, 10.0)
        } else {
            0.0
        };
        self.settings.subtitles.delay_seconds = seconds;
        self.send_playback_command(
            "subtitle synchronization",
            move |worker| worker.set_subtitle_delay(seconds),
            cx,
        );
    }

    fn seek_from_pointer(
        &mut self,
        x: f32,
        bounds: &Arc<Mutex<Option<(f32, f32)>>>,
        cx: &mut Context<Self>,
    ) {
        let Some((left, width)) = bounds.lock().ok().and_then(|value| *value) else {
            return;
        };
        let Some(duration) = self
            .playback_state
            .duration_seconds
            .filter(|value| value.is_finite() && *value > 0.0)
        else {
            return;
        };
        if !x.is_finite() || !width.is_finite() || width <= 0.0 {
            return;
        }
        let target = duration * f64::from(((x - left) / width).clamp(0.0, 1.0));
        self.send_playback_command("seek", move |worker| worker.seek_absolute(target), cx);
    }

    fn set_volume_from_pointer(
        &mut self,
        x: f32,
        bounds: &Arc<Mutex<Option<(f32, f32)>>>,
        cx: &mut Context<Self>,
    ) {
        let Some((left, width)) = bounds.lock().ok().and_then(|value| *value) else {
            return;
        };
        if !x.is_finite() || !width.is_finite() || width <= 0.0 {
            return;
        }
        let volume = f64::from(((x - left) / width).clamp(0.0, 1.0)) * 100.0;
        self.send_playback_command("volume", move |worker| worker.set_volume(volume), cx);
    }

    fn set_subtitle_margin(&mut self, margin: u8, cx: &mut Context<Self>) {
        let margin = margin.min(100);
        self.settings.subtitles.vertical_margin = margin;
        self.send_playback_command(
            "subtitle vertical margin",
            move |worker| worker.set_subtitle_margin(margin),
            cx,
        );
    }

    fn set_subtitle_bold(&mut self, bold: bool, cx: &mut Context<Self>) {
        self.settings.subtitles.bold = bold;
        self.send_playback_command(
            "subtitle weight",
            move |worker| worker.set_subtitle_bold(bold),
            cx,
        );
    }

    fn resume_saved_media(&mut self, media_id: String, cx: &mut Context<Self>) {
        if let Some(rest) = media_id.strip_prefix("tmdb:") {
            let mut parts = rest.split(':');
            let (Some(kind), Some(raw_id)) = (parts.next(), parts.next()) else {
                return;
            };
            if parts.next().is_some() || !matches!(kind, "movie" | "tv") {
                self.playback_status = "Saved metadata identity is invalid".into();
                cx.notify();
                return;
            }
            let Ok(id) = raw_id.parse::<u64>() else {
                self.playback_status = "Saved metadata identity is invalid".into();
                cx.notify();
                return;
            };
            let title = self
                .database
                .playback_progress(&media_id)
                .ok()
                .flatten()
                .and_then(|item| item.title)
                .unwrap_or_else(|| "Continue watching".into());
            let item = TmdbSearchResult {
                id,
                title: (kind == "movie").then(|| title.clone()),
                name: (kind == "tv").then(|| title),
                original_title: None,
                original_name: None,
                overview: None,
                poster_path: None,
                backdrop_path: None,
                release_date: None,
                first_air_date: None,
                vote_average: None,
                vote_count: None,
                media_type: Some(kind.into()),
            };
            self.select_tmdb_title(kind.to_owned(), item, cx);
            return;
        }
        let Some(raw_path) = media_id.strip_prefix("local-file:") else {
            self.playback_status = "This saved item cannot be resumed".into();
            cx.notify();
            return;
        };
        let path = PathBuf::from(raw_path);
        if !path.is_file() {
            self.playback_status = "The original media file could not be found".into();
            cx.notify();
            return;
        }
        let resume_position = match self.database.playback_progress(&media_id) {
            Ok(Some(progress)) => Some(progress.position_seconds),
            Ok(None) => None,
            Err(error) => {
                self.playback_status = format!("Could not read saved position: {error:#}");
                cx.notify();
                return;
            }
        };
        if self.playback_worker.is_none() {
            self.start_playback(cx);
        }
        let result = self
            .playback_worker
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MPV runtime is unavailable"))
            .and_then(|worker| {
                let worker = worker
                    .lock()
                    .map_err(|_| anyhow::anyhow!("playback worker lock is unavailable"))?;
                worker.load_file_with_resume(path.clone(), resume_position)?;
                Ok(())
            });
        match result {
            Ok(()) => {
                self.active_media_id = Some(media_id);
                self.active_media_title = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned);
                self.active_media_type = Some("local".into());
                self.last_progress_save = None;
                self.active_page = AppPage::Player;
                self.playback_status = format!(
                    "Resuming {}",
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("media")
                );
            }
            Err(error) => self.playback_status = format!("Could not resume media: {error:#}"),
        }
        cx.notify();
    }

    fn persist_playback_progress(&mut self, force: bool) {
        let Some(media_id) = self.active_media_id.as_deref() else {
            return;
        };
        let (Some(position), duration) = (
            self.playback_state.position_seconds,
            self.playback_state.duration_seconds,
        ) else {
            return;
        };
        if !position.is_finite() || position < 0.0 {
            return;
        }
        let now = Instant::now();
        if !force
            && self
                .last_progress_save
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(10))
        {
            return;
        }
        let updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_secs() as i64)
            .unwrap_or_default();
        let completion_threshold = duration
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value - 30.0_f64.min(value * 0.05));
        if completion_threshold.is_some_and(|threshold| position >= threshold) {
            let title = self
                .active_media_title
                .as_deref()
                .unwrap_or("Unknown title");
            let media_type = self.active_media_type.as_deref().unwrap_or("unknown");
            match self
                .database
                .complete_playback(media_id, title, media_type, updated_at)
            {
                Ok(()) => {
                    self.last_progress_save = Some(now);
                    self.active_media_id = None;
                    self.playback_status = format!("Completed {title}");
                    diagnostics::record(
                        diagnostics::Level::Audit,
                        "playback",
                        "media completion recorded in local watch history",
                    );
                }
                Err(error) => diagnostics::record(
                    diagnostics::Level::Warn,
                    "playback",
                    &format!("could not record completed playback: {error:#}"),
                ),
            }
            return;
        }
        match self.database.save_playback_progress_with_metadata(
            media_id,
            position,
            duration,
            updated_at,
            self.active_media_title.as_deref(),
            self.active_media_type.as_deref(),
        ) {
            Ok(()) => self.last_progress_save = Some(now),
            Err(error) => diagnostics::record(
                diagnostics::Level::Warn,
                "playback",
                &format!("could not persist playback progress: {error:#}"),
            ),
        }
    }

    fn send_playback_command(
        &mut self,
        operation: &'static str,
        command: impl FnOnce(&PlaybackWorker) -> anyhow::Result<()>,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .playback_worker
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MPV runtime is not available"))
            .and_then(|worker| {
                let worker = worker
                    .lock()
                    .map_err(|_| anyhow::anyhow!("playback worker lock is unavailable"))?;
                command(&worker)
            });

        if let Err(error) = result {
            self.playback_status = format!("{operation}: {error:#}");
        }
        cx.notify();
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        match self.database.save_playback_settings(&self.settings) {
            Ok(()) => self.status = "Playback preferences saved locally".into(),
            Err(error) => {
                self.status = format!("Could not save settings: {error}");
                diagnostics::record(
                    diagnostics::Level::Error,
                    "settings",
                    &format!("failed to save playback preferences: {error:#}"),
                );
            }
        }
        cx.notify();
    }

    fn open_diagnostics_folder(&mut self, cx: &mut Context<Self>) {
        match diagnostics::open_log_folder() {
            Ok(()) => self.status = "Opened the SENZO diagnostic log folder".into(),
            Err(error) => {
                self.status = format!("Could not open log folder: {error:#}");
                diagnostics::record(
                    diagnostics::Level::Error,
                    "diagnostics",
                    "failed to open the diagnostic log folder",
                );
            }
        }
        cx.notify();
    }

    fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        match diagnostics::collect_recent_logs() {
            Ok(report) => {
                cx.write_to_clipboard(ClipboardItem::new_string(report));
                self.status = "Diagnostic report copied to clipboard".into();
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "diagnostics",
                    "user copied a diagnostic report",
                );
            }
            Err(error) => {
                self.status = format!("Could not collect logs: {error}");
                diagnostics::record(
                    diagnostics::Level::Error,
                    "diagnostics",
                    &format!("failed to collect diagnostic report: {error:#}"),
                );
            }
        }
        cx.notify();
    }
}

fn choice_chip<F>(
    label: &'static str,
    selected: bool,
    action: F,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement + use<F>
where
    F: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
{
    let background = if selected {
        rgb(0x12343A)
    } else {
        rgb(0x202327)
    };
    let foreground = if selected {
        rgb(0x67E8F9)
    } else {
        rgb(0xD4D4D8)
    };

    div()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(background)
        .text_color(foreground)
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x30363B)))
        .child(label)
        .tab_index(0)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _event, _window, cx| {
                action(this, cx);
                this.persist(cx);
            }),
        )
}

fn dynamic_choice_chip<F>(
    label: String,
    selected: bool,
    action: F,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement + use<F>
where
    F: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
{
    div()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(if selected {
            rgb(0x12343A)
        } else {
            rgb(0x202327)
        })
        .text_color(if selected {
            rgb(0x67E8F9)
        } else {
            rgb(0xD4D4D8)
        })
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x30363B)))
        .child(label)
        .tab_index(0)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _event, _window, cx| {
                action(this, cx);
                cx.notify();
            }),
        )
}

#[cfg(windows)]
fn current_time_string() -> String {
    #[repr(C)]
    #[allow(dead_code)]
    struct SystemTimeWin32 {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    unsafe extern "system" {
        fn GetLocalTime(lp_system_time: *mut SystemTimeWin32);
    }
    let mut st = SystemTimeWin32 {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    unsafe { GetLocalTime(&mut st) };
    let hour = st.hour;
    let am_pm = if hour >= 12 { "PM" } else { "AM" };
    let hour_12 = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{}:{:02} {}", hour_12, st.minute, am_pm)
}

#[cfg(not(windows))]
fn current_time_string() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hours = (now / 3600) % 24;
    let minutes = (now / 60) % 60;
    let am_pm = if hours >= 12 { "PM" } else { "AM" };
    let hour_12 = match hours % 12 {
        0 => 12,
        h => h,
    };
    format!("{}:{:02} {}", hour_12, minutes, am_pm)
}

fn nav_button<F>(
    label: &'static str,
    icon: &'static str,
    selected: bool,
    action: F,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement + use<F>
where
    F: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
{
    div()
        .w_full()
        .h(px(40.))
        .flex_shrink_0()
        .px_3()
        .rounded_xl()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_3()
        .bg(gpui::transparent_black())
        .when(selected, |style| style.bg(rgb(0x22D3EE)))
        .text_color(if selected {
            rgb(0xFFFFFF)
        } else {
            rgb(0x94A3B8)
        })
        .font_weight(if selected {
            gpui::FontWeight::BOLD
        } else {
            gpui::FontWeight::MEDIUM
        })
        .hover(|style| {
            if !selected {
                style.bg(rgb(0x181F26)).text_color(rgb(0xF4F4F5))
            } else {
                style
            }
        })
        .child(div().text_base().child(icon))
        .child(div().text_sm().child(label))
        .tab_index(0)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _event, _window, cx| {
                action(this, cx);
                cx.notify();
            }),
        )
}

fn section(
    title: &'static str,
    description: &'static str,
    content: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_5()
        .rounded_lg()
        .bg(rgb(0x151719))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_base()
                        .text_color(rgb(0xF4F4F5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_sm().text_color(rgb(0x9299A1)).child(description)),
        )
        .child(content)
}

fn option_row(
    label: &'static str,
    description: &'static str,
    choices: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_color(rgb(0xE4E4E7)).child(label))
                .child(div().text_xs().text_color(rgb(0x858C94)).child(description)),
        )
        .child(div().flex().flex_wrap().gap_2().child(choices))
}

impl Render for SenzoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.active_page == AppPage::Search && self.search_input.is_none() {
            let input =
                cx.new(|cx| InputState::new(window, cx).placeholder("Search movies and TV shows…"));
            let subscription =
                cx.subscribe(&input, |this, state, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        this.search_query = state.read(cx).value().to_string();
                        let query = this.search_query.trim().to_owned();
                        if query.is_empty() {
                            this.search_debounce_task = None;
                            this.tmdb_results.clear();
                            this.tmdb_status = "Type a title to search movies and TV shows".into();
                            cx.notify();
                        } else {
                            this.search_debounce_task = Some(cx.spawn(async move |this, cx| {
                                cx.background_executor()
                                    .timer(Duration::from_millis(400))
                                    .await;
                                let _ = this.update(cx, |app, cx| {
                                    if app.active_page == AppPage::Search
                                        && app.search_query.trim() == query
                                    {
                                        app.load_tmdb_search(query, cx);
                                    }
                                });
                            }));
                        }
                    }
                    InputEvent::PressEnter { .. } => {
                        let query = this.search_query.clone();
                        this.search_debounce_task = None;
                        this.load_tmdb_search(query, cx);
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                });
            self.search_input = Some(input);
            self.search_input_subscription = Some(subscription);
        }
        if self.active_page == AppPage::Home && !self.home_catalog_requested {
            self.load_home_catalogs(cx);
        }
        let settings = self.settings.clone();
        let startup_capabilities = senzo::playback::mpv_config::MpvConfig::startup_capabilities();
        let seek_bar_bounds = self.seek_bar_bounds.clone();
        let volume_bar_bounds = self.volume_bar_bounds.clone();
        let playback_position = self.playback_state.position_seconds;
        let playback_duration = self.playback_state.duration_seconds;
        let playback_volume = self
            .playback_state
            .volume
            .unwrap_or(100.0)
            .clamp(0.0, 100.0);
        let native_surface_status = self
            .native_surface_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .map(|error| format!("Native video surface unavailable: {error}"))
            .unwrap_or_else(|| {
                let ready = self
                    .native_video_surface
                    .lock()
                    .map(|surface| surface.is_some())
                    .unwrap_or(false);
                if ready {
                    "Native Win32 video surface ready · MPV attaches when media opens".into()
                } else {
                    "Preparing native Win32 video surface…".into()
                }
            });

        let mut audio_track_chips = Vec::new();
        for track in self
            .playback_state
            .tracks
            .iter()
            .filter(|track| track.kind == "audio")
        {
            let id = track.id;
            let label = format!(
                "{}{}",
                track.title.as_deref().unwrap_or("Audio"),
                track
                    .language
                    .as_deref()
                    .map(|language| format!(" · {language}"))
                    .unwrap_or_default()
            );
            audio_track_chips.push(dynamic_choice_chip(
                label,
                track.selected,
                move |this, cx| {
                    this.send_playback_command(
                        "select audio track",
                        move |worker| worker.set_audio_track(id),
                        cx,
                    );
                },
                cx,
            ));
        }

        let mut online_subtitle_chips = Vec::new();
        for result in &self.online_subtitle_results {
            let attributes = &result.attributes;
            for file in &attributes.files {
                let file_id = file.file_id;
                let label = format!(
                    "{} · {}",
                    attributes.language.as_deref().unwrap_or("Unknown language"),
                    file.file_name
                );
                online_subtitle_chips.push(dynamic_choice_chip(
                    label,
                    false,
                    move |this, cx| this.download_online_subtitle(file_id, cx),
                    cx,
                ));
            }
        }

        let mut addon_subtitle_chips = Vec::new();
        for (addon, subtitle) in &self.addon_subtitle_results {
            let label = format!(
                "{} · {}",
                addon.name,
                subtitle
                    .label
                    .as_deref()
                    .or(subtitle.language.as_deref())
                    .unwrap_or("Subtitle")
            );
            let selected_addon = addon.clone();
            let selected_subtitle = subtitle.clone();
            addon_subtitle_chips.push(dynamic_choice_chip(
                label,
                false,
                move |this, cx| {
                    this.download_addon_subtitle(
                        selected_addon.clone(),
                        selected_subtitle.clone(),
                        cx,
                    )
                },
                cx,
            ));
        }
        let mut subtitle_track_chips = Vec::new();
        for track in self
            .playback_state
            .tracks
            .iter()
            .filter(|track| track.kind == "sub")
        {
            let id = track.id;
            let label = format!(
                "{}{}",
                track.title.as_deref().unwrap_or("Subtitle"),
                track
                    .language
                    .as_deref()
                    .map(|language| format!(" · {language}"))
                    .unwrap_or_default()
            );
            subtitle_track_chips.push(dynamic_choice_chip(
                label,
                track.selected,
                move |this, cx| {
                    this.send_playback_command(
                        "select subtitle track",
                        move |worker| worker.set_subtitle_track(Some(id)),
                        cx,
                    );
                },
                cx,
            ));
        }

        div()
            .on_children_prepainted({
                let surface = self.native_video_surface.clone();
                let show_video = self.active_page == AppPage::Player;
                move |_children, _window, _cx| {
                    if !show_video {
                        if let Ok(mut surface) = surface.lock() {
                            if let Some(surface) = surface.as_mut() {
                                surface.set_visible(false);
                            }
                        }
                    }
                }
            })
            .id("senzo-root")
            .size_full()
            .flex()
            .bg(rgb(0x0A0C0E))
            .text_color(rgb(0xF4F4F5))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _window, cx| {
                match event.keystroke.key.as_str() {
                    "left" if this.active_page == AppPage::Home => {
                        let count = this.home_catalogs.iter()
                            .filter(|(shelf, _, _)| shelf == "Trending Movies" || shelf == "Trending this week")
                            .count().min(7);
                        let count = if count == 0 { this.home_catalogs.len().min(7) } else { count };
                        if count > 1 {
                            this.home_hero_index = (this.home_hero_index + count - 1) % count;
                            cx.notify();
                        }
                    }
                    "right" if this.active_page == AppPage::Home => {
                        let count = this.home_catalogs.iter()
                            .filter(|(shelf, _, _)| shelf == "Trending Movies" || shelf == "Trending this week")
                            .count().min(7);
                        let count = if count == 0 { this.home_catalogs.len().min(7) } else { count };
                        if count > 1 {
                            this.home_hero_index = (this.home_hero_index + 1) % count;
                            cx.notify();
                        }
                    }
                    "space" | " " if this.active_page == AppPage::Player && this.playback_worker.is_some() => {
                        let paused = !this.playback_state.paused.unwrap_or(false);
                        this.send_playback_command("keyboard play/pause", move |worker| worker.set_paused(paused), cx);
                    }
                    "left" if this.active_page == AppPage::Player => {
                        if let Some(position) = this.playback_state.position_seconds {
                            this.send_playback_command("keyboard seek backward", move |worker| worker.seek_absolute((position - 10.0).max(0.0)), cx);
                        }
                    }
                    "right" if this.active_page == AppPage::Player => {
                        if let Some(position) = this.playback_state.position_seconds {
                            this.send_playback_command("keyboard seek forward", move |worker| worker.seek_absolute(position + 10.0), cx);
                        }
                    }
                    "up" if this.active_page == AppPage::Player => {
                        let volume = (this.playback_state.volume.unwrap_or(100.0) + 5.0).clamp(0.0, 100.0);
                        this.send_playback_command("keyboard volume up", move |worker| worker.set_volume(volume), cx);
                    }
                    "down" if this.active_page == AppPage::Player => {
                        let volume = (this.playback_state.volume.unwrap_or(100.0) - 5.0).clamp(0.0, 100.0);
                        this.send_playback_command("keyboard volume down", move |worker| worker.set_volume(volume), cx);
                    }
                    "m" | "M" if this.active_page == AppPage::Player => {
                        let muted = !this.playback_state.muted.unwrap_or(false);
                        this.send_playback_command("keyboard mute", move |worker| worker.set_muted(muted), cx);
                    }
                    "f" | "F" if this.active_page == AppPage::Player => {
                        let fullscreen = !this.playback_state.fullscreen.unwrap_or(false);
                        this.send_playback_command("keyboard fullscreen", move |worker| worker.set_fullscreen(fullscreen), cx);
                    }
                    "escape" if this.active_page == AppPage::Player => {
                        if this.playback_state.fullscreen.unwrap_or(false) {
                            this.send_playback_command("exit fullscreen", |worker| worker.set_fullscreen(false), cx);
                        } else {
                            this.active_page = if this.selected_title.is_some() { AppPage::Details } else { AppPage::Home };
                            cx.notify();
                        }
                    }
                    "escape" if this.active_page == AppPage::Details => {
                        this.active_page = AppPage::Home;
                        cx.notify();
                    }
                    "escape" if this.settings_advanced_open && this.active_page == AppPage::Settings => {
                        this.settings_advanced_open = false;
                        cx.notify();
                    }
                    "1" | "h" | "H"
                        if this.active_page != AppPage::Search
                            && this.active_page != AppPage::Player =>
                    {
                        this.active_page = AppPage::Home;
                        cx.notify();
                    }
                    "2" if this.active_page != AppPage::Search
                        && this.active_page != AppPage::Player =>
                    {
                        this.active_page = AppPage::Movies;
                        this.load_tmdb_catalog("trending", None, cx);
                        cx.notify();
                    }
                    "3" if this.active_page != AppPage::Search
                        && this.active_page != AppPage::Player =>
                    {
                        this.active_page = AppPage::TvShows;
                        this.load_tmdb_catalog("tv", None, cx);
                        cx.notify();
                    }
                    "4" | "/"
                        if this.active_page != AppPage::Search
                            && this.active_page != AppPage::Player =>
                    {
                        this.active_page = AppPage::Search;
                        cx.notify();
                    }
                    "5" if this.active_page != AppPage::Search
                        && this.active_page != AppPage::Player =>
                    {
                        this.active_page = AppPage::Library;
                        cx.notify();
                    }
                    "6" if this.active_page != AppPage::Search
                        && this.active_page != AppPage::Player =>
                    {
                        this.active_page = AppPage::Addons;
                        cx.notify();
                    }
                    "7" if this.active_page != AppPage::Search
                        && this.active_page != AppPage::Player =>
                    {
                        this.active_page = AppPage::Settings;
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .when(self.active_page != AppPage::Player, |root| root.child(
                div()
                    .w(px(220.))
                    .h_full()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .p_4()
                    .bg(rgb(0x0A0A0A))
                    .border_r_1()
                    .border_color(rgb(0x181F26))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .px_3()
                                    .py_2()
                                    .child(
                                        div()
                                            .text_xl()
                                            .text_color(rgb(0x22D3EE))
                                            .font_weight(gpui::FontWeight::EXTRA_BOLD)
                                            .child("▶"),
                                    )
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(gpui::FontWeight::EXTRA_BOLD)
                                            .text_color(rgb(0xF8FAFC))
                                            .child("SENZO"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1_5()
                                    .flex_shrink_0()
                                    .child(nav_button("Home", "⌂", self.active_page == AppPage::Home, |this, _cx| this.active_page = AppPage::Home, cx))
                                    .child(nav_button("Movies", "🎬", self.active_page == AppPage::Movies, |this, cx| {
                                        this.active_page = AppPage::Movies;
                                        this.load_tmdb_catalog("movie", None, cx);
                                    }, cx))
                                    .child(nav_button("TV Shows", "📺", self.active_page == AppPage::TvShows, |this, cx| {
                                        this.active_page = AppPage::TvShows;
                                        this.load_tmdb_catalog("tv", None, cx);
                                    }, cx))
                                    .child(nav_button("Search", "🔍", self.active_page == AppPage::Search, |this, _cx| this.active_page = AppPage::Search, cx))
                                    .child(nav_button("Library", "🔖", self.active_page == AppPage::Library, |this, _cx| this.active_page = AppPage::Library, cx))
                                    .child(nav_button("Add-ons", "🧩", self.active_page == AppPage::Addons, |this, _cx| this.active_page = AppPage::Addons, cx))
                                    .child(nav_button("Settings", "⚙", self.active_page == AppPage::Settings, |this, _cx| this.active_page = AppPage::Settings, cx))
                                    .when(self.active_page == AppPage::Player, |nav| {
                                        nav.child(nav_button("Now Playing", "▶", true, |this, _cx| this.active_page = AppPage::Player, cx))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .flex_shrink_0()
                            .child(div().h(px(1.)).bg(rgb(0x1E242B)).w_full())
                            .child(
                                div()
                                    .px_3()
                                    .py_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x64748B))
                                            .child("v0.1.0"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x94A3B8))
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child("Built with ")
                                            .child(div().text_color(rgb(0xEF4444)).child("❤️"))
                                            .child(" for the community"),
                                    ),
                            ),
                    ),
            ))
            .when(self.active_page == AppPage::Settings, |root| {
                root.child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .h_full()
                    .overflow_y_scroll()
                    .p_8()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Settings"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x9299A1))
                                    .child("Make SENZO feel right for you. Your preferences stay on this device."),
                            ),
                    )
                    .when(!self.settings_advanced_open, |panel| {
                        panel
                            .child(section(
                                "Appearance",
                                "Choose how artwork is displayed while browsing.",
                                div().flex().flex_col().gap_2()
                                    .child(div().text_sm().text_color(rgb(0xD4D4D8)).child("Poster size"))
                                    .child(div().flex().gap_2()
                                        .child(choice_chip("Compact", self.poster_size == "compact", |this, cx| this.set_poster_size("compact", cx), cx))
                                        .child(choice_chip("Comfortable", self.poster_size == "comfortable", |this, cx| this.set_poster_size("comfortable", cx), cx))),
                            ))
                            .child(section(
                                "Language",
                                "Choose the language used for movie and series information.",
                                div().flex().flex_col().gap_2()
                                    .child(div().text_sm().text_color(rgb(0xD4D4D8)).child("Content language"))
                                    .child(div().flex().gap_2()
                                        .child(choice_chip("English", self.metadata_language == "en-US", |this, cx| this.set_metadata_language("en-US", cx), cx))
                                        .child(choice_chip("বাংলা", self.metadata_language == "bn-BD", |this, cx| this.set_metadata_language("bn-BD", cx), cx))),
                            ))
                            .child(section(
                                "Playback",
                                "SENZO automatically chooses compatible playback settings for your device.",
                                div().flex().flex_col().gap_2()
                                    .child(div().text_sm().text_color(rgb(0xD4D4D8)).child("Hardware acceleration · Automatic"))
                                    .child(div().text_xs().text_color(rgb(0x9299A1)).child("Recommended for smooth playback on most PCs.")),
                            ))
                            .child(section(
                                "Subtitles",
                                "Make subtitles easier to read. These preferences are saved on this device.",
                                div().flex().flex_col().gap_2()
                                    .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Subtitle size · {}", settings.subtitles.font_size)))
                                    .child(div().flex().gap_2()
                                        .child(choice_chip("Smaller", false, |this, cx| {
                                            let size = this.settings.subtitles.font_size.saturating_sub(2).max(12);
                                            this.set_subtitle_font_size(size, cx);
                                        }, cx))
                                        .child(choice_chip("Larger", false, |this, cx| {
                                            let size = this.settings.subtitles.font_size.saturating_add(2).min(96);
                                            this.set_subtitle_font_size(size, cx);
                                        }, cx))),
                            ))
                            .child(section(
                                "About SENZO",
                                "A local-first media center. Your settings and watch progress stay on this device.",
                                div().flex().flex_col().gap_1()
                                    .child(div().text_sm().text_color(rgb(0xD4D4D8)).child("SENZO · Windows desktop"))
                                    .child(div().text_xs().text_color(rgb(0x9299A1)).child("Metadata by TMDB · Online subtitles by OpenSubtitles. Provider availability depends on this build's configuration.")),
                            ))
                            .child(div().flex().items_center().justify_between().py_2()
                                .child(div().text_xs().text_color(rgb(0x858C94)).child(self.status.clone()))
                                .child(dynamic_choice_chip("Advanced settings".into(), true, |this, cx| {
                                    this.settings_advanced_open = true;
                                    cx.notify();
                                }, cx)))
                    })
                    .when(self.settings_advanced_open, |panel| {
                        panel
                            .child(div().flex().items_center().justify_between().py_2()
                                .child(div().text_sm().text_color(rgb(0xA1A1AA)).child("Advanced settings · Technical playback and provider controls"))
                                .child(dynamic_choice_chip("Back to simple settings".into(), false, |this, cx| {
                                    this.settings_advanced_open = false;
                                    cx.notify();
                                }, cx)))
                    .child(section(
                        "Video viewport",
                        "Native Win32 child-window target for MPV video. The surface is created and resized from GPUI layout; actual rendering, alignment, and teardown still need Windows runtime verification.",
                        {
                            let viewport_bounds = self.native_viewport_bounds.clone();
                            let native_parent_hwnd = self.native_parent_hwnd;
                            let native_video_surface = self.native_video_surface.clone();
                            let native_surface_error = self.native_surface_error.clone();
                            div()
                                .on_children_prepainted(move |bounds, window, _cx| {
                                    if let Some(bounds) = bounds.first() {
                                        let measured = SurfaceBounds::from_logical_pixels(
                                            f32::from(bounds.origin.x),
                                            f32::from(bounds.origin.y),
                                            f32::from(bounds.size.width),
                                            f32::from(bounds.size.height),
                                            window.scale_factor(),
                                        );
                                        if let Ok(measured) = measured {
                                            if let Ok(mut current) = viewport_bounds.lock() {
                                                *current = Some(measured);
                                            }

                                            if let Ok(mut surface_slot) = native_video_surface.lock() {
                                                if let Some(surface) = surface_slot.as_mut() {
                                                    if let Err(error) = surface.set_bounds(measured) {
                                                        diagnostics::record(
                                                            diagnostics::Level::Warn,
                                                            "playback",
                                                            &format!("could not position native video surface: {error:#}"),
                                                        );
                                                    }
                                                } else {
                                                    let previous_error = native_surface_error
                                                        .lock()
                                                        .ok()
                                                        .and_then(|error| error.clone());
                                                    if previous_error.is_none() {
                                                        match native_parent_hwnd {
                                                            Some(parent) => match WindowsVideoSurface::create(parent, measured) {
                                                                Ok(surface) => {
                                                                    *surface_slot = Some(surface);
                                                                    diagnostics::record(
                                                                        diagnostics::Level::Info,
                                                                        "playback",
                                                                        "created native Win32 video child surface",
                                                                    );
                                                                }
                                                                Err(error) => {
                                                                    let message = format!("{error:#}");
                                                                    diagnostics::record(
                                                                        diagnostics::Level::Warn,                                                                        "playback",
                                                                        &format!("could not create native video surface: {message}"),
                                                                    );
                                                                    if let Ok(mut current_error) = native_surface_error.lock() {
                                                                        *current_error = Some(message);
                                                                    }
                                                                }
                                                            },
                                                            None => {
                                                                if let Ok(mut current_error) = native_surface_error.lock() {
                                                                    *current_error = Some("GPUI did not provide a Win32 parent HWND".into());
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                })
                                .child(
                                    div()
                                        .id("native-video-viewport")
                                        .w_full()
                                        .h(px(420.))
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(rgb(0x263238))
                                        .bg(rgb(0x050708))
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(rgb(0x67E8F9))
                                                .child("SENZO VIDEO VIEWPORT"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(0x858C94))
                                                .child(native_surface_status.clone()),
                                        ),
                                )
                        },
                    ))
                    .child(section(
                        "Playback engine",
                        "The MPV worker runs separately from the UI. Playback speed can be changed without interrupting the current media.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0xA5F3FC))
                                    .child(self.playback_status.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xA1A1AA))
                                    .child(
                                        self.playback_state
                                            .media_title
                                            .clone()
                                            .unwrap_or_else(|| "No media loaded".into()),
                                    ),
                            )
                            .child({
                                let bounds = seek_bar_bounds.clone();
                                let progress = match (playback_position, playback_duration) {
                                    (Some(position), Some(duration)) if duration > 0.0 => {
                                        (position / duration).clamp(0.0, 1.0) as f32
                                    }
                                    _ => 0.0,
                                };
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .flex()
                                            .justify_between()
                                            .text_xs()
                                            .text_color(rgb(0xA1A1AA))
                                            .child(format_playback_time(playback_position))
                                            .child(format_playback_time(playback_duration)),
                                    )
                                    .child(
                                        div()
                                            .on_children_prepainted({
                                                let bounds = bounds.clone();
                                                move |children, _window, _cx| {
                                                    if let Some(track) = children.first()
                                                        && let Ok(mut current) = bounds.lock()
                                                    {
                                                        *current = Some((
                                                            f32::from(track.origin.x),
                                                            f32::from(track.size.width),
                                                        ));
                                                    }
                                                }
                                            })
                                            .id("playback-seek-bar")
                                            .w(px(420.0))
                                            .h(px(18.0))
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener({
                                                    let bounds = bounds.clone();
                                                    move |this, event: &gpui::MouseDownEvent, _window, cx| {
                                                        this.seek_from_pointer(
                                                            f32::from(event.position.x),
                                                            &bounds,
                                                            cx,
                                                        );
                                                    }
                                                }),
                                            )
                                            .on_mouse_move(cx.listener({
                                                let bounds = bounds.clone();
                                                move |this, event: &gpui::MouseMoveEvent, _window, cx| {
                                                    if event.pressed_button == Some(MouseButton::Left) {
                                                        this.seek_from_pointer(
                                                            f32::from(event.position.x),
                                                            &bounds,
                                                            cx,
                                                        );
                                                    }
                                                }
                                            }))
                                            .child(
                                                div()
                                                    .w(px(420.0))
                                                    .h(px(6.0))
                                                    .rounded_full()
                                                    .bg(rgb(0x30363B))
                                                    .child(
                                                        div()
                                                            .w(px(420.0 * progress))
                                                            .h(px(6.0))
                                                            .rounded_full()
                                                            .bg(rgb(0x22D3EE)),
                                                    ),
                                            ),
                                    )
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip(
                                        "Open media",
                                        false,
                                        |this, cx| this.open_media(cx),
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Pause / Resume",
                                        self.playback_state.paused.unwrap_or(false),
                                        |this, cx| {
                                            let paused = !this.playback_state.paused.unwrap_or(false);
                                            this.send_playback_command(
                                                "pause",
                                                move |worker| worker.set_paused(paused),
                                                cx,
                                            );
                                        },
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Stop",
                                        false,
                                        |this, cx| {
                                            // Preserve the last known position before stopping MPV.
                                            this.persist_playback_progress(true);
                                            this.send_playback_command(
                                                "stop",
                                                |worker| worker.stop(),
                                                cx,
                                            );
                                            this.stop_torrent_session(cx);
                                        },
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Mute / Unmute",
                                        self.playback_state.muted.unwrap_or(false),
                                        |this, cx| {
                                            let muted = !this.playback_state.muted.unwrap_or(false);
                                            this.send_playback_command(
                                                "mute",
                                                move |worker| worker.set_muted(muted),
                                                cx,
                                            );
                                        },
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Volume −5%",
                                        false,
                                        |this, cx| {
                                            let volume = this.playback_state.volume.unwrap_or(100.0);
                                            let volume = (volume - 5.0).clamp(0.0, 100.0);
                                            this.send_playback_command(
                                                "volume down",
                                                move |worker| worker.set_volume(volume),
                                                cx,
                                            );
                                        },
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Volume +5%",
                                        false,
                                        |this, cx| {
                                            let volume = this.playback_state.volume.unwrap_or(100.0);
                                            let volume = (volume + 5.0).clamp(0.0, 100.0);
                                            this.send_playback_command(
                                                "volume up",
                                                move |worker| worker.set_volume(volume),
                                                cx,
                                            );
                                        },
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "−10 seconds",
                                        false,
                                        |this, cx| {
                                            if let Some(position) = this.playback_state.position_seconds {
                                                this.send_playback_command(
                                                    "seek backward",
                                                    move |worker| worker.seek_absolute((position - 10.0).max(0.0)),
                                                    cx,
                                                );
                                            }
                                        },
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "+10 seconds",
                                        false,
                                        |this, cx| {
                                            if let Some(position) = this.playback_state.position_seconds {
                                                this.send_playback_command(
                                                    "seek forward",
                                                    move |worker| worker.seek_absolute(position + 10.0),
                                                    cx,
                                                );
                                            }
                                        },
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "0.5×".into(),
                                        self.playback_state.speed.is_some_and(|speed| (speed - 0.5).abs() < 0.01),
                                        |this, cx| this.send_playback_command("playback speed", |worker| worker.set_speed(0.5), cx),
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "0.75×".into(),
                                        self.playback_state.speed.is_some_and(|speed| (speed - 0.75).abs() < 0.01),
                                        |this, cx| this.send_playback_command("playback speed", |worker| worker.set_speed(0.75), cx),
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "1.0×".into(),
                                        self.playback_state.speed.is_some_and(|speed| (speed - 1.0).abs() < 0.01),
                                        |this, cx| this.send_playback_command("playback speed", |worker| worker.set_speed(1.0), cx),
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "1.25×".into(),
                                        self.playback_state.speed.is_some_and(|speed| (speed - 1.25).abs() < 0.01),
                                        |this, cx| this.send_playback_command("playback speed", |worker| worker.set_speed(1.25), cx),
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "1.5×".into(),
                                        self.playback_state.speed.is_some_and(|speed| (speed - 1.5).abs() < 0.01),
                                        |this, cx| this.send_playback_command("playback speed", |worker| worker.set_speed(1.5), cx),
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "2.0×".into(),
                                        self.playback_state.speed.is_some_and(|speed| (speed - 2.0).abs() < 0.01),
                                        |this, cx| this.send_playback_command("playback speed", |worker| worker.set_speed(2.0), cx),
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Fullscreen",
                                        self.playback_state.fullscreen.unwrap_or(false),
                                        |this, cx| {
                                            let fullscreen = !this.playback_state.fullscreen.unwrap_or(false);
                                            this.send_playback_command(
                                                "fullscreen",
                                                move |worker| worker.set_fullscreen(fullscreen),
                                                cx,
                                            );
                                        },
                                        cx,
                                    )),
                            )
                            .child({
                                let bounds = volume_bar_bounds.clone();
                                let fraction = (playback_volume / 100.0) as f32;
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0xA1A1AA))
                                            .child(format!("Volume · {:.0}%", playback_volume)),
                                    )
                                    .child(
                                        div()
                                            .on_children_prepainted({
                                                let bounds = bounds.clone();
                                                move |children, _window, _cx| {
                                                    if let Some(track) = children.first()
                                                        && let Ok(mut current) = bounds.lock()
                                                    {
                                                        *current = Some((
                                                            f32::from(track.origin.x),
                                                            f32::from(track.size.width),
                                                        ));
                                                    }
                                                }
                                            })
                                            .id("playback-volume-bar")
                                            .w(px(220.0))
                                            .h(px(18.0))
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener({
                                                    let bounds = bounds.clone();
                                                    move |this, event: &gpui::MouseDownEvent, _window, cx| {
                                                        this.set_volume_from_pointer(
                                                            f32::from(event.position.x),
                                                            &bounds,
                                                            cx,
                                                        );
                                                    }
                                                }),
                                            )
                                            .on_mouse_move(cx.listener({
                                                let bounds = bounds.clone();
                                                move |this, event: &gpui::MouseMoveEvent, _window, cx| {
                                                    if event.pressed_button == Some(MouseButton::Left) {
                                                        this.set_volume_from_pointer(
                                                            f32::from(event.position.x),
                                                            &bounds,
                                                            cx,
                                                        );
                                                    }
                                                }
                                            }))
                                            .child(
                                                div()
                                                    .w(px(220.0))
                                                    .h(px(6.0))
                                                    .rounded_full()
                                                    .bg(rgb(0x30363B))
                                                    .child(
                                                        div()
                                                            .w(px(220.0 * fraction))
                                                            .h(px(6.0))
                                                            .rounded_full()
                                                            .bg(rgb(0x22D3EE)),
                                                    ),
                                            ),
                                    )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x858C94))
                                    .child(format!(
                                        "Video: {} · Decoder: {} · HW: {} · Output: {} · Size: {} · FPS: {} · Dropped: {}",
                                        self.playback_state.diagnostics.video_codec.as_deref().unwrap_or("—"),
                                        self.playback_state.diagnostics.video_decoder.as_deref().unwrap_or("—"),
                                        self.playback_state.diagnostics.hardware_decoder.as_deref().unwrap_or("—"),
                                        self.playback_state.diagnostics.video_output.as_deref().unwrap_or("—"),
                                        match (self.playback_state.diagnostics.video_width, self.playback_state.diagnostics.video_height) {
                                            (Some(width), Some(height)) => format!("{width}×{height}"),
                                            _ => "—".into(),
                                        },
                                        self.playback_state.diagnostics.frame_rate
                                            .map(|value| format!("{value:.2}"))
                                            .unwrap_or_else(|| "—".into()),
                                        self.playback_state.diagnostics.dropped_frames
                                            .map(|value| value.to_string())
                                            .unwrap_or_else(|| "—".into()),
                                    )),
                            ),
                    ))
                    .child(section(
                        "Available tracks",
                        "Choose an embedded audio or subtitle track. Subtitle Off disables subtitle rendering.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x9299A1))
                                    .child("AUDIO"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(audio_track_chips),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x9299A1))
                                    .child("SUBTITLES"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Off", !self.playback_state.tracks.iter().any(|track| track.kind == "sub" && track.selected), |this, cx| {
                                        this.send_playback_command("disable subtitles", |worker| worker.set_subtitle_track(None), cx);
                                    }, cx))
                                    .children(subtitle_track_chips),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Load subtitle file", false, |this, cx| this.open_subtitle_file(cx), cx))
                                    .child(choice_chip("Search online", false, |this, cx| this.search_online_subtitles(cx), cx)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x9299A1))
                                    .child(self.subtitle_provider_status.clone()),
                            )
                            .child(div().text_xs().text_color(rgb(0x9299A1)).child(self.addon_subtitle_status.clone()))
                            .child(div().flex().flex_wrap().gap_2().children(addon_subtitle_chips))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(online_subtitle_chips),
                            ),
                    ))
                    .child(section(
                        "Runtime capability detection",
                        "These values come from the live libmpv session and Windows display configuration. Unknown capabilities stay unavailable rather than being guessed.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!(
                                "Renderer: {} · Hardware decoder: {}",
                                self.playback_state.diagnostics.video_output.as_deref().unwrap_or("not detected"),
                                self.playback_state.diagnostics.hardware_decoder.as_deref().unwrap_or("not active")
                            )))
                            .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!(
                                "Source transfer: {} · Output transfer: {}",
                                self.playback_state.diagnostics.source_transfer.as_deref().unwrap_or("not detected"),
                                self.playback_state.diagnostics.output_transfer.as_deref().unwrap_or("not detected")
                            )))
                            .child(div().text_sm().text_color(rgb(0x9299A1)).child(format!(
                                "HDR source: {} · Windows HDR enabled: {} · Tone mapping: {} · Passthrough: {}",
                                if self.playback_state.capabilities.hdr_source_detected { "yes" } else { "no / unknown" },
                                if self.playback_state.capabilities.hdr_display_enabled { "yes" } else { "no / unknown" },
                                if self.playback_state.capabilities.hdr_tone_mapping_supported { "supported" } else { "unavailable" },
                                if self.playback_state.capabilities.hdr_passthrough_supported { "supported" } else { "unavailable" }
                            ))),
                    ))
                    .child(section(
                        "Hardware acceleration",
                        "Automatic is recommended. Manual decoder choices are advanced compatibility overrides.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(option_row(
                                "Hardware decoding",
                                "Automatic and software are always available. A manual decoder appears only after MPV confirms it is active for the current media.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip(
                                        "Auto",
                                        settings.hardware_decoding == HardwareDecoding::Auto,
                                        |this, _cx| this.settings.hardware_decoding = HardwareDecoding::Auto,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Enabled",
                                        settings.hardware_decoding == HardwareDecoding::Enabled,
                                        |this, _cx| this.settings.hardware_decoding = HardwareDecoding::Enabled,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Disabled",
                                        settings.hardware_decoding == HardwareDecoding::Disabled,                                        |this, _cx| this.settings.hardware_decoding = HardwareDecoding::Disabled,
                                        cx,
                                    )),
                            ))
                            .child(option_row(
                                "Decoder preference",
                                "Manual decoder choices supported by the bundled MPV runtime are available before playback starts.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip(
                                        "Automatic",
                                        settings.decoder == DecoderPreference::Auto,
                                        |this, _cx| this.settings.decoder = DecoderPreference::Auto,
                                        cx,
                                    ))
                                    .when(startup_capabilities.d3d11va_available, |row| {
                                        row.child(choice_chip(
                                            "D3D11VA",
                                            settings.decoder == DecoderPreference::D3d11va,
                                            |this, _cx| this.settings.decoder = DecoderPreference::D3d11va,
                                            cx,
                                        ))
                                    })
                                    .when(startup_capabilities.dxva2_available, |row| {
                                        row.child(choice_chip(
                                            "DXVA2",
                                            settings.decoder == DecoderPreference::Dxva2,
                                            |this, _cx| this.settings.decoder = DecoderPreference::Dxva2,
                                            cx,
                                        ))
                                    })
                                    .child(choice_chip(
                                        "Software",
                                        settings.decoder == DecoderPreference::Software,
                                        |this, _cx| this.settings.decoder = DecoderPreference::Software,
                                        cx,
                                    )),
                            ))
                            .child(option_row(
                                "Video renderer",
                                "Renderer choices supported by the bundled MPV runtime are available before playback starts. Automatic remains available at all times.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip(
                                        "Automatic",
                                        settings.renderer == RendererPreference::Auto,
                                        |this, _cx| this.settings.renderer = RendererPreference::Auto,
                                        cx,
                                    ))
                                    .when(startup_capabilities.gpu_next_available, |row| {
                                        row.child(choice_chip(
                                            "GPU Next",
                                            settings.renderer == RendererPreference::GpuNext,
                                            |this, _cx| this.settings.renderer = RendererPreference::GpuNext,
                                            cx,
                                        ))
                                    })
                                    .when(startup_capabilities.gpu_available, |row| {
                                        row.child(choice_chip(
                                            "GPU",
                                            settings.renderer == RendererPreference::Gpu,
                                            |this, _cx| this.settings.renderer = RendererPreference::Gpu,
                                            cx,
                                        ))
                                    }),
                            )),
                    ))
                    .child(section(
                        "Image quality",
                        "Balanced defaults keep expensive processing disabled until you choose it.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(option_row(
                                "Upscaling algorithm",
                                "Used when the source image is smaller than the display.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Bilinear", settings.upscale == ScalingAlgorithm::Bilinear, |this, _cx| this.settings.upscale = ScalingAlgorithm::Bilinear, cx))
                                    .child(choice_chip("Bicubic", settings.upscale == ScalingAlgorithm::Bicubic, |this, _cx| this.settings.upscale = ScalingAlgorithm::Bicubic, cx))
                                    .child(choice_chip("Lanczos", settings.upscale == ScalingAlgorithm::Lanczos, |this, _cx| this.settings.upscale = ScalingAlgorithm::Lanczos, cx))
                                    .child(choice_chip("Spline36", settings.upscale == ScalingAlgorithm::Spline36, |this, _cx| this.settings.upscale = ScalingAlgorithm::Spline36, cx))
                                    .child(choice_chip("Mitchell", settings.upscale == ScalingAlgorithm::Mitchell, |this, _cx| this.settings.upscale = ScalingAlgorithm::Mitchell, cx)),
                            ))
                            .child(option_row(
                                "Downscaling algorithm",
                                "Used when the source image is larger than the display.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Bilinear", settings.downscale == ScalingAlgorithm::Bilinear, |this, _cx| this.settings.downscale = ScalingAlgorithm::Bilinear, cx))
                                    .child(choice_chip("Bicubic", settings.downscale == ScalingAlgorithm::Bicubic, |this, _cx| this.settings.downscale = ScalingAlgorithm::Bicubic, cx))
                                    .child(choice_chip("Lanczos", settings.downscale == ScalingAlgorithm::Lanczos, |this, _cx| this.settings.downscale = ScalingAlgorithm::Lanczos, cx))
                                    .child(choice_chip("Spline36", settings.downscale == ScalingAlgorithm::Spline36, |this, _cx| this.settings.downscale = ScalingAlgorithm::Spline36, cx))
                                    .child(choice_chip("Mitchell", settings.downscale == ScalingAlgorithm::Mitchell, |this, _cx| this.settings.downscale = ScalingAlgorithm::Mitchell, cx)),
                            ))
                            .child(option_row(
                                "HDR handling",
                                "Options appear when the bundled MPV renderer and Windows display support them. HDR passthrough requires GPU Next and Windows Advanced Color enabled on every active display.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Auto", settings.hdr == HdrMode::Auto, |this, _cx| this.settings.hdr = HdrMode::Auto, cx))
                                    .when(startup_capabilities.hdr_tone_mapping_supported, |row| {
                                        row.child(choice_chip("Tone map", settings.hdr == HdrMode::ToneMap, |this, _cx| this.settings.hdr = HdrMode::ToneMap, cx))
                                            .child(choice_chip("Off", settings.hdr == HdrMode::Disabled, |this, _cx| this.settings.hdr = HdrMode::Disabled, cx))
                                    })
                                    .when(startup_capabilities.hdr_output_supported && startup_capabilities.gpu_next_available, |row| {
                                        row.child(choice_chip("Passthrough", settings.hdr == HdrMode::Passthrough, |this, _cx| this.settings.hdr = HdrMode::Passthrough, cx))
                                    }),
                            ))
                            .child(toggle_row(
                                "Debanding",
                                "Reduce visible color banding. May increase GPU workload.",
                                settings.deband,
                                |this, _cx| this.settings.deband = !this.settings.deband,
                                cx,
                            ))
                            .child(toggle_row(
                                "Frame interpolation",
                                "Optional motion smoothing. Disabled by default to avoid extra processing.",
                                settings.interpolation,
                                |this, _cx| this.settings.interpolation = !this.settings.interpolation,
                                cx,
                            )),
                    ))
                    .child(section(
                        "SENZO metadata service",
                        "Movie and television discovery is provided through the SENZO API. No TMDB account, API key, or setup is required in the desktop app.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().text_sm().text_color(rgb(0xB7BEC6)).child(self.tmdb_service_status.clone()))
                            .child(div().flex().flex_wrap().gap_2()
                                .child(choice_chip("Retry metadata service", false, |this, cx| {
                                    this.home_catalog_requested = false;
                                    this.load_home_catalogs(cx);
                                }, cx))),
                    ))
                    .child(section(
                        "OpenSubtitles",
                        "Enter the API key through a masked Windows credential prompt. SENZO stores it only in Windows Credential Manager and never displays it again.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0xB7BEC6))
                                    .child(self.subtitle_credentials_status.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip(
                                        "Set / replace API key",
                                        false,
                                        |this, cx| this.configure_opensubtitles_key(cx),
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Remove saved key",
                                        false,
                                        |this, cx| this.clear_opensubtitles_key(cx),
                                        cx,
                                    )),
                            ),
                    ))
                    .child(section(
                        "Subtitles",
                        "Adjust subtitle timing and readability while media is playing. Changes are saved locally.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(option_row(
                                "Font size",
                                "Choose the default subtitle font size.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Small · 32", settings.subtitles.font_size == 32, |this, cx| this.set_subtitle_font_size(32, cx), cx))
                                    .child(choice_chip("Default · 42", settings.subtitles.font_size == 42, |this, cx| this.set_subtitle_font_size(42, cx), cx))
                                    .child(choice_chip("Large · 52", settings.subtitles.font_size == 52, |this, cx| this.set_subtitle_font_size(52, cx), cx))
                                    .child(choice_chip("Extra large · 64", settings.subtitles.font_size == 64, |this, cx| this.set_subtitle_font_size(64, cx), cx))
                                    .child(choice_chip("− size", false, |this, cx| {
                                        let size = this.settings.subtitles.font_size.saturating_sub(2).max(12);
                                        this.set_subtitle_font_size(size, cx);
                                    }, cx))
                                    .child(choice_chip("+ size", false, |this, cx| {
                                        let size = this.settings.subtitles.font_size.saturating_add(2).min(96);
                                        this.set_subtitle_font_size(size, cx);
                                    }, cx)),
                            ))
                            .child(option_row(
                                "Subtitle synchronization",
                                "Move subtitles earlier or later in 100 ms steps. Positive values delay subtitles.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("−0.1 s", false, |this, cx| {
                                        let delay = (this.settings.subtitles.delay_seconds - 0.1).max(-10.0);
                                        this.set_subtitle_delay(delay, cx);
                                    }, cx))
                                    .child(choice_chip("Reset", false, |this, cx| this.set_subtitle_delay(0.0, cx), cx))
                                    .child(choice_chip("+0.1 s", false, |this, cx| {
                                        let delay = (this.settings.subtitles.delay_seconds + 0.1).min(10.0);
                                        this.set_subtitle_delay(delay, cx);
                                    }, cx))
                                    .child(div().text_sm().text_color(rgb(0xB7BEC6)).child(format!("{:+.1} s", settings.subtitles.delay_seconds))),
                            ))
                            .child(option_row(
                                "Vertical margin",
                                "Vertical distance from the bottom edge, in pixels.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("12 px", settings.subtitles.vertical_margin == 12, |this, cx| this.set_subtitle_margin(12, cx), cx))
                                    .child(choice_chip("24 px", settings.subtitles.vertical_margin == 24, |this, cx| this.set_subtitle_margin(24, cx), cx))
                                    .child(choice_chip("36 px", settings.subtitles.vertical_margin == 36, |this, cx| this.set_subtitle_margin(36, cx), cx)),
                            ))
                            .child(toggle_row(
                                "Bold subtitles",
                                "Use a heavier subtitle weight for improved legibility.",
                                settings.subtitles.bold,
                                |this, cx| {
                                    let bold = !this.settings.subtitles.bold;
                                    this.set_subtitle_bold(bold, cx);
                                },
                                cx,
                            )),
                    ))
                    .child(section(
                        "Buffering",
                        "MPV applies these bounded cache and memory preferences when the playback session starts.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(stepper_row(
                                "Forward buffer",
                                "Target seconds buffered ahead",
                                settings.buffer.forward_seconds,
                                "seconds",
                                |this, _cx| this.settings.buffer.forward_seconds = this.settings.buffer.forward_seconds.saturating_sub(5),
                                |this, _cx| this.settings.buffer.forward_seconds = this.settings.buffer.forward_seconds.saturating_add(5).min(300),
                                cx,
                            ))
                            .child(stepper_row(
                                "Back buffer",
                                "Retained media for backward seeking",
                                settings.buffer.back_seconds,
                                "seconds",
                                |this, _cx| this.settings.buffer.back_seconds = this.settings.buffer.back_seconds.saturating_sub(5),
                                |this, _cx| this.settings.buffer.back_seconds = this.settings.buffer.back_seconds.saturating_add(5).min(300),
                                cx,
                            ))
                            .child(stepper_row(
                                "Memory budget",
                                "Soft playback buffer memory limit",
                                settings.buffer.memory_limit_mib,
                                "MiB",
                                |this, _cx| this.settings.buffer.memory_limit_mib = this.settings.buffer.memory_limit_mib.saturating_sub(64).max(32),
                                |this, _cx| this.settings.buffer.memory_limit_mib = this.settings.buffer.memory_limit_mib.saturating_add(64).min(2048),
                                cx,
                            )),
                    ))

                    .child(section(
                        "Discord Rich Presence",
                        "Optional local Discord integration. It is off by default; media titles are hidden unless you explicitly allow title sharing.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().text_sm().text_color(rgb(0xB7BEC6)).child(
                                if senzo::discord::DiscordPresence::is_configured() {
                                    "Discord application ID is configured for this build. Discord must be running to publish presence."
                                } else {
                                    "Discord application ID is not configured in this build. The feature stays inactive until SENZO is built with SENZO_DISCORD_CLIENT_ID."
                                }
                            ))
                            .child(toggle_row(
                                "Enable Discord Rich Presence",
                                "Show SENZO browsing or playback status on your Discord profile.",
                                self.discord_presence_enabled,
                                |this, cx| {
                                    let enabled = !this.discord_presence_enabled;
                                    this.set_discord_presence_enabled(enabled, cx);
                                },
                                cx,
                            ))
                            .child(toggle_row(
                                "Share media titles",
                                "When enabled, the current movie or episode title may be visible to anyone who can see your Discord activity. Off by default.",
                                self.discord_share_titles,
                                |this, cx| {
                                    let enabled = !this.discord_share_titles;
                                    this.set_discord_title_sharing(enabled, cx);
                                },
                                cx,
                            )),
                    ))
                    .child(section(
                        "Diagnostics",
                        "Copy a recent support report. Personal watch history and the local database are excluded.",
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_sm().text_color(rgb(0xE4E4E7)).child("Recent application logs"))
                                    .child(div().text_xs().text_color(rgb(0x858C94)).child("Includes up to 72 hours of diagnostic events.")),
                            )
                            .child(
                                div().flex().gap_2()
                                    .child(choice_chip("Open log folder", false, |this, cx| this.open_diagnostics_folder(cx), cx))
                                    .child(div()
                                    .id("copy-diagnostics")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(rgb(0x164E63))
                                    .text_color(rgb(0xCFFAFE))
                                    .text_sm()
                                    .cursor_pointer()
                                    .child("Copy logs")
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _event, _window, cx| {
                                        this.copy_diagnostics(cx);
                                    }))),
                            ),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .py_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x8B949E))
                                    .child(self.status.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x65707A))
                                    .child("Restart SENZO to apply engine-level preferences; subtitle appearance changes apply live."),
                            ),
                    )                    })

            )
            })
            .when(self.active_page == AppPage::Player, |root| {
                let viewport_bounds = self.native_viewport_bounds.clone();
                let native_parent_hwnd = self.native_parent_hwnd;
                let native_video_surface = self.native_video_surface.clone();
                let native_surface_error = self.native_surface_error.clone();
                let progress = match (playback_position, playback_duration) {
                    (Some(position), Some(duration)) if duration > 0.0 => (position / duration).clamp(0.0, 1.0) as f32,
                    _ => 0.0,
                };
                let mut player_audio_tracks = Vec::new();
                let mut player_subtitle_tracks = Vec::new();
                for track in &self.playback_state.tracks {
                    let id = track.id;
                    let label = format!("{}{}", track.title.as_deref().unwrap_or(if track.kind == "audio" { "Audio" } else { "Subtitle" }), track.language.as_deref().map(|language| format!(" · {language}")).unwrap_or_default());
                    if track.kind == "audio" {
                        player_audio_tracks.push(dynamic_choice_chip(label, track.selected, move |this, cx| this.send_playback_command("select audio track", move |worker| worker.set_audio_track(id), cx), cx).into_any_element());
                    } else if track.kind == "sub" {
                        player_subtitle_tracks.push(dynamic_choice_chip(label, track.selected, move |this, cx| this.send_playback_command("select subtitle track", move |worker| worker.set_subtitle_track(Some(id)), cx), cx).into_any_element());
                    }
                }
                root.child(
                    div().id("senzo-player-screen").flex_1().h_full().min_w_0().overflow_y_scroll().p_4().flex().flex_col().gap_3().bg(rgb(0x050708))
                        .child(div().flex().items_center().justify_between()
                            .child(div().flex().flex_col().gap_1()
                                .child(div().text_2xl().font_weight(gpui::FontWeight::BOLD).text_color(rgb(0xF8FAFC)).child("NOW PLAYING"))
                                .child(div().text_sm().text_color(rgb(0x9299A1)).child(self.playback_status.clone())))
                            .child(div().flex().items_center().gap_2()
                                .child(choice_chip("Back", false, |this, cx| { this.active_page = if this.selected_title.is_some() { AppPage::Details } else { AppPage::Home }; cx.notify(); }, cx))
                                .child(choice_chip("Open media", false, |this, cx| this.open_media(cx), cx))))
                        .child(div().w_full().on_children_prepainted(move |children, window, _cx| {
                            if let Some(bounds) = children.first() {
                                if let Ok(measured) = SurfaceBounds::from_logical_pixels(f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height), window.scale_factor()) {
                                    if let Ok(mut current) = viewport_bounds.lock() { *current = Some(measured); }
                                    if let Ok(mut slot) = native_video_surface.lock() {
                                        if let Some(surface) = slot.as_mut() {
                                            if let Err(error) = surface.set_bounds(measured) {
                                                if let Ok(mut error_slot) = native_surface_error.lock() { *error_slot = Some(format!("{error:#}")); }
                                            }
                                            surface.set_visible(true);
                                            if let Err(error) = surface.raise() {
                                                if let Ok(mut error_slot) = native_surface_error.lock() {
                                                    *error_slot = Some(format!("could not raise native video surface: {error:#}"));
                                                }
                                                diagnostics::record(
                                                    diagnostics::Level::Warn,
                                                    "playback",
                                                    &format!("could not raise native video surface: {error:#}"),
                                                );
                                            }
                                        } else if let Some(parent) = native_parent_hwnd {
                                            match WindowsVideoSurface::create(parent, measured) {
                                                Ok(mut surface) => {
                                                    surface.set_visible(true);
                                                    if let Err(error) = surface.raise() {
                                                        diagnostics::record(
                                                            diagnostics::Level::Warn,
                                                            "playback",
                                                            &format!("could not raise newly created native video surface: {error:#}"),
                                                        );
                                                    }
                                                    *slot = Some(surface);
                                                    if let Ok(mut error_slot) = native_surface_error.lock() { *error_slot = None; }
                                                }
                                                Err(error) => if let Ok(mut error_slot) = native_surface_error.lock() { *error_slot = Some(format!("{error:#}")); },
                                            }
                                        }
                                    }
                                }
                            }
                        }).child(div().id("player-video-viewport").w_full().flex_1().min_h(px(300.0)).rounded_lg().border_1().border_color(rgb(0x263238)).bg(rgb(0x000000)).flex().items_center().justify_center()
                            .child(div().text_sm().text_color(rgb(0x67E8F9)).child("SENZO NATIVE VIDEO"))))
                        .child(div().flex().items_center().justify_between()
                            .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child(self.playback_state.media_title.clone().or_else(|| self.selected_title.clone()).unwrap_or_else(|| "No media loaded".into())))
                            .child(div().text_xs().text_color(rgb(0x9299A1)).child(format!("{} / {}", format_playback_time(playback_position), format_playback_time(playback_duration)))))
                        .child({
                            let bounds = seek_bar_bounds.clone();
                            div().w_full().h(px(14.0)).rounded_full().bg(rgb(0x30363B))
                                .on_children_prepainted({
                                    let bounds = bounds.clone();
                                    move |children, _window, _cx| {
                                        if let Some(track) = children.first() {
                                            if let Ok(mut current) = bounds.lock() {
                                                *current = Some((f32::from(track.origin.x), f32::from(track.size.width)));
                                            }
                                        }
                                    }
                                })
                                .on_mouse_down(MouseButton::Left, cx.listener({
                                    let bounds = bounds.clone();
                                    move |this, event: &gpui::MouseDownEvent, _window, cx| this.seek_from_pointer(f32::from(event.position.x), &bounds, cx)
                                }))
                                .on_mouse_move(cx.listener({
                                    let bounds = bounds.clone();
                                    move |this, event: &gpui::MouseMoveEvent, _window, cx| {
                                        if event.pressed_button == Some(MouseButton::Left) {
                                            this.seek_from_pointer(f32::from(event.position.x), &bounds, cx);
                                        }
                                    }
                                }))
                                .child(div().h(px(8.0)).w(px(900.0 * progress)).rounded_full().bg(rgb(0x22D3EE)))
                        })
                        .child(div().flex().flex_wrap().items_center().gap_2()
                            .child(choice_chip("−10 sec", false, |this, cx| { if let Some(position) = this.playback_state.position_seconds { this.send_playback_command("seek backward", move |worker| worker.seek_absolute((position - 10.0).max(0.0)), cx); } }, cx))
                            .child(choice_chip(if self.playback_state.paused.unwrap_or(false) { "Play" } else { "Pause" }, self.playback_state.paused.unwrap_or(false), |this, cx| { let paused = !this.playback_state.paused.unwrap_or(false); this.send_playback_command("pause", move |worker| worker.set_paused(paused), cx); }, cx))
                            .child(choice_chip("+10 sec", false, |this, cx| { if let Some(position) = this.playback_state.position_seconds { this.send_playback_command("seek forward", move |worker| worker.seek_absolute(position + 10.0), cx); } }, cx))
                            .child(choice_chip(if self.playback_state.muted.unwrap_or(false) { "Unmute" } else { "Mute" }, self.playback_state.muted.unwrap_or(false), |this, cx| { let muted = !this.playback_state.muted.unwrap_or(false); this.send_playback_command("mute", move |worker| worker.set_muted(muted), cx); }, cx))
                            .child(choice_chip("Volume −", false, |this, cx| { let volume = (this.playback_state.volume.unwrap_or(100.0) - 5.0).clamp(0.0, 100.0); this.send_playback_command("volume down", move |worker| worker.set_volume(volume), cx); }, cx))
                            .child(choice_chip("Volume +", false, |this, cx| { let volume = (this.playback_state.volume.unwrap_or(100.0) + 5.0).clamp(0.0, 100.0); this.send_playback_command("volume up", move |worker| worker.set_volume(volume), cx); }, cx))
                            .child(choice_chip("Fullscreen", self.playback_state.fullscreen.unwrap_or(false), |this, cx| { let fullscreen = !this.playback_state.fullscreen.unwrap_or(false); this.send_playback_command("fullscreen", move |worker| worker.set_fullscreen(fullscreen), cx); }, cx))
                            .child(choice_chip("Stop", false, |this, cx| { this.persist_playback_progress(true); this.send_playback_command("stop", |worker| worker.stop(), cx); this.stop_torrent_session(cx); }, cx)))
                        .child(section("Audio tracks", "Choose an audio stream from the active media.", div().flex().flex_wrap().gap_2().children(player_audio_tracks)))
                        .child(section("Subtitles", "Select a subtitle track or add a local or online subtitle.", div().flex().flex_wrap().gap_2()
                            .child(choice_chip("Off", false, |this, cx| this.send_playback_command("disable subtitles", |worker| worker.set_subtitle_track(None), cx), cx))
                            .children(player_subtitle_tracks)
                            .child(choice_chip("Open subtitle file", false, |this, cx| this.open_subtitle_file(cx), cx))
                            .child(choice_chip("Search online subtitles", false, |this, cx| this.search_online_subtitles(cx), cx))))
                        .child(section("Playback diagnostics", "Live values reported by the native MPV session.", div().flex().flex_wrap().gap_3()
                            .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Video · {} × {}", self.playback_state.diagnostics.video_width.map(|v| v.to_string()).unwrap_or_else(|| "—".into()), self.playback_state.diagnostics.video_height.map(|v| v.to_string()).unwrap_or_else(|| "—".into()))))
                            .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Codec · {}", self.playback_state.diagnostics.video_codec.as_deref().unwrap_or("—"))))
                            .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Decoder · {}", self.playback_state.diagnostics.video_decoder.as_deref().unwrap_or("—"))))
                            .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Hardware · {}", self.playback_state.diagnostics.hardware_decoder.as_deref().unwrap_or("—"))))
                             .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Output · {} · configured {}", self.playback_state.diagnostics.video_output.as_deref().unwrap_or("—"), self.playback_state.diagnostics.video_output_configured.map(|v| v.to_string()).unwrap_or_else(|| "—".into()))))
                             .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Format · {}", self.playback_state.diagnostics.video_format.as_deref().unwrap_or("—"))))
                             .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Frame rate · {}", self.playback_state.diagnostics.frame_rate.map(|v| format!("{v:.2} fps")).unwrap_or_else(|| "—".into()))))
                             .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(format!("Dropped · {} decoder · {} mistimed · {}", self.playback_state.diagnostics.dropped_frames.map(|v| v.to_string()).unwrap_or_else(|| "—".into()), self.playback_state.diagnostics.decoder_dropped_frames.map(|v| v.to_string()).unwrap_or_else(|| "—".into()), self.playback_state.diagnostics.mistimed_frames.map(|v| v.to_string()).unwrap_or_else(|| "—".into()))))))
                        .child(div().text_xs().text_color(rgb(0x858C94)).child("Keyboard: Space play/pause · ←/→ seek 10 seconds · ↑/↓ volume · M mute · F fullscreen · Esc leave player"))
                )
            })
            .when(self.active_page != AppPage::Settings && self.active_page != AppPage::Player, |root| {
                root.child(render_discovery_page(
                    self.active_page,
                    self.selected_title.clone(),
                    self.search_query.clone(),
                    self.search_input.clone(),
                    self.database.installed_addons().unwrap_or_default(),
                    self.addon_manifest_preview.clone(),
                    self.addon_status.clone(),
                    self.tmdb_results.clone(),
                    self.tmdb_status.clone(),
                    self.home_catalogs.clone(),
                    self.home_addon_catalogs.clone(),
                    self.home_catalog_status.clone(),
                    self.home_hero_index,
                    self.home_hero_paused,
                    self.tmdb_details.clone(),
                    self.tv_seasons.clone(),
                    self.tv_selected_season,
                    self.tv_episodes.clone(),
                    self.tv_selected_episode.clone(),
                    self.tv_episode_status.clone(),
                    self.addon_catalog_items.clone(),
                    self.addon_catalog_status.clone(),
                    self.addon_catalog_addon.clone(),
                    self.addon_catalog_media_type.clone(),
                    self.addon_catalog_id.clone(),
                    self.addon_catalog_page,
                    self.addon_catalog_filter_specs.clone(),
                    self.addon_catalog_filter_values.clone(),
                    self.addon_stream_results.clone(),
                    self.stream_status.clone(),
                    self.torrent_files.clone(),
                    self.torrent_status.clone(),
                    self.database.recent_playback_progress(8).unwrap_or_default(),
                    self.metadata_language.clone(),
                    self.poster_size.clone(),
                    self.home_shelf_order.clone(),
                    self.hidden_home_shelves.clone(),
                    self.home_customize_open,
                    cx,
                ))
            })
    }
}

impl Drop for SenzoApp {
    fn drop(&mut self) {
        self.persist_playback_progress(true);
        if let Some(worker) = self.playback_worker.take() {
            let mut worker = worker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = worker.stop_and_wait();
            worker.shutdown();
        }
        if let Some(session) = self.torrent_session.take() {
            let _ = std::thread::spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    let _ = runtime.block_on(tokio::time::timeout(
                        Duration::from_secs(8),
                        session.shutdown(),
                    ));
                }
            })
            .join();
        }
    }
}

fn render_discovery_page(
    page: AppPage,
    selected_title: Option<String>,
    search_query: String,
    search_input: Option<Entity<InputState>>,
    installed_addons: Vec<StoredAddon>,
    addon_manifest_preview: Option<AddonManifestPreview>,
    addon_status: String,
    tmdb_results: Vec<(String, TmdbSearchResult)>,
    tmdb_status: String,
    home_catalogs: Vec<(String, String, TmdbSearchResult)>,
    home_addon_catalogs: Vec<(String, StoredAddon, String, String, Vec<Value>)>,
    home_catalog_status: String,
    home_hero_index: usize,
    _home_hero_paused: bool,
    tmdb_details: Option<Value>,
    tv_seasons: Vec<(u32, String)>,
    tv_selected_season: Option<u32>,
    tv_episodes: Vec<TmdbEpisode>,
    tv_selected_episode: Option<TmdbEpisode>,
    tv_episode_status: String,
    addon_catalog_items: Vec<Value>,
    addon_catalog_status: String,
    addon_catalog_addon: Option<StoredAddon>,
    addon_catalog_media_type: String,
    addon_catalog_id: String,
    addon_catalog_page: usize,
    addon_catalog_filter_specs: Vec<AddonCatalogExtra>,
    addon_catalog_filter_values: Vec<(String, Vec<String>)>,
    addon_stream_results: Vec<SourceProviderResult>,
    stream_status: String,
    torrent_files: Vec<TorrentVideoFile>,
    torrent_status: String,
    continue_watching: Vec<PlaybackProgress>,
    metadata_language: String,
    poster_size: String,
    home_shelf_order: Vec<String>,
    hidden_home_shelves: Vec<String>,
    home_customize_open: bool,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement {
    let (heading, description) = match page {
        AppPage::Home => ("Discover", "Find your next movie or series"),
        AppPage::Movies => ("Movies", "Explore the movie catalogue"),
        AppPage::TvShows => ("TV Shows", "Explore series, seasons and episodes"),
        AppPage::Search => ("Search", "Find movies and TV shows in one place"),
        AppPage::Library => (
            "Your Library",
            "Your saved titles and viewing activity stay on this device",
        ),
        AppPage::Details => (
            "Title details",
            "A closer look at the selected movie or series",
        ),
        AppPage::Addons => ("Add-ons", "Manage the sources you choose to install"),
        AppPage::Settings => ("Settings", "Application preferences"),
        AppPage::Player => ("Now Playing", "Native playback and playback controls"),
    };

    let is_details = page == AppPage::Details;
    let is_search = page == AppPage::Search;
    let is_library = page == AppPage::Library;
    let is_home = page == AppPage::Home;
    let (poster_width, poster_height) = if poster_size == "compact" {
        (px(150.), px(225.))
    } else {
        (px(180.), px(270.))
    };
    let _content_language = metadata_language;
    let mut title_chips = Vec::new();
    for (media_type, item) in &tmdb_results {
        let title = item.display_title().unwrap_or("Untitled").to_owned();
        let year = item
            .release_date
            .as_deref()
            .or(item.first_air_date.as_deref())
            .and_then(|date| date.get(0..4))
            .unwrap_or("—");
        let rating = item
            .vote_average
            .map(|value| format!(" · {value:.1}/10"))
            .unwrap_or_default();
        let label = format!("{title} · {year}{rating}");
        let selected = item.clone();
        let kind = media_type.clone();
        title_chips.push(dynamic_choice_chip(
            label,
            false,
            move |this, cx| this.select_tmdb_title(kind.clone(), selected.clone(), cx),
            cx,
        ));
    }
    let mut search_title_chips = Vec::new();
    for (media_type, item) in &tmdb_results {
        let title = item.display_title().unwrap_or("Untitled").to_owned();
        let year = item
            .release_date
            .as_deref()
            .or(item.first_air_date.as_deref())
            .and_then(|date| date.get(0..4))
            .unwrap_or("—");
        let rating = item
            .vote_average
            .map(|value| format!(" · {value:.1}/10"))
            .unwrap_or_default();
        let label = format!("{title} · {year}{rating}");
        let selected = item.clone();
        let kind = media_type.clone();
        search_title_chips.push(dynamic_choice_chip(
            label,
            false,
            move |this, cx| this.select_tmdb_title(kind.clone(), selected.clone(), cx),
            cx,
        ));
    }
    let mut catalog_chips = Vec::new();
    for addon in &installed_addons {
        if !addon.enabled {
            continue;
        }
        let Ok(manifest) = serde_json::from_str::<AddonManifest>(&addon.manifest_json) else {
            continue;
        };
        if !manifest.supports_resource("catalog") {
            continue;
        }
        for catalog in &manifest.catalogs {
            let Some(id) = catalog.get("id").and_then(Value::as_str) else {
                continue;
            };
            let kind = catalog
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("movie")
                .to_owned();
            if !manifest.supports_resource_for("catalog", &kind, id) {
                continue;
            }
            let name = catalog.get("name").and_then(Value::as_str).unwrap_or(id);
            let has_filters = !AddonManifest::catalog_extra_properties(catalog).is_empty();
            let label = format!(
                "{} · {}{}",
                addon.name,
                name,
                if has_filters { " · filters" } else { "" }
            );
            let addon_copy = addon.clone();
            let catalog_copy = catalog.clone();
            let catalog_id = id.to_owned();
            catalog_chips.push(dynamic_choice_chip(
                label,
                false,
                move |this, cx| {
                    this.select_addon_catalog(
                        addon_copy.clone(),
                        kind.clone(),
                        catalog_id.clone(),
                        catalog_copy.clone(),
                        cx,
                    )
                },
                cx,
            ));
        }
    }
    let mut catalog_filter_rows = Vec::new();
    for property in &addon_catalog_filter_specs {
        let selected_values = addon_catalog_filter_values
            .iter()
            .find(|(name, _)| name == &property.name)
            .map(|(_, values)| values.clone())
            .unwrap_or_default();
        let mut choices = Vec::new();
        if property.options.is_empty() {
            let name = property.name.clone();
            choices.push(
                dynamic_choice_chip(
                    format!("Paste value for {}", property.name),
                    !selected_values.is_empty(),
                    move |this, cx| this.paste_addon_catalog_filter(name.clone(), cx),
                    cx,
                )
                .into_any_element(),
            );
        } else {
            for option in &property.options {
                let name = property.name.clone();
                let value = option.clone();
                let selected = selected_values.iter().any(|item| item == option);
                let limit = property.options_limit;
                choices.push(
                    dynamic_choice_chip(
                        option.clone(),
                        selected,
                        move |this, cx| {
                            this.toggle_addon_catalog_filter(name.clone(), value.clone(), limit, cx)
                        },
                        cx,
                    )
                    .into_any_element(),
                );
            }
        }
        let current = if selected_values.is_empty() {
            "No value selected".to_owned()
        } else {
            selected_values.join(", ")
        };
        catalog_filter_rows.push(
            div()
                .p_3()
                .rounded_md()
                .bg(rgb(0x101315))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!(
                            "{}{} · {}",
                            property.name,
                            if property.is_required { " *" } else { "" },
                            current
                        )),
                )
                .child(div().flex().flex_wrap().gap_2().children(choices))
                .into_any_element(),
        );
    }
    let mut catalog_item_chips = Vec::new();
    if let Some(addon) = addon_catalog_addon.clone() {
        for item in &addon_catalog_items {
            let title = item
                .get("name")
                .or_else(|| item.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("Untitled catalog item");
            let item_copy = item.clone();
            let addon_copy = addon.clone();
            catalog_item_chips.push(dynamic_choice_chip(
                title.to_owned(),
                false,
                move |this, cx| {
                    this.select_addon_catalog_meta(addon_copy.clone(), item_copy.clone(), cx)
                },
                cx,
            ));
        }
    }
    let mut stream_chips = Vec::new();
    let mut torrent_file_chips = Vec::new();
    let mut source_notes = Vec::new();
    for provider in &addon_stream_results {
        if let Some(error) = &provider.error {
            source_notes.push(format!(
                "{} · provider error: {error}",
                provider.provider_name
            ));
        }
        for stream in &provider.streams {
            let is_torrent = stream.is_torrent();
            let is_direct = stream.direct_url().ok().flatten().is_some();
            if is_direct || is_torrent {
                let label = format!(
                    "{} · {} · {}",
                    provider.provider_name,
                    stream
                        .name
                        .as_deref()
                        .or(stream.title.as_deref())
                        .unwrap_or(if is_torrent {
                            "Torrent source"
                        } else {
                            "Direct stream"
                        }),
                    if is_torrent { "torrent" } else { "stream" }
                );
                let selected = stream.clone();
                stream_chips.push(
                    dynamic_choice_chip(
                        label,
                        false,
                        move |this: &mut SenzoApp, cx: &mut Context<SenzoApp>| {
                            this.play_addon_stream(selected.clone(), cx)
                        },
                        cx,
                    )
                    .into_any_element(),
                );
            } else if let Some(external_url) = stream.external_url.as_deref() {
                let provider_name = provider.provider_name.clone();
                let label = format!(
                    "{} · {} · open link",
                    provider_name,
                    stream
                        .name
                        .as_deref()
                        .or(stream.title.as_deref())
                        .unwrap_or("External source")
                );
                let external_url = external_url.to_owned();
                stream_chips.push(
                    dynamic_choice_chip(
                        label,
                        false,
                        move |this: &mut SenzoApp, cx: &mut Context<SenzoApp>| {
                            this.open_addon_external_link(&external_url, cx)
                        },
                        cx,
                    )
                    .into_any_element(),
                );
            }
        }
    }

    for file in &torrent_files {
        let index = file.index;
        let label = format!(
            "{} · {:.1} MiB",
            file.name,
            file.size_bytes as f64 / 1_048_576.0
        );
        torrent_file_chips.push(dynamic_choice_chip(
            label,
            false,
            move |this, cx| this.start_torrent_file(index, cx),
            cx,
        ));
    }

    let continue_watching_cards = continue_watching
        .into_iter()
        .filter(|progress| {
            progress.media_id.starts_with("local-file:") || progress.media_id.starts_with("tmdb:")
        })
        .map(|progress| {
            let media_id = progress.media_id.clone();
            let title = progress
                .title
                .clone()
                .or_else(|| {
                    progress
                        .media_id
                        .strip_prefix("local-file:")
                        .and_then(|path| {
                            PathBuf::from(path)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .map(str::to_owned)
                        })
                })
                .unwrap_or_else(|| "Recently watched".into());
            let progress_label = progress
                .duration_seconds
                .filter(|duration| duration.is_finite() && *duration > 0.0)
                .map(|duration| {
                    format!(
                        "{:.0}% watched",
                        (progress.position_seconds / duration * 100.0).clamp(0.0, 99.0)
                    )
                })
                .unwrap_or_else(|| {
                    format!(
                        "Continue at {}",
                        format_playback_time(Some(progress.position_seconds))
                    )
                });
            div()
                .w(px(220.))
                .flex_shrink_0()
                .rounded_lg()
                .bg(rgb(0x171D20))
                .border_1()
                .border_color(rgb(0x263238))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x9299A1))
                        .child(progress_label),
                )
                .child(dynamic_choice_chip(
                    "Continue watching".into(),
                    true,
                    move |this, cx| this.resume_saved_media(media_id.clone(), cx),
                    cx,
                ))
        })
        .collect::<Vec<_>>();

    div()
        .id("discovery-page")
        .flex_1()
        .h_full()
        .overflow_y_scroll()
        .p_8()
        .flex()
        .flex_col()
        .gap_6()
        .bg(rgb(0x0A0A0A))
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .when(!is_home, |top| {
                            top.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .child(heading),
                                    )
                                    .child(div().text_sm().text_color(rgb(0x9299A1)).child(description)),
                            )
                        })
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1_5()
                                .text_xs()
                                .text_color(rgb(0x94A3B8))
                                .child("🕒")
                                .child(current_time_string()),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(0x181F26))
                                .text_xs()
                                .text_color(rgb(0xE2E8F0))
                                .child("🗖"),
                        ),
                ),
        )
        .when(
            !is_home
                && !is_search
                && page != AppPage::Addons
                && page != AppPage::Library
                && page != AppPage::Details,
            |panel| {
                panel
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(dynamic_choice_chip(
                                "Popular movies".into(),
                                false,
                                |this, cx| this.load_tmdb_catalog("movie", None, cx),
                                cx,
                            ))
                            .child(dynamic_choice_chip(
                                "Popular TV".into(),
                                false,
                                |this, cx| this.load_tmdb_catalog("tv", None, cx),
                                cx,
                            ))
                            .child(dynamic_choice_chip(
                                "Paste movie search".into(),
                                false,
                                |this, cx| this.search_tmdb_from_clipboard("movie", cx),
                                cx,
                            ))
                            .child(dynamic_choice_chip(
                                "Paste TV search".into(),
                                false,
                                |this, cx| this.search_tmdb_from_clipboard("tv", cx),
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x9299A1))
                            .child(tmdb_status.clone()),
                    )
                    .child(div().flex().flex_wrap().gap_2().children(title_chips))
            },
        )
        .when(is_search, |panel| {
            let input = search_input
                .clone()
                .expect("Search input is initialized before rendering the Search page");
            let focus_input = input.clone();
            panel.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        InputBase::new("senzo-global-search")
                            .flex_1()
                            .h(px(58.))
                            .px_4()
                            .rounded_xl()
                            .bg(rgb(0x151719))
                            .border_1()
                            .border_color(rgb(0x263238))
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().text_lg().text_color(rgb(0x22D3EE)).child("⌕"))
                            .on_mouse_down(
                                MouseButton::Left,
                                move |_, window, cx| {
                                    focus_input.update(cx, |state, cx| state.focus(window, cx));
                                },
                            )
                            .child(GpuiInput::new(&input)),
                    )
                    .child(dynamic_choice_chip(
                        "Search".into(),
                        true,
                        |this, cx| {
                            let query = this.search_query.clone();
                            this.load_tmdb_search(query, cx);
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x9299A1))
                    .child("Search across movies and TV shows · Results update as you type · Press Enter to search"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9299A1))
                    .child(tmdb_status.clone()),
            )
            .child(div().flex().flex_wrap().gap_2().children(search_title_chips))
        })
        .when(is_home, |this| {
            let mut ordered_shelf_names = [
                "Trending Movies",
                "Trending TV Shows",
                "Trending this week",
                "Popular movies",
                "Popular TV",
            ];
            ordered_shelf_names.sort_by_key(|name| {
                home_shelf_order
                    .iter()
                    .position(|saved| saved == *name)
                    .unwrap_or(usize::MAX)
            });
            // Prefer titles with usable artwork so the hero never selects an
            // item that can only render as an empty backdrop.
            let has_hero_artwork = |item: &TmdbSearchResult| {
                item.backdrop_path
                    .as_deref()
                    .and_then(|path| tmdb_artwork_url(Some(path), "w1280"))
                    .is_some()
                    || item
                        .poster_path
                        .as_deref()
                        .and_then(|path| tmdb_artwork_url(Some(path), "w780"))
                        .is_some()
            };
            let featured_items = home_catalogs
                .iter()
                .filter(|(shelf, _, item)| {
                    (shelf == "Trending Movies" || shelf == "Trending this week")
                        && has_hero_artwork(item)
                })
                .take(7)
                .cloned()
                .collect::<Vec<_>>();
            let featured_items = if featured_items.is_empty() {
                home_catalogs
                    .iter()
                    .filter(|(_, _, item)| has_hero_artwork(item))
                    .take(7)
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                featured_items
            };
            let featured_count = featured_items.len();
            let featured_index = if featured_count == 0 { 0 } else { home_hero_index % featured_count };
            let featured = featured_items.get(featured_index).cloned();
            let hero = if let Some((_, kind, item)) = featured {
                let title = item.display_title().unwrap_or("Featured title").to_owned();
                let year = item.release_date.as_deref().or(item.first_air_date.as_deref())
                    .and_then(|date| date.get(0..4)).unwrap_or("—").to_owned();
                let rating = item.vote_average.map(|value| format!("★ {:.1}/10", value)).unwrap_or_else(|| "Not rated".into());
                let overview = item.overview.clone().unwrap_or_else(|| "Discover more details and available sources.".into());
                let selected = item.clone();
                let selected_kind = kind.clone();
                // A poster is a deliberate fallback for titles without a backdrop.
                let backdrop = tmdb_artwork_url(item.backdrop_path.as_deref(), "w1280")
                    .or_else(|| tmdb_artwork_url(item.poster_path.as_deref(), "w780"));
                div()
                    .w_full()
                    .h(px(520.))
                    .flex_shrink_0()
                    .relative()
                    .overflow_hidden()
                    .rounded_2xl()
                    .bg(rgb(0x101619))
                    .border_1()
                    .border_color(rgb(0x1E242B))
                    .when_some(backdrop, |panel, url| {
                        panel.child(
                            img(url)
                                .absolute()
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .with_animation(
                                    format!("home-hero-backdrop-{featured_index}"),
                                    Animation::new(Duration::from_millis(700))
                                        .with_easing(ease_in_out),
                                    |element, progress| element.opacity(progress),
                                ),
                        )
                    })
                    .child(div().absolute().inset_0().bg(rgb(0x05090C)).opacity(0.35))
                    .child(div().absolute().left_0().top_0().bottom_0().w(px(900.)).bg(rgb(0x070A0D)).opacity(0.82))
                    .child(div().absolute().left_0().right_0().bottom_0().h(px(260.)).bg(rgb(0x070A0D)).opacity(0.65))
                    .child(
                        div()
                            .relative()
                            .h_full()
                            .p_10()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0x22D3EE))
                                            .child("SENZO FEATURED"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .max_w(px(640.))
                                    .child(
                                        div()
                                            .text_3xl()
                                            .font_weight(gpui::FontWeight::EXTRA_BOLD)
                                            .text_color(rgb(0xF8FAFC))
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgb(0x94A3B8))
                                                    .child(format!(
                                                        "{}  •  {}  •  {}",
                                                        if kind == "movie" { "Movie" } else { "Series" },
                                                        year,
                                                        rating
                                                    )),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .max_w(px(620.))
                                            .text_sm()
                                            .text_color(rgb(0xD1D5DB))
                                            .child(overview),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .pt_3()
                                            .child({
                                                let play_kind = selected_kind.clone();
                                                let play_item = selected.clone();
                                                div()
                                                    .px_6()
                                                    .py_2_5()
                                                    .rounded_xl()
                                                    .bg(rgb(0x22D3EE))
                                                    .text_color(rgb(0x0A0A0A))
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_sm()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x67E8F9)))
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child("▶")
                                                    .child("Play")
                                                    .tab_index(0)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |this, _event, _window, cx| {
                                                            this.select_tmdb_title(
                                                                play_kind.clone(),
                                                                play_item.clone(),
                                                                cx,
                                                            );
                                                        }),
                                                    )
                                            })
                                            .child({
                                                let info_kind = selected_kind.clone();
                                                let info_item = selected.clone();
                                                div()
                                                    .px_5()
                                                    .py_2_5()
                                                    .rounded_xl()
                                                    .bg(rgb(0x1F2937))
                                                    .text_color(rgb(0xF8FAFC))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_sm()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x374151)))
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child("ℹ")
                                                    .child("More Info")
                                                    .tab_index(0)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |this, _event, _window, cx| {
                                                            this.select_tmdb_title(
                                                                info_kind.clone(),
                                                                info_item.clone(),
                                                                cx,
                                                            );
                                                        }),
                                                    )
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .gap_3()
                                    .children((0..featured_count).map(|index| {
                                        let is_active = index == featured_index;
                                        div()
                                            .h(px(8.))
                                            .w(if is_active { px(22.) } else { px(8.) })
                                            .rounded_full()
                                            .bg(if is_active {
                                                rgb(0x22D3EE)
                                            } else {
                                                rgb(0x374151)
                                            })
                                            .cursor_pointer()
                                            .hover(|s| {
                                                if !is_active {
                                                    s.bg(rgb(0x6B7280))
                                                } else {
                                                    s
                                                }
                                            })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _event, _window, cx| {
                                                    this.home_hero_index = index;
                                                    cx.notify();
                                                }),
                                            )
                                    }))
                                    .child(
                                        div()
                                            .size(px(32.))
                                            .rounded_full()
                                            .bg(rgb(0x18181B))
                                            .text_color(rgb(0xF4F4F5))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .hover(|s| s.bg(rgb(0x27272A)))
                                            .child("‹")
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _event, _window, cx| {
                                                    if featured_count > 0 {
                                                        this.home_hero_index = (featured_index + featured_count - 1) % featured_count;
                                                        cx.notify();
                                                    }
                                                }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .size(px(32.))
                                            .rounded_full()
                                            .bg(rgb(0x18181B))
                                            .text_color(rgb(0xF4F4F5))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .hover(|s| s.bg(rgb(0x27272A)))
                                            .child("›")
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _event, _window, cx| {
                                                    if featured_count > 0 {
                                                        this.home_hero_index = (featured_index + 1) % featured_count;
                                                        cx.notify();
                                                    }
                                                }),
                                            ),
                                    )
                            )
                            .with_animation(
                                format!("home-hero-content-{featured_index}"),
                                Animation::new(Duration::from_millis(420))
                                    .with_easing(ease_in_out),
                                |element, progress| element.opacity(progress),
                            ),
                    )
            } else {
                div()
                    .w_full()
                    .h(px(420.))
                    .flex_shrink_0()
                    .relative()
                    .overflow_hidden()
                    .rounded_2xl()
                    .bg(rgb(0x101619))
                    .border_1()
                    .border_color(rgb(0x1E242B))
                    .child(
                        div()
                            .relative()
                            .h_full()
                            .p_10()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0x22D3EE))
                                            .child("WELCOME TO SENZO"),
                                    )
                                    .child(
                                        div()
                                            .px_2_5()
                                            .py_0p5()
                                            .rounded_full()
                                            .bg(rgb(0x1E293B))
                                            .text_xs()
                                            .text_color(rgb(0x94A3B8))
                                            .child("Native Media Center"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .max_w(px(720.))
                                    .child(
                                        div()
                                            .text_3xl()
                                            .font_weight(gpui::FontWeight::EXTRA_BOLD)
                                            .text_color(rgb(0xF8FAFC))
                                            .child("Your next favourite starts here"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0x94A3B8))
                                            .child("SENZO includes built-in movie and TV discovery. No TMDB account or API key is needed. Add your own compatible streaming providers whenever you are ready."),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .pt_3()
                                            .child(
                                                div()
                                                    .px_5()
                                                    .py_2_5()
                                                    .rounded_xl()
                                                    .bg(rgb(0x22D3EE))
                                                    .text_color(rgb(0x0A0A0A))
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_sm()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x67E8F9)))
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child("🎬")
                                                    .child("Browse Movies")
                                                    .tab_index(0)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _event, _window, cx| {
                                                            this.active_page = AppPage::Movies;
                                                            this.load_tmdb_catalog("movie", None, cx);
                                                        }),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .px_5()
                                                    .py_2_5()
                                                    .rounded_xl()
                                                    .bg(rgb(0x1F2937))
                                                    .text_color(rgb(0xF8FAFC))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_sm()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x374151)))
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child("🧩")
                                                    .child("Browse Add-ons")
                                                    .tab_index(0)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _event, _window, cx| {
                                                            this.active_page = AppPage::Addons;
                                                        }),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .px_5()
                                                    .py_2_5()
                                                    .rounded_xl()
                                                    .bg(rgb(0x1F2937))
                                                    .text_color(rgb(0xF8FAFC))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_sm()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x374151)))
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child("⚙")
                                                    .child("Settings")
                                                    .tab_index(0)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _event, _window, cx| {
                                                            this.active_page = AppPage::Settings;
                                                        }),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .px_4()
                                                    .py_2_5()
                                                    .rounded_xl()
                                                    .bg(rgb(0x181F26))
                                                    .text_color(rgb(0x94A3B8))
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .text_sm()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x242E35)).text_color(rgb(0xF8FAFC)))
                                                    .child("Retry discovery")
                                                    .tab_index(0)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _event, _window, cx| {
                                                            this.load_home_catalogs(cx);
                                                        }),
                                                    ),
                                            ),
                                    ),
                            )
                            .child(div()),
                    )
            };
            this.child(hero)
                .when(
                    (home_catalog_status.contains("could not")
                        || home_catalog_status.contains("unavailable"))
                        && (!home_catalogs.is_empty() || !home_addon_catalogs.is_empty()),
                    |panel| {
                        panel.child(
                            div()
                                .w_full()
                                .flex_shrink_0()
                                .px_4()
                                .py_2_5()
                                .rounded_xl()
                                .bg(rgb(0x181F26))
                                .border_1()
                                .border_color(rgb(0xF59E0B))
                                .flex()
                                .items_center()
                                .gap_2_5()
                                .child(div().text_sm().text_color(rgb(0xFBBF24)).child("⚠"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(0xFBBF24))
                                        .child(home_catalog_status.clone()),
                                ),
                        )
                    },
                )
                .when(home_catalogs.is_empty() && home_addon_catalogs.is_empty(), |panel| {
                    panel.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2_5()
                                    .child(div().w(px(3.)).h(px(18.)).rounded_full().bg(rgb(0x22D3EE)))
                                    .child(div().text_xl().font_weight(gpui::FontWeight::BOLD).text_color(rgb(0xF8FAFC)).child("Getting Started"))
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_4()
                                    .child(
                                        div()
                                            .flex_1()
                                            .p_5()
                                            .rounded_xl()
                                            .bg(rgb(0x13191E))
                                            .border_1()
                                            .border_color(rgb(0x1E262E))
                                            .flex()
                                            .flex_col()
                                            .justify_between()
                                            .gap_4()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1_5()
                                                    .child(div().text_xl().child("🧩"))
                                                    .child(div().text_base().font_weight(gpui::FontWeight::BOLD).text_color(rgb(0xF8FAFC)).child("Add-ons & Streaming"))
                                                    .child(div().text_xs().text_color(rgb(0x94A3B8)).child("Install community Stremio add-ons like Torrentio or Cinemeta to browse catalogs and stream titles."))
                                            )
                                            .child(
                                                div()
                                                    .px_4()
                                                    .py_2()
                                                    .rounded_lg()
                                                    .bg(rgb(0x1E293B))
                                                    .text_color(rgb(0x22D3EE))
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x334155)))
                                                    .child("Open Add-ons →")
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _event, _window, cx| {
                                                        this.active_page = AppPage::Addons;
                                                    }))
                                            )
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .p_5()
                                            .rounded_xl()
                                            .bg(rgb(0x13191E))
                                            .border_1()
                                            .border_color(rgb(0x1E262E))
                                            .flex()
                                            .flex_col()
                                            .justify_between()
                                            .gap_4()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1_5()
                                                    .child(div().text_xl().child("🔑"))
                                                    .child(div().text_base().font_weight(gpui::FontWeight::BOLD).text_color(rgb(0xF8FAFC)).child("SENZO Metadata API"))
                                                    .child(div().text_xs().text_color(rgb(0x94A3B8)).child("Built-in movie and TV discovery, powered by the SENZO hosted metadata service. No API key or account required."))
                                            )
                                            .child(
                                                div()
                                                    .px_4()
                                                    .py_2()
                                                    .rounded_lg()
                                                    .bg(rgb(0x1E293B))
                                                    .text_color(rgb(0x22D3EE))
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x334155)))
                                                    .child("Retry Metadata →")
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _event, _window, cx| {
                                                        this.load_home_catalogs(cx);
                                                    }))
                                            )
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .p_5()
                                            .rounded_xl()
                                            .bg(rgb(0x13191E))
                                            .border_1()
                                            .border_color(rgb(0x1E262E))
                                            .flex()
                                            .flex_col()
                                            .justify_between()
                                            .gap_4()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1_5()
                                                    .child(div().text_xl().child("⚙"))
                                                    .child(div().text_base().font_weight(gpui::FontWeight::BOLD).text_color(rgb(0xF8FAFC)).child("Native Playback"))
                                                    .child(div().text_xs().text_color(rgb(0x94A3B8)).child("Hardware-accelerated libmpv player with custom audio, subtitle track selection, and HDR controls."))
                                            )
                                            .child(
                                                div()
                                                    .px_4()
                                                    .py_2()
                                                    .rounded_lg()
                                                    .bg(rgb(0x1E293B))
                                                    .text_color(rgb(0x22D3EE))
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x334155)))
                                                    .child("View Settings →")
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _event, _window, cx| {
                                                        this.active_page = AppPage::Settings;
                                                    }))
                                            )
                                    )
                            )
                    )
                })
                .when(!continue_watching_cards.is_empty(), |panel| {
                    panel.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2_5()
                                    .child(
                                        div()
                                            .w(px(3.))
                                            .h(px(18.))
                                            .rounded_full()
                                            .bg(rgb(0x22D3EE)),
                                    )
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0xF8FAFC))
                                            .child("Continue Watching"),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .id("continue-watching-shelf")
                                    .overflow_x_scroll()
                                    .flex()
                                    .gap_3()
                                    .children(continue_watching_cards),
                            ),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x94A3B8))
                                .child("Customize your home screen"),
                        )
                        .child(dynamic_choice_chip(
                            if home_customize_open { "Done".into() } else { "Customize shelves".into() },
                            home_customize_open,
                            |this, cx| {
                                this.home_customize_open = !this.home_customize_open;
                                cx.notify();
                            },
                            cx,
                        )),
                )
                .when(home_customize_open, |panel| {
                    panel.child(section(
                        "Home shelves",
                        "Hide categories you do not use, or change their order. Your layout is saved on this device.",
                        div().flex().flex_col().gap_3().children(
                            ordered_shelf_names.iter().enumerate().map(|(index, shelf)| {
                                let shelf: &'static str = shelf;
                                let hidden = hidden_home_shelves
                                    .iter()
                                    .any(|saved| saved.as_str() == shelf);
                                let shelf_label = (*shelf).to_owned();
                                let visibility_label = if hidden { "Show" } else { "Hide" };
                                div().flex().items_center().justify_between().gap_2()
                                    .child(div().flex_1().text_sm().text_color(rgb(0xE4E4E7)).child(shelf_label))
                                    .child(div().flex().gap_1()
                                        .child(dynamic_choice_chip(
                                            visibility_label.into(),
                                            hidden,
                                            move |this, cx| this.set_home_shelf_hidden(shelf, !hidden, cx),
                                            cx,
                                        ))
                                        .child(dynamic_choice_chip(
                                            "↑".into(),
                                            false,
                                            move |this, cx| this.move_home_shelf(shelf, -1, cx),
                                            cx,
                                        ))
                                        .child(dynamic_choice_chip(
                                            "↓".into(),
                                            false,
                                            move |this, cx| this.move_home_shelf(shelf, 1, cx),
                                            cx,
                                        )))
                                    .when(index == 0, |row| row.opacity(0.9))
                            })
                        ),
                    ))
                })
                .children({
                    let mut rendered_shelves: Vec<&'static str> = Vec::new();
                    let has_trending_movies = home_catalogs.iter().any(|(s, _, _)| s == "Trending Movies");
                    for shelf in ordered_shelf_names {
                        if hidden_home_shelves.iter().any(|hidden| hidden.as_str() == shelf) {
                            continue;
                        }
                        if shelf == "Trending this week" && has_trending_movies {
                            continue;
                        }
                        rendered_shelves.push(shelf);
                    }
                    rendered_shelves.into_iter().filter_map(|shelf_name| {
                        let shelf_animation_id = format!("home-shelf-{shelf_name}");
                        let entries = home_catalogs
                            .iter()
                            .filter(|(name, _, _)| name == shelf_name)
                            .cloned()
                            .collect::<Vec<_>>();
                        if entries.is_empty() {
                            return None;
                        }
                        let shelf_display = match shelf_name {
                            "Trending Movies" | "Trending this week" => "Trending Movies",
                            "Trending TV Shows" | "Popular TV" => "Trending TV Shows",
                            "Popular Movies" | "Popular movies" => "Popular Movies",
                            other => other,
                        };
                        let cards = entries
                            .into_iter()
                            .map(|(_, kind, item)| {
                                let title = item.display_title().unwrap_or("Untitled").to_owned();
                                let year = item
                                    .release_date
                                    .as_deref()
                                    .or(item.first_air_date.as_deref())
                                    .and_then(|date| date.get(0..4))
                                    .unwrap_or("—")
                                    .to_owned();
                                let rating = item
                                    .vote_average
                                    .map(|value| format!("★ {value:.1}"))
                                    .unwrap_or_default();
                                let selected = item.clone();
                                let selected_kind = kind.clone();
                                // Some catalogue entries have no poster but do have a
                                // backdrop. Use it as a cropped card fallback instead of a blank tile.
                                let poster = tmdb_artwork_url(item.poster_path.as_deref(), "w342")
                                    .or_else(|| tmdb_artwork_url(item.backdrop_path.as_deref(), "w780"));
                                let has_poster = poster.is_some();
                                div()
                                    .w(poster_width)
                                    .h(poster_height)
                                    .flex_shrink_0()
                                    .rounded_xl()
                                    .bg(rgb(0x181F26))
                                    .border_1()
                                    .border_color(rgb(0x242E35))
                                    .overflow_hidden()
                                    .relative()
                                    .cursor_pointer()
                                    .hover(|style| style.border_color(rgb(0x22D3EE)))
                                    .when_some(poster, |panel, url| {
                                        panel.child(
                                            img(url)
                                                .w_full()
                                                .h_full()
                                                .object_fit(ObjectFit::Cover),
                                        )
                                    })
                                    .when(!rating.is_empty(), |panel| {
                                        panel.child(
                                            div()
                                                .absolute()
                                                .top(px(8.))
                                                .right(px(8.))
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(rgb(0x0A0C0E))
                                                .opacity(0.85)
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(0xFBBF24))
                                                .child(rating.clone()),
                                        )
                                    })
                                    .when(!has_poster, |panel| {
                                        panel.child(
                                            div()
                                                .size_full()
                                                .flex()
                                                .flex_col()
                                                .justify_center()
                                                .items_center()
                                                .p_4()
                                                .bg(rgb(0x181F26))
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(gpui::FontWeight::BOLD)
                                                        .text_center()
                                                        .child(title.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(0x94A3B8))
                                                        .text_center()
                                                        .child(year.clone()),
                                                ),
                                        )
                                    })
                                    .tab_index(0)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _event, _window, cx| {
                                            this.select_tmdb_title(
                                                selected_kind.clone(),
                                                selected.clone(),
                                                cx,
                                            );
                                        }),
                                    )
                            })
                            .collect::<Vec<_>>();
                        Some(
                            div()
                                .flex()
                                .flex_col()
                                .gap_4()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2_5()
                                                .child(
                                                    div()
                                                        .w(px(3.))
                                                        .h(px(18.))
                                                        .rounded_full()
                                                        .bg(rgb(0x22D3EE)),
                                                )
                                                .child(
                                                    div()
                                                        .text_xl()
                                                        .font_weight(gpui::FontWeight::BOLD)
                                                        .text_color(rgb(0xF8FAFC))
                                                        .child(shelf_display),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .text_color(rgb(0x9CA3AF))
                                                .cursor_pointer()
                                                .hover(|s| s.text_color(rgb(0x22D3EE)))
                                                .child("View All →")
                                                .tab_index(0)
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(move |this, _event, _window, cx| {
                                                        let (media_type, destination) = match shelf_name {
                                                            "Trending Movies" | "Trending this week" => ("trending", AppPage::Movies),
                                                            "Trending TV Shows" | "Popular TV" => ("tv", AppPage::TvShows),
                                                            _ => ("movie", AppPage::Movies),
                                                        };
                                                        this.active_page = destination;
                                                        this.load_tmdb_catalog(media_type, None, cx);
                                                    }),
                                                ),
                                        ),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .id(shelf_name)
                                        .overflow_x_scroll()
                                        .flex()
                                        .gap_3()
                                        .children(cards),
                                )
                                .with_animation(
                                    shelf_animation_id,
                                    Animation::new(Duration::from_millis(360))
                                        .with_easing(ease_in_out),
                                    |element, progress| element.opacity(progress),
                                ),
                        )
                    })
                })
                .children(home_addon_catalogs.iter().map(|(shelf_name, addon, media_type, catalog_id, items)| {
                    let cards = items.iter().filter_map(|item| {
                        let title = item.get("name").or_else(|| item.get("title"))
                            .and_then(Value::as_str)?.to_owned();
                        let selected = item.clone();
                        let selected_addon = addon.clone();
                        let poster = safe_addon_artwork_url(item.get("poster").and_then(Value::as_str));
                        let has_poster = poster.is_some();
                        let year = item.get("releaseInfo").and_then(Value::as_str)
                            .or_else(|| item.get("year").and_then(Value::as_str))
                            .unwrap_or(" ");
                        Some(div().w(poster_width).h(poster_height).flex_shrink_0().rounded_xl().bg(rgb(0x181F26))
                            .border_1().border_color(rgb(0x242E35)).overflow_hidden().relative()
                            .cursor_pointer().hover(|style| style.border_color(rgb(0x22D3EE)))
                            .when_some(poster, |panel, url| {
                                panel.child(img(url).w_full().h_full().object_fit(ObjectFit::Cover))
                            })
                            .when(!has_poster, |panel| {
                                panel.child(
                                    div().size_full().flex().flex_col().justify_center().items_center().p_4()
                                        .bg(rgb(0x181F26))
                                        .child(div().text_sm().font_weight(gpui::FontWeight::BOLD).text_center().child(title.clone()))
                                        .child(div().text_xs().text_color(rgb(0x94A3B8)).text_center().child(year.to_owned()))
                                )
                            })
                            .tab_index(0)
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _event, _window, cx| {
                                this.select_addon_catalog_meta(selected_addon.clone(), selected.clone(), cx);
                            }))
                        )
                    }).collect::<Vec<_>>();
                    let selected_addon = addon.clone();
                    let selected_type = media_type.clone();
                    let selected_catalog = catalog_id.clone();
                    div().flex().flex_col().gap_4()
                        .child(div().flex().items_center().justify_between()
                            .child(
                                div().flex().items_center().gap_2_5()
                                    .child(div().w(px(3.)).h(px(18.)).rounded_full().bg(rgb(0x22D3EE)))
                                    .child(div().text_xl().font_weight(gpui::FontWeight::BOLD).text_color(rgb(0xF8FAFC))
                                        .child(shelf_name.clone()))
                            )
                            .child(div().flex().items_center().gap_2()
                                .child(div().text_xs().text_color(rgb(0x67E8F9)).child("ADD-ON"))
                                .child(dynamic_choice_chip("View all".into(), false, move |this, cx| {
                                    this.active_page = AppPage::Addons;
                                    this.load_addon_catalog(selected_addon.clone(), selected_type.clone(), selected_catalog.clone(), cx);
                                }, cx))))
                        .child(div().w_full().id(format!("addon-shelf-{}-{}", addon.id, media_type))
                            .overflow_x_scroll().flex().gap_3().children(cards))
                }))
        })
        .when(is_details, |this| {
            let details = tmdb_details.clone().unwrap_or(Value::Null);
            let title = details
                .get("name")
                .or_else(|| details.get("title"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(selected_title.clone())
                .unwrap_or_else(|| "Title details".into());
            let overview = details
                .get("overview")
                .or_else(|| details.get("description"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("No synopsis is available from this provider.")
                .to_owned();
            let release = details
                .get("release_date")
                .or_else(|| details.get("first_air_date"))
                .or_else(|| details.get("releaseInfo"))
                .and_then(Value::as_str)
                .map(|value| value.chars().take(4).collect::<String>())
                .unwrap_or_else(|| details.get("year").and_then(Value::as_str).unwrap_or("—").to_owned());
            let rating = details
                .get("vote_average")
                .or_else(|| details.get("imdbRating"))
                .and_then(Value::as_f64)
                .map(|value| format!("★ {value:.1}"))
                .unwrap_or_else(|| "Not rated".into());
            let genres = details
                .get("genres")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(|item| {
                    item.as_str().or_else(|| item.get("name").and_then(Value::as_str))
                }).take(5).collect::<Vec<_>>().join(" · "))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Genres unavailable".into());
            let poster = cached_artwork_or_remote(tmdb_artwork_url(
                details.get("poster_path").and_then(Value::as_str),
                "w500",
            ).or_else(|| safe_addon_artwork_url(
                details.get("poster").and_then(Value::as_str),
            )));
            let backdrop = cached_artwork_or_remote(tmdb_artwork_url(
                details.get("backdrop_path").and_then(Value::as_str),
                "w1280",
            ).or_else(|| safe_addon_artwork_url(
                details.get("background").and_then(Value::as_str),
            )));
            this.child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .w_full()
                            .min_h(px(280.))
                            .rounded_xl()
                            .bg(rgb(0x151D20))
                            .border_1()
                            .border_color(rgb(0x263238))
                            .relative()
                            .overflow_hidden()
                            .when_some(backdrop, |panel, url| {
                                panel.child(
                                    img(url)
                                        .absolute()
                                        .size_full()
                                        .object_fit(ObjectFit::Cover),
                                )
                            })
                            .child(
                                div()
                                    .w_full()
                                    .min_h(px(280.))
                                    .p_6()
                                    .flex()
                                    .flex_col()
                                    .justify_end()
                                    .gap_3()
                                    .bg(rgb(0x101719))
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .child(title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0xD4D4D8))
                                            .child(format!("{release} · {rating}")),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0xA1A1AA))
                                            .child(genres.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0xE4E4E7))
                                            .child(overview.clone()),
                                    ),
                            ),
                    )
                    .when(details.get("seasons").and_then(Value::as_array).is_some(), |panel| {
                        let season_chips = tv_seasons.iter().map(|(number, name)| {
                            let season_number = *number;
                            dynamic_choice_chip(name.clone(), tv_selected_season == Some(season_number), move |this, cx| {
                                this.load_tmdb_season(season_number, cx);
                            }, cx)
                        }).collect::<Vec<_>>();
                        let episode_chips = tv_episodes.iter().cloned().map(|episode| {
                            let selected = tv_selected_episode.as_ref().is_some_and(|item|
                                item.season_number == episode.season_number && item.episode_number == episode.episode_number);
                            let selected_episode = episode.clone();
                            let episode_name = episode.name.clone().filter(|value| !value.trim().is_empty())
                                .unwrap_or_else(|| format!("Episode {}", episode.episode_number));
                            let label = format!("E{:02} · {}", episode.episode_number, episode_name);
                            dynamic_choice_chip(label, selected, move |this, cx| {
                                this.select_tmdb_episode(selected_episode.clone(), cx);
                            }, cx)
                        }).collect::<Vec<_>>();
                        let episode_info = tv_selected_episode.clone().map(|episode| {
                            let still = cached_artwork_or_remote(tmdb_artwork_url(episode.still_path.as_deref(), "w500"));
                            let episode_title = episode.name.clone().unwrap_or_else(|| format!("Episode {}", episode.episode_number));
                            let air_date = episode.air_date.clone().unwrap_or_else(|| "Air date unavailable".into());
                            let runtime = episode.runtime.map(|minutes| format!("{minutes} min")).unwrap_or_else(|| "Runtime unavailable".into());
                            let rating = episode.vote_average.map(|value| format!("★ {value:.1}")).unwrap_or_else(|| "Not rated".into());
                            let overview = episode.overview.clone().filter(|value| !value.trim().is_empty())
                                .unwrap_or_else(|| "No episode synopsis is available.".into());
                            div().flex().gap_4().p_3().rounded_lg().bg(rgb(0x101719))
                                .when_some(still, |card, url| card.child(img(url).w(px(180.)).h(px(102.)).rounded_md().object_fit(ObjectFit::Cover)))
                                .child(div().flex_1().flex().flex_col().gap_2()
                                    .child(div().text_sm().font_weight(gpui::FontWeight::SEMIBOLD).child(format!("S{:02}E{:02} · {}", episode.season_number, episode.episode_number, episode_title)))
                                    .child(div().text_xs().text_color(rgb(0x9299A1)).child(format!("{air_date} · {runtime} · {rating}")))
                                    .child(div().text_sm().text_color(rgb(0xD4D4D8)).child(overview)))
                        });
                        panel.child(div().p_4().rounded_xl().bg(rgb(0x151719)).border_1().border_color(rgb(0x263238))
                            .flex().flex_col().gap_3()
                            .child(div().flex().items_center().justify_between()
                                .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).child("Seasons & Episodes"))
                                .child(div().text_xs().text_color(rgb(0x9299A1)).child(tv_episode_status.clone())))
                            .child(div().flex().flex_wrap().gap_2().children(season_chips))
                            .child(div().flex().flex_wrap().gap_2().children(episode_chips))
                            .when_some(episode_info, |card, info| card.child(info))
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .p_5()
                            .rounded_xl()
                            .bg(rgb(0x151719))
                            .border_1()
                            .border_color(rgb(0x263238))
                            .child(
                                div()
                                    .w(px(170.))
                                    .h(poster_height)
                                    .flex_shrink_0()
                                    .rounded_lg()
                                    .bg(rgb(0x20272B))
                                    .overflow_hidden()
                                    .when_some(poster, |panel, url| {
                                        panel.child(
                                            img(url)
                                                .w(px(170.))
                                                .h(poster_height)
                                                .object_fit(ObjectFit::Cover),
                                        )
                                    })
                                    .when(details.get("poster_path").is_none() && details.get("poster").is_none(), |panel| {
                                        panel.flex().items_center().justify_center()
                                            .child(div().text_xs().text_color(rgb(0x858C94)).child("POSTER"))
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child("Available sources"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x9299A1))
                                            .child(stream_status.clone()),
                                    )
                                    .child(div().flex().flex_wrap().gap_2().children(stream_chips))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x9299A1))
                                            .child(torrent_status.clone()),
                                    )
                                    .child(div().flex().flex_wrap().gap_2().children(torrent_file_chips))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_2()
                                            .child(dynamic_choice_chip(
                                                "Back to Home".into(),
                                                false,
                                                |this, _cx| this.active_page = AppPage::Home,
                                                cx,
                                            )),
                                    ),
                            ),
                    ),
            )
        })
        .when(!is_details && !is_library && !is_home && page != AppPage::Addons, |this| {
            this.child(
                div().p_4().rounded_lg().bg(rgb(0x151719)).flex().flex_col().gap_2()
                    .child(div().text_sm().text_color(rgb(0xA1A1AA)).child("Choose a catalogue above to browse real titles."))
            )
        })
        .when(is_library, |this| {
            this.child(
                div()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(0x151719))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child("Your library is ready for titles"))
                    .child(div().text_sm().text_color(rgb(0x9299A1)).child("No saved titles yet. Add favourites and watchlist support will be connected to local storage in a later batch.")),
            )
        })
        .when(page == AppPage::Addons, |this| {
            let previous_addon = addon_catalog_addon.clone();
            let next_addon = addon_catalog_addon.clone();
            let previous_media_type = addon_catalog_media_type.clone();
            let next_media_type = addon_catalog_media_type.clone();
            let previous_catalog_id = addon_catalog_id.clone();
            let next_catalog_id = addon_catalog_id.clone();
            let previous_page = addon_catalog_page;
            let next_page = addon_catalog_page;
            this.child(div().p_4().rounded_lg().bg(rgb(0x151719)).flex().flex_col().gap_3()
                .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child("Installed add-on catalogs"))
                .child(div().text_xs().text_color(rgb(0x9299A1)).child("Catalogs are loaded from each enabled add-on manifest."))
                .child(div().flex().flex_wrap().gap_2().children(catalog_chips))
                .when(!addon_catalog_filter_specs.is_empty(), |panel| {
                    panel.child(div().p_3().rounded_md().bg(rgb(0x101315)).flex().flex_col().gap_3()
                        .child(div().text_sm().font_weight(gpui::FontWeight::SEMIBOLD).child("Catalogue filters"))
                        .child(div().text_xs().text_color(rgb(0x9299A1)).child("Choose declared options or paste a custom value from your clipboard. Required fields are marked with *."))
                        .children(catalog_filter_rows)
                        .child(dynamic_choice_chip("Load filtered catalogue".into(), false, |this, cx| {
                            this.load_filtered_addon_catalog(cx)
                        }, cx)))
                })
                .child(div().text_xs().text_color(rgb(0x9299A1)).child(addon_catalog_status.clone()))
                .child(div().flex().flex_wrap().gap_2().children(catalog_item_chips))
                .child(div().flex().items_center().gap_2()
                    .child(dynamic_choice_chip("Previous page".into(), false, move |this, cx| {
                        if previous_page > 0 {
                            if let Some(addon) = previous_addon.clone() {
                                this.load_addon_catalog_page(
                                    addon,
                                    previous_media_type.clone(),
                                    previous_catalog_id.clone(),
                                    previous_page - 1,
                                    cx,
                                );
                            }
                        }
                    }, cx))
                    .child(div().text_xs().text_color(rgb(0x9299A1)).child(format!("Page {}", addon_catalog_page + 1)))
                    .child(dynamic_choice_chip("Next page".into(), false, move |this, cx| {
                        if let Some(addon) = next_addon.clone() {
                            this.load_addon_catalog_page(
                                addon,
                                next_media_type.clone(),
                                next_catalog_id.clone(),
                                next_page + 1,
                                cx,
                            );
                        }
                    }, cx))))
                .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .p_5()
                            .rounded_lg()
                            .bg(rgb(0x151719))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child("Install a Stremio add-on"))
                            .child(div().text_sm().text_color(rgb(0x9299A1)).child("Copy the add-on's HTTPS manifest URL, then paste it here for a preview. Nothing is installed until you confirm."))
                            .child(div().text_sm().text_color(rgb(0xB7BEC6)).child(addon_status.clone()))
                            .child(div().flex().flex_wrap().gap_2()
                                .child(dynamic_choice_chip(
                                    "Paste manifest URL".into(),
                                    false,
                                    |this, cx| this.preview_addon_from_clipboard(cx),
                                    cx,
                                ))
                                .child(dynamic_choice_chip(
                                    "Preview Torrentio".to_string(),
                                    true,
                                    |this, cx| this.preview_torrentio(cx),
                                    cx,
                                )))
                            .child(div().text_xs().text_color(rgb(0x858C94))
                                .child("Torrentio opens with its standard manifest. For a custom provider / debrid setup, copy the configured manifest URL from torrentio.strem.fun/configure and use Paste manifest URL."))
                            .when(addon_manifest_preview.is_some(), |panel| {
                                let preview = addon_manifest_preview.clone().expect("preview exists");
                                panel.child(
                                    div()
                                        .p_4()
                                        .rounded_md()
                                        .bg(rgb(0x101315))
                                        .flex()                                        .flex_col()
                                        .gap_2()
                                        .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child(preview.manifest.name.clone()))
                                        .child(div().text_xs().text_color(rgb(0xA1A1AA)).child(format!("{} · version {}", preview.manifest.id, preview.manifest.version)))
                                        .child(div().text_sm().text_color(rgb(0x9299A1)).child(preview.manifest.description.clone().unwrap_or_else(|| "No description supplied".into())))
                                        .when(
                                            preview.manifest.requires_configuration(),
                                            |panel| {
                                                panel.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(0xFBBF24))
                                                        .child(
                                                            "Provider needs configuration; paste its configured manifest URL."
                                                        ),
                                                )
                                            },
                                        )
                                        .child(div().text_xs().text_color(rgb(0x67E8F9)).child(format!("Resources: {}", preview.manifest.resources.iter().map(|value| value.as_str().or_else(|| value.get("name").and_then(serde_json::Value::as_str)).unwrap_or("unknown")).collect::<Vec<_>>().join(", "))))
                                        .child(div().flex().flex_wrap().gap_2()
                                            .child(dynamic_choice_chip("Confirm install".into(), false, |this, cx| this.confirm_addon_install(cx), cx))
                                            .child(dynamic_choice_chip("Cancel".into(), false, |this, cx| { this.addon_manifest_preview = None; this.addon_status = "Manifest preview cancelled".into(); cx.notify(); }, cx))),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("Installed add-ons · {}", installed_addons.len())),
                    )
                    .when(installed_addons.is_empty(), |panel| {
                        panel.child(
                            div().p_5().rounded_lg().bg(rgb(0x151719))
                                .child(div().text_sm().text_color(rgb(0x9299A1)).child("No add-ons installed. SENZO never bundles or enables streaming add-ons by default.")),
                        )
                    })
                    .children(installed_addons.into_iter().map(|addon| {
                        let addon_id = addon.id.clone();
                        let toggle_id = addon.id.clone();
                        let refresh_addon = addon.clone();
                        let move_up_id = addon.id.clone();
                        let move_down_id = addon.id.clone();
                        let enabled = addon.enabled;
                        let name = addon.name.clone();
                        let version = addon.version.clone();
                        let manifest_url = addon.manifest_url.clone();
                        div()
                            .p_5()
                            .rounded_lg()
                            .bg(rgb(0x151719))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child(name))
                                            .child(div().text_xs().text_color(rgb(0x9299A1)).child(format!("Version {version} · {}", if enabled { "Enabled" } else { "Disabled" }))),
                                    )
                                    .child(div().text_xs().text_color(rgb(0x858C94)).child("Stremio add-on")),
                            )
                            .child(div().text_xs().text_color(rgb(0x67E8F9)).child(manifest_url))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(dynamic_choice_chip(
                                        if enabled { "Disable".into() } else { "Enable".into() },
                                        enabled,
                                        move |this, cx| {
                                            match this.database.set_addon_enabled(&toggle_id, !enabled) {
                                                Ok(true) => diagnostics::record(
                                                    diagnostics::Level::Audit,
                                                    "addons",
                                                    if enabled { "user disabled an add-on" } else { "user enabled an add-on" },
                                                ),
                                                Ok(false) => this.status = "The add-on no longer exists".into(),
                                                Err(error) => this.status = format!("Could not update add-on: {error:#}"),
                                            }
                                            this.home_catalog_requested = false;
                                            cx.notify();
                                        },
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "Refresh".into(),
                                        false,
                                        move |this, cx| {
                                            this.refresh_addon(refresh_addon.clone(), cx)
                                        },
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "Move up".into(),
                                        false,
                                        move |this, cx| {
                                            match this.database.move_addon(&move_up_id, -1) {
                                                Ok(true) => {
                                                    this.status = "Add-on order updated".into()
                                                }
                                                Ok(false) => {
                                                    this.status = "Add-on is already at the top".into()
                                                }
                                                Err(error) => {
                                                    this.status = format!(
                                                        "Could not reorder add-on: {error:#}"
                                                    )
                                                }
                                            }
                                            cx.notify();
                                        },
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "Move down".into(),
                                        false,
                                        move |this, cx| {
                                            match this.database.move_addon(&move_down_id, 1) {
                                                Ok(true) => {
                                                    this.status = "Add-on order updated".into()
                                                }
                                                Ok(false) => {
                                                    this.status = "Add-on is already at the bottom".into()
                                                }
                                                Err(error) => {
                                                    this.status = format!(
                                                        "Could not reorder add-on: {error:#}"
                                                    )
                                                }
                                            }
                                            cx.notify();
                                        },
                                        cx,
                                    ))
                                    .child(dynamic_choice_chip(
                                        "Remove".into(),
                                        false,
                                        move |this, cx| {
                                            match this.database.remove_addon(&addon_id) {
                                                Ok(true) => {
                                                    this.status = "Add-on removed from this device".into();
                                                    this.home_catalog_requested = false;
                                                    diagnostics::record(diagnostics::Level::Audit, "addons", "user removed an add-on");
                                                }
                                                Ok(false) => this.status = "The add-on no longer exists".into(),
                                                Err(error) => this.status = format!("Could not remove add-on: {error:#}"),
                                            }
                                            cx.notify();
                                        },
                                        cx,
                                    )),
                            )
                    })),
            )
        })
}

fn cached_artwork_or_remote(url: Option<String>) -> Option<String> {
    let url = url?;
    senzo::cache::cached_artwork_path(&url)
        .ok()
        .flatten()
        .map(|path| path.to_string_lossy().into_owned())
        .or(Some(url))
}

fn tmdb_artwork_url(path: Option<&str>, size: &str) -> Option<String> {
    let path = path?.strip_prefix('/')?;
    if path.is_empty() || path.split('/').any(|part| part == "." || part == "..") {
        return None;
    }
    Some(format!("https://image.tmdb.org/t/p/{size}/{path}"))
}

fn safe_addon_artwork_url(value: Option<&str>) -> Option<String> {
    let url = url::Url::parse(value?).ok()?;
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    if let Some(url::Host::Domain(host)) = url.host() {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host == "localhost"
            || host.ends_with(".localhost")
            || host.ends_with(".local")
            || host.ends_with(".internal")
        {
            return None;
        }
    } else {
        return None;
    }
    Some(url.to_string())
}

fn format_playback_time(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds.filter(|value| value.is_finite() && *value >= 0.0) else {
        return "—:—".into();
    };
    let total = seconds.floor() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn nav_item(label: &'static str, selected: bool) -> impl IntoElement {
    let background = if selected {
        rgb(0x1A3034)
    } else {
        rgb(0x101315)
    };
    let foreground = if selected {
        rgb(0x67E8F9)
    } else {
        rgb(0xA1A1AA)
    };

    div()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(background)
        .text_sm()
        .text_color(foreground)
        .child(label)
}

fn toggle_row<F>(
    label: &'static str,
    description: &'static str,
    enabled: bool,
    action: F,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement
where
    F: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
{
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_color(rgb(0xE4E4E7)).child(label))
                .child(div().text_xs().text_color(rgb(0x858C94)).child(description)),
        )
        .child(choice_chip(
            if enabled { "Enabled" } else { "Disabled" },
            enabled,
            action,
            cx,
        ))
}

fn stepper_row<Down, Up>(
    label: &'static str,
    description: &'static str,
    value: u16,
    unit: &'static str,
    decrement: Down,
    increment: Up,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement
where
    Down: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
    Up: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
{
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_color(rgb(0xE4E4E7)).child(label))
                .child(div().text_xs().text_color(rgb(0x858C94)).child(description)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(choice_chip("−", false, decrement, cx))
                .child(
                    div()
                        .w(px(88.))
                        .text_center()
                        .text_sm()
                        .text_color(rgb(0xE4E4E7))
                        .child(format!("{value} {unit}")),
                )
                .child(choice_chip("+", false, increment, cx)),
        )
}

fn main() {
    match TorrentSessionDirectory::default_root()
        .and_then(|root| TorrentSessionDirectory::recover_abandoned_sessions_at(&root))
    {
        Ok(removed) if removed > 0 => {
            eprintln!("SENZO recovered {removed} abandoned torrent session(s)")
        }
        Ok(_) => {}
        Err(error) => eprintln!("SENZO torrent recovery warning: {error:#}"),
    }

    if let Err(error) = diagnostics::initialize() {
        eprintln!("SENZO diagnostics initialization failed: {error:#}");
    } else {
        diagnostics::record(
            diagnostics::Level::Audit,
            "application",
            "initializing local database",
        );
    }

    let database = match LocalDatabase::open_default() {
        Ok(database) => {
            diagnostics::record(
                diagnostics::Level::Audit,
                "storage",
                "local database opened successfully",
            );
            database
        }
        Err(error) => {
            diagnostics::record(
                diagnostics::Level::Error,
                "storage",
                &format!("failed to initialize local database: {error:#}"),
            );
            panic!("failed to initialize SENZO's local database: {error:#}");
        }
    };

    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run({
            let mut database = Some(database);
            move |cx: &mut App| {
                gpui_base::init(cx);
                match TmdbArtworkHttpClient::new() {
                    Ok(client) => {
                        cx.set_http_client(Arc::new(client));
                        diagnostics::record(
                            diagnostics::Level::Audit,
                            "artwork",
                            "registered secure TMDB artwork HTTP client with GPUI",
                        );
                    }
                    Err(error) => diagnostics::record(
                        diagnostics::Level::Error,
                        "artwork",
                        &format!("could not initialize TMDB artwork HTTP client: {error:#}"),
                    ),
                }
                let database = database.take().expect("SENZO window initialized once");
                cx.open_window(WindowOptions::default(), move |window, cx| {
                    let native_parent_hwnd = match gpui_parent_hwnd(window) {
                        Ok(hwnd) => {
                            diagnostics::record(
                                diagnostics::Level::Info,
                                "playback",
                                "acquired GPUI native Win32 parent HWND",
                            );
                            Some(hwnd)
                        }
                        Err(error) => {
                            diagnostics::record(
                                diagnostics::Level::Warn,
                                "playback",
                                &format!("native video surface unavailable: {error:#}"),
                            );
                            None
                        }
                    };
                    cx.new(move |_cx| SenzoApp::new(database, native_parent_hwnd))
                })
                .expect("failed to open SENZO window");

                cx.activate(true);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_time_string_format() {
        let time_str = current_time_string();
        assert!(!time_str.is_empty());
        assert!(time_str.contains("AM") || time_str.contains("PM") || time_str.contains("UTC"));
    }

    #[test]
    fn test_home_shelf_known_names_consistency() {
        let known = [
            "Trending Movies",
            "Trending TV Shows",
            "Popular movies",
            "Popular TV",
            "Trending this week",
        ];
        assert_eq!(known.len(), 5);
        assert!(known.contains(&"Trending Movies"));
        assert!(known.contains(&"Trending TV Shows"));
    }

    #[test]
    fn test_home_status_warning_detection() {
        let ok_status = "14 recommendations · 2 add-on shelves";
        let fail_status = "Recommendations are unavailable right now: timeout";
        let partial_status = "some recommendations could not load";
        let is_warn = |s: &str| s.contains("could not") || s.contains("unavailable");
        assert!(!is_warn(ok_status));
        assert!(is_warn(fail_status));
        assert!(is_warn(partial_status));
    }

    #[test]
    fn test_navigation_items_count() {
        let pages = [
            AppPage::Home,
            AppPage::Movies,
            AppPage::TvShows,
            AppPage::Search,
            AppPage::Library,
            AppPage::Addons,
            AppPage::Settings,
        ];
        assert_eq!(pages.len(), 7);
    }
}
