use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context as _;
use gpui::{
    App, ClipboardItem, Context, MouseButton, QuitMode, Render, Task, Window, WindowOptions, div,
    prelude::*, px, rgb,
};
use serde_json::Value;

use senzo::{
    addons::{AddonClient, AddonManifest, AddonManifestPreview, AddonStreamResult, StremioStream},
    diagnostics,
    metadata::{TmdbClient, TmdbCredentialStore, TmdbSearchResult},
    playback::{
        mpv_config::{MpvCapabilities, build_mpv_config},
        mpv_worker::{PlaybackEvent, PlaybackState, PlaybackWorker},
        windows_surface::{SurfaceBounds, WindowsVideoSurface, gpui_parent_hwnd},
    },
    settings::{
        DecoderPreference, HardwareDecoding, HdrMode, PlaybackSettings, RendererPreference,
        ScalingAlgorithm,
    },
    storage::{LocalDatabase, StoredAddon},
    torrent::TorrentSessionDirectory,
    torrent_engine::{TorrentPlaybackSession, TorrentProgress, TorrentVideoFile},
    subtitles::{
        OpenSubtitlesClient, OpenSubtitlesCredentialStore, OpenSubtitlesResult, SubtitleSearch,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum AppPage {
    Home,
    Movies,
    TvShows,
    Search,
    Library,
    Details,
    Addons,
    Settings,
}

struct SenzoApp {
    database: LocalDatabase,
    active_page: AppPage,
    selected_title: Option<String>,
    search_query: String,
    settings: PlaybackSettings,
    status: String,
    playback_worker: Option<Arc<Mutex<PlaybackWorker>>>,
    playback_state: PlaybackState,
    playback_status: String,
    active_media_id: Option<String>,
    last_progress_save: Option<Instant>,
    playback_monitor: Option<Task<()>>,
    online_subtitle_results: Vec<OpenSubtitlesResult>,
    subtitle_provider_status: String,
    subtitle_credentials_status: String,
    tmdb_credentials_status: String,
    subtitle_task: Option<Task<()>>,
    addon_manifest_preview: Option<AddonManifestPreview>,
    addon_status: String,
    addon_task: Option<Task<()>>,
    tmdb_results: Vec<(String, TmdbSearchResult)>,
    tmdb_status: String,
    tmdb_details: Option<Value>,
    tmdb_task: Option<Task<()>>,
    addon_catalog_items: Vec<Value>,
    addon_catalog_status: String,
    addon_catalog_addon: Option<StoredAddon>,
    addon_stream_results: Vec<AddonStreamResult>,
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

        Self {
            database,
            active_page: AppPage::Settings,
            selected_title: None,
            search_query: String::new(),
            settings,
            status: "Changes are saved on this device".into(),
            playback_worker: None,
            playback_state: PlaybackState::default(),
            playback_status: "MPV runtime not found (expected runtime/libmpv-2.dll)".into(),
            active_media_id: None,
            last_progress_save: None,
            playback_monitor: None,
            online_subtitle_results: Vec::new(),
            subtitle_provider_status: "Online subtitles have not been searched".into(),
            subtitle_credentials_status: match OpenSubtitlesCredentialStore::has_api_key() {
                Ok(true) => "API key saved in Windows Credential Manager".into(),
                Ok(false) => "No API key saved in Windows Credential Manager".into(),
                Err(_) => "Could not inspect Windows Credential Manager".into(),
            },
            tmdb_credentials_status: match TmdbCredentialStore::has_access_token() {
                Ok(true) => "TMDB Read Access Token saved in Windows Credential Manager".into(),
                Ok(false) => {
                    "TMDB is not configured; add a Read Access Token to enable discovery".into()
                }
                Err(_) => {
                    "Could not inspect Windows Credential Manager for TMDB credentials".into()
                }
            },
            subtitle_task: None,
            addon_manifest_preview: None,
            addon_status: "Copy a Stremio manifest URL, then choose Paste URL to preview it".into(),
            addon_task: None,
            tmdb_results: Vec::new(),
            tmdb_status: "Configure a TMDB Read Access Token in Settings, then load a catalogue"
                .into(),
            tmdb_details: None,
            tmdb_task: None,
            addon_catalog_items: Vec::new(),
            addon_catalog_status: "Choose a catalog from an installed add-on".into(),
            addon_catalog_addon: None,
            addon_stream_results: Vec::new(),
            stream_status: "Choose a title or catalog item to discover sources".into(),
            torrent_session: None,
            torrent_files: Vec::new(),
            torrent_progress: None,
            torrent_status: "No torrent session active".into(),
            torrent_task: None,
            native_parent_hwnd,
            native_viewport_bounds: Arc::new(Mutex::new(None)),
            native_video_surface: Arc::new(Mutex::new(None)),
            native_surface_error: Arc::new(Mutex::new(None)),
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

        let mut config = build_mpv_config(&self.settings, &MpvCapabilities::default());
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
                                app.torrent_status = format!(
                                    "{:.1} / {:.1} MiB downloaded · {:.1} MiB uploaded · {}",
                                    progress.downloaded_bytes as f64 / 1_048_576.0,
                                    progress.total_bytes as f64 / 1_048_576.0,
                                    progress.uploaded_bytes as f64 / 1_048_576.0,
                                    progress.state
                                );
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
                                    app.playback_state = state.clone();
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
                                    app.playback_status = format!("MPV failed: {message}");
                                    diagnostics::record(
                                        diagnostics::Level::Error,
                                        "playback",
                                        &app.playback_status,
                                    );
                                    app.playback_worker = None;
                                }
                                PlaybackEvent::Stopped => {
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

    fn load_tmdb_catalog(
        &mut self,
        media_type: &'static str,
        query: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.tmdb_status = match query.as_deref() {
            Some(value) => format!("Searching TMDB for {value}…"),
            None => format!("Loading popular {media_type}…"),
        };
        self.tmdb_results.clear();
        self.tmdb_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = TmdbClient::from_credential_store("en-US")?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize TMDB runtime")?;
                    runtime.block_on(async {
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
        let tmdb_id = title.id;
        let addons = self.database.installed_addons().unwrap_or_default();
        self.selected_title = Some(display_title.clone());
        self.active_page = AppPage::Details;
        self.tmdb_details = None;
        self.addon_stream_results.clear();
        self.stream_status = "Resolving provider IDs and checking enabled add-ons…".into();
        self.tmdb_status = format!("Loading details for {display_title}…");
        self.active_media_id = Some(title.media_id(&media_type));
        self.tmdb_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                let client = TmdbClient::from_credential_store("en-US")?;
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
                    .context("could not initialize metadata and source runtime")?;
                runtime.block_on(async {
                    let (details, external_ids) = if media_type == "movie" {
                        (client.movie_details(tmdb_id).await?, client.movie_external_ids(tmdb_id).await?)
                    } else {
                        (client.tv_details(tmdb_id).await?, client.tv_external_ids(tmdb_id).await?)
                    };
                    let provider_id = external_ids.imdb_id.clone().unwrap_or_else(|| format!("tmdb:{tmdb_id}"));
                    let stremio_id = if media_type == "tv" { format!("{provider_id}:1:1") } else { provider_id };
                    let stremio_type = if media_type == "tv" { "series" } else { "movie" };
                    let streams = AddonClient::new().discover_streams(&addons, stremio_type, &stremio_id).await;
                    Ok::<_, anyhow::Error>((details, streams, stremio_id))
                })
            }).await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((details, streams, stream_id)) => {
                        app.tmdb_details = Some(details);
                        app.addon_stream_results = streams;
                        app.tmdb_status = "TMDB details loaded".into();
                        app.stream_status = format!("Source discovery completed for {stream_id} (TV currently checks S01E01)");
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

    fn load_addon_catalog(
        &mut self,
        addon: StoredAddon,
        media_type: String,
        catalog_id: String,
        cx: &mut Context<Self>,
    ) {
        self.addon_catalog_status = format!("Loading {} catalog…", addon.name);
        self.addon_catalog_items.clear();
        self.addon_catalog_addon = Some(addon.clone());
        self.addon_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = AddonClient::new();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize add-on catalog runtime")?;
                    runtime.block_on(client.catalog(&addon, &media_type, &catalog_id, &[]))
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
                    let stream_id = if media_type == "series" { format!("{media_id}:1:1") } else { media_id.clone() };
                    let streams = client.streams(&addon, &media_type, &stream_id).await?;
                    Ok::<_, anyhow::Error>((metadata, streams.streams, media_id))
                })
            }).await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((metadata, streams, id)) => {
                        app.tmdb_details = Some(metadata);
                        app.addon_stream_results = vec![AddonStreamResult { addon_id: addon_id.clone(), addon_name: addon_name.clone(), streams, error: None }];
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

    fn inspect_torrent_stream(&mut self, stream: StremioStream, cx: &mut Context<Self>) {
        let Some(info_hash) = stream.info_hash.clone() else {
            self.stream_status = "Torrent source is missing its info hash".into();
            cx.notify();
            return;
        };
        let requested_file = stream.file_index.map(|index| index as usize);
        let trackers = stream.sources.clone();
        let previous = self.torrent_session.take();
        if previous.is_some() {
            if let Some(worker) = self.playback_worker.as_ref() {
                if let Ok(worker) = worker.lock() {
                    let _ = worker.stop();
                }
            }
        }
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
                        let preferred = requested_file.filter(|index| files.iter().any(|file| file.index == *index));
                        let automatic = preferred.or_else(|| (files.len() == 1).then_some(files[0].index));
                        app.torrent_session = Some(session);
                        app.torrent_files = files;
                        app.torrent_status = format!("{} video file(s) available", app.torrent_files.len());
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
        let Some(session) = self.torrent_session.take() else {
            self.torrent_status = "No inspected torrent session is available".into();
            cx.notify();
            return;
        };
        if self.playback_worker.is_none() {
            self.start_playback(cx);
        }
        let worker = self.playback_worker.clone();
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
                                    .load_local_url(&url)
                            });
                        match queued {
                            Ok(()) => {
                                app.torrent_status =
                                    "Torrent stream is ready; playback downloads on demand".into();
                                app.stream_status = "Torrent source queued in MPV".into();
                                app.playback_status =
                                    "Playing torrent stream from the local loopback server".into();
                                app.active_media_id = None;
                            }
                            Err(error) => {
                                app.torrent_status =
                                    format!("Could not hand torrent stream to MPV: {error:#}");
                                app.stream_status = app.torrent_status.clone();
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
        self.torrent_status = "Stopping torrent engine and removing temporary session data…".into();
        self.torrent_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("could not initialize torrent shutdown runtime")?;
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
        self.stop_torrent_session(cx);
        if self.playback_worker.is_none() {
            self.start_playback(cx);
        }
        let Some(worker) = self.playback_worker.clone() else {
            self.stream_status = "MPV is not ready; check the playback runtime status".into();
            cx.notify();
            return;
        };
        let url_string = url.to_string();
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
                        let parsed = url::Url::parse(&url_string)?;
                        client.validate_direct_stream_url(&parsed).await?;
                        let worker = worker
                            .lock()
                            .map_err(|_| anyhow::anyhow!("playback worker lock is unavailable"))?;
                        worker.load_url(parsed.as_str())?;
                        Ok::<_, anyhow::Error>(())
                    })
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(()) => {
                        app.stream_status = "Direct stream queued in MPV".into();
                        app.playback_status = "Playing selected add-on stream".into();
                        app.active_media_id = None;
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
            self.addon_status = "Clipboard does not contain a manifest URL".into();
            cx.notify();
            return;
        };

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
                        app.addon_status = format!("Manifest preview failed: {error:#}");
                        diagnostics::record(
                            diagnostics::Level::Warn,
                            "addons",
                            "user-supplied add-on manifest preview failed",
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
            id: preview.manifest.id,
            name: preview.manifest.name,
            version: preview.manifest.version,
            manifest_url: preview.manifest_url,
            enabled: true,
            manifest_json: preview.manifest_json,
            installed_at,
        };
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

    fn configure_tmdb_key(&mut self, cx: &mut Context<Self>) {
        match TmdbCredentialStore::prompt_and_save_access_token() {
            Ok(true) => {
                self.tmdb_credentials_status =
                    "TMDB Read Access Token saved in Windows Credential Manager".into();
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "metadata",
                    "TMDB Read Access Token configured securely",
                );
            }
            Ok(false) => {
                self.tmdb_credentials_status = "TMDB token setup cancelled; no changes made".into()
            }
            Err(error) => {
                self.tmdb_credentials_status = format!("Could not save TMDB token: {error:#}");
                diagnostics::record(
                    diagnostics::Level::Warn,
                    "metadata",
                    "TMDB token configuration failed",
                );
            }
        }
        cx.notify();
    }

    fn clear_tmdb_key(&mut self, cx: &mut Context<Self>) {
        match TmdbCredentialStore::clear_access_token() {
            Ok(()) => {
                self.tmdb_credentials_status =
                    "TMDB token removed from Windows Credential Manager".into();
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "metadata",
                    "TMDB token removed from secure storage",
                );
            }
            Err(error) => {
                self.tmdb_credentials_status = format!("Could not remove TMDB token: {error:#}");
                diagnostics::record(
                    diagnostics::Level::Warn,
                    "metadata",
                    "TMDB token removal failed",
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
        match self
            .database
            .save_playback_progress(media_id, position, duration, updated_at)
        {
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
        .on_mouse_down(
            gpui::MouseButton::Left,
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
        .on_mouse_down(
            gpui::MouseButton::Left,
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.settings.clone();
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
            .id("senzo-root")
            .size_full()
            .flex()
            .bg(rgb(0x0A0C0E))
            .text_color(rgb(0xF4F4F5))
            .child(
                div()
                    .w(px(218.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .p_5()
                    .bg(rgb(0x101315))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(9.))
                                    .rounded_full()
                                    .bg(rgb(0x22D3EE)),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0xF8FAFC))
                                    .child("SENZO"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(dynamic_choice_chip("Home".into(), self.active_page == AppPage::Home, |this, _cx| this.active_page = AppPage::Home, cx))
                            .child(dynamic_choice_chip("Movies".into(), self.active_page == AppPage::Movies, |this, _cx| this.active_page = AppPage::Movies, cx))
                            .child(dynamic_choice_chip("TV Shows".into(), self.active_page == AppPage::TvShows, |this, _cx| this.active_page = AppPage::TvShows, cx))
                            .child(dynamic_choice_chip("Search".into(), self.active_page == AppPage::Search, |this, _cx| this.active_page = AppPage::Search, cx))
                            .child(dynamic_choice_chip("Library".into(), self.active_page == AppPage::Library, |this, _cx| this.active_page = AppPage::Library, cx))
                            .child(dynamic_choice_chip("Add-ons".into(), self.active_page == AppPage::Addons, |this, _cx| this.active_page = AppPage::Addons, cx))
                            .child(dynamic_choice_chip("Settings".into(), self.active_page == AppPage::Settings, |this, _cx| this.active_page = AppPage::Settings, cx)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x858C94))
                                    .child("LOCAL-FIRST"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xB7BEC6))
                                    .child("Your settings stay on this device."),
                            ),
                    ),
            )
            .child(
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
                                    .child("Playback settings"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x9299A1))
                                    .child("Tune decoding, image processing and playback resources."),
                            ),
                    )
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
                                                                        diagnostics::Level::Warn,
                                                                        "playback",
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
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(online_subtitle_chips),
                            ),
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
                                "Choose whether the playback engine may use your GPU decoder.",
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
                                        settings.hardware_decoding == HardwareDecoding::Disabled,
                                        |this, _cx| this.settings.hardware_decoding = HardwareDecoding::Disabled,
                                        cx,
                                    )),
                            ))
                            .child(option_row(
                                "Decoder preference",
                                "D3D11VA is the preferred Windows path. DXVA2 remains for compatibility.",
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
                                    .child(choice_chip(
                                        "D3D11VA",
                                        settings.decoder == DecoderPreference::D3d11va,
                                        |this, _cx| this.settings.decoder = DecoderPreference::D3d11va,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "DXVA2",
                                        settings.decoder == DecoderPreference::Dxva2,
                                        |this, _cx| this.settings.decoder = DecoderPreference::Dxva2,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Software",
                                        settings.decoder == DecoderPreference::Software,
                                        |this, _cx| this.settings.decoder = DecoderPreference::Software,
                                        cx,
                                    )),
                            ))
                            .child(option_row(
                                "Video renderer",
                                "Renderer preference is applied when the MPV playback session starts.",
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
                                    .child(choice_chip(
                                        "GPU Next",
                                        settings.renderer == RendererPreference::GpuNext,
                                        |this, _cx| this.settings.renderer = RendererPreference::GpuNext,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "GPU",
                                        settings.renderer == RendererPreference::Gpu,
                                        |this, _cx| this.settings.renderer = RendererPreference::Gpu,
                                        cx,
                                    )),
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
                                "The playback engine will validate the display and renderer before applying HDR output.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Auto", settings.hdr == HdrMode::Auto, |this, _cx| this.settings.hdr = HdrMode::Auto, cx))
                                    .child(choice_chip("Tone map", settings.hdr == HdrMode::ToneMap, |this, _cx| this.settings.hdr = HdrMode::ToneMap, cx))
                                    .child(choice_chip("Passthrough", settings.hdr == HdrMode::Passthrough, |this, _cx| this.settings.hdr = HdrMode::Passthrough, cx))
                                    .child(choice_chip("Off", settings.hdr == HdrMode::Disabled, |this, _cx| this.settings.hdr = HdrMode::Disabled, cx)),
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
                        "TMDB metadata",
                        "Configure the official TMDB Read Access Token. SENZO stores it only in Windows Credential Manager; it is never written to settings, SQLite, or logs.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().text_sm().text_color(rgb(0xB7BEC6)).child(self.tmdb_credentials_status.clone()))
                            .child(div().flex().flex_wrap().gap_2()
                                .child(choice_chip("Set / replace TMDB token", false, |this, cx| this.configure_tmdb_key(cx), cx))
                                .child(choice_chip("Remove TMDB token", false, |this, cx| this.clear_tmdb_key(cx), cx))),
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
                                div()
                                    .id("copy-diagnostics")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(rgb(0x164E63))
                                    .text_color(rgb(0xCFFAFE))
                                    .text_sm()
                                    .cursor_pointer()
                                    .child("Copy logs")
                                    .on_click(cx.listener(|this, _event, _window, cx| {
                                        this.copy_diagnostics(cx);
                                    })),
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
                    ),
            )
            .when(self.active_page != AppPage::Settings, |root| {
                root.child(render_discovery_page(
                    self.active_page,
                    self.selected_title.clone(),
                    self.search_query.clone(),
                    self.database.installed_addons().unwrap_or_default(),
                    self.addon_manifest_preview.clone(),
                    self.addon_status.clone(),
                    self.tmdb_results.clone(),
                    self.tmdb_status.clone(),
                    self.tmdb_details.clone(),
                    self.addon_catalog_items.clone(),
                    self.addon_catalog_status.clone(),
                    self.addon_catalog_addon.clone(),
                    self.addon_stream_results.clone(),
                    self.stream_status.clone(),
                    self.torrent_files.clone(),
                    self.torrent_status.clone(),
                    cx,
                ))
            })
    }
}

impl Drop for SenzoApp {
    fn drop(&mut self) {
        if let Some(worker) = self.playback_worker.take() {
            if let Ok(worker) = worker.lock() {
                let _ = worker.stop();
            }
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
    installed_addons: Vec<StoredAddon>,
    addon_manifest_preview: Option<AddonManifestPreview>,
    addon_status: String,
    tmdb_results: Vec<(String, TmdbSearchResult)>,
    tmdb_status: String,
    tmdb_details: Option<Value>,
    addon_catalog_items: Vec<Value>,
    addon_catalog_status: String,
    addon_catalog_addon: Option<StoredAddon>,
    addon_stream_results: Vec<AddonStreamResult>,
    stream_status: String,
    torrent_files: Vec<TorrentVideoFile>,
    torrent_status: String,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement {
    let (heading, description) = match page {
        AppPage::Home => (
            "Welcome to SENZO",
            "Your personal space for movies and television",
        ),
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
    };

    let is_details = page == AppPage::Details;
    let is_search = page == AppPage::Search;
    let is_library = page == AppPage::Library;
    let is_home = page == AppPage::Home;
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
    let mut catalog_chips = Vec::new();
    for addon in &installed_addons {
        if let Ok(manifest) = serde_json::from_str::<AddonManifest>(&addon.manifest_json) {
            if manifest.supports_resource("catalog") {
                for catalog in &manifest.catalogs {
                    let Some(id) = catalog.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let kind = catalog
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("movie")
                        .to_owned();
                    let name = catalog.get("name").and_then(Value::as_str).unwrap_or(id);
                    let label = format!("{} · {}", addon.name, name);
                    let addon_copy = addon.clone();
                    let catalog_id = id.to_owned();
                    catalog_chips.push(dynamic_choice_chip(
                        label,
                        false,
                        move |this, cx| {
                            this.load_addon_catalog(
                                addon_copy.clone(),
                                kind.clone(),
                                catalog_id.clone(),
                                cx,
                            )
                        },
                        cx,
                    ));
                }
            }
        }
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
            source_notes.push(format!("{} · provider error: {error}", provider.addon_name));
        }
        for stream in &provider.streams {
            if stream.direct_url().ok().flatten().is_some() {
                let label = format!(
                    "{} · {}",
                    provider.addon_name,
                    stream
                        .name
                        .as_deref()
                        .or(stream.title.as_deref())
                        .unwrap_or("Direct stream")
                );
                let selected = stream.clone();
                stream_chips.push(dynamic_choice_chip(
                    label,
                    false,
                    move |this, cx| this.play_addon_stream(selected.clone(), cx),
                    cx,
                ));
            } else if stream.is_torrent() {
                let label = format!(
                    "{} · {} · torrent",
                    provider.addon_name,
                    stream
                        .title
                        .as_deref()
                        .or(stream.name.as_deref())
                        .unwrap_or("Torrent source")
                );
                let selected = stream.clone();
                stream_chips.push(dynamic_choice_chip(
                    label,
                    false,
                    move |this, cx| this.play_addon_stream(selected.clone(), cx),
                    cx,
                ));
            } else if stream.external_url.is_some() {
                source_notes.push(format!(
                    "{} · external link supplied by provider",
                    provider.addon_name
                ));
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

    div()
        .id("discovery-page")
        .flex_1()
        .h_full()
        .overflow_y_scroll()
        .p_8()
        .flex()
        .flex_col()
        .gap_6()
        .bg(rgb(0x0A0C0E))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_2xl()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(heading),
                )
                .child(div().text_sm().text_color(rgb(0x9299A1)).child(description)),
        )
        .when(
            page != AppPage::Addons && page != AppPage::Library && page != AppPage::Details,
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
        .when(is_details, |panel| {
            let details = tmdb_details.clone().unwrap_or(Value::Null);
            let overview = details
                .get("overview")
                .and_then(Value::as_str)
                .unwrap_or("No synopsis available.");
            let date = details
                .get("release_date")
                .or_else(|| details.get("first_air_date"))
                .and_then(Value::as_str)
                .unwrap_or("Release date unavailable");
            let rating = details
                .get("vote_average")
                .and_then(Value::as_f64)
                .map(|value| format!("{value:.1}/10"))
                .unwrap_or_else(|| "No rating".into());
            panel
                .child(
                    div()
                        .p_4()
                        .rounded_lg()
                        .bg(rgb(0x151719))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x67E8F9))
                                .child(format!("{date} · {rating}")),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0xD4D4D8))
                                .child(overview.to_owned()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x9299A1))
                                .child(tmdb_status.clone()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_base()
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
                        .children(source_notes.into_iter().map(|label| {
                            div().text_xs().text_color(rgb(0xA1A1AA)).child(label)
                        })),
                )
        })
        .when(is_search, |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .bg(rgb(0x151719))
                    .border_1()
                    .border_color(rgb(0x263238))
                    .child(
                        div()
                            .text_base()
                            .text_color(rgb(0x67E8F9))
                            .child("⌕"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(rgb(0x9299A1))
                            .child(if search_query.is_empty() {
                                "Search movies and TV shows — metadata search connects in the next integration batch".to_string()
                            } else {
                                search_query.clone()
                            }),
                    )
                    .child(dynamic_choice_chip(
                        "Movies".into(),
                        false,
                        |this, _cx| this.active_page = AppPage::Movies,
                        cx,
                    ))
                    .child(dynamic_choice_chip(
                        "TV Shows".into(),
                        false,
                        |this, _cx| this.active_page = AppPage::TvShows,
                        cx,
                    )),
            )
        })
        .when(is_home, |this| {
            this.child(
                div()
                    .h(px(260.))
                    .w_full()
                    .rounded_xl()
                    .bg(rgb(0x151D20))
                    .border_1()
                    .border_color(rgb(0x263238))
                    .p_6()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x67E8F9))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("FEATURED"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Your next favourite starts here"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0xA1A1AA))
                                    .child("Cinematic artwork and recommendations will appear when TMDB discovery is connected."),
                            ),
                    ),
            )
        })
        .when(is_details, |this| {
            this.child(
                div()
                    .flex()
                    .gap_6()
                    .p_5()
                    .rounded_xl()
                    .bg(rgb(0x151719))
                    .child(
                        div()
                            .w(px(190.))
                            .h(px(285.))
                            .rounded_lg()
                            .bg(rgb(0x20272B))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(div().text_xs().text_color(rgb(0x858C94)).child("POSTER")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(selected_title.unwrap_or_else(|| "Choose a title to see its details".into())),
                            )
                            .child(div().text_sm().text_color(rgb(0xA1A1AA)).child("Release information, synopsis, ratings, cast, trailers and artwork will be populated from TMDB in the metadata integration batch."))
                            .child(dynamic_choice_chip(
                                "Back to Home".into(),
                                false,
                                |this, _cx| this.active_page = AppPage::Home,
                                cx,
                            )),
                    ),
            )
        })
        .when(!is_details && !is_library && page != AppPage::Addons, |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).child(if is_home { "Continue exploring" } else { "Browse titles" }))
                            .child(div().text_xs().text_color(rgb(0x858C94)).child("Artwork placeholders · live catalogue in Batch 3")),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .children((0..6).map(|index| {
                                let title = format!("Untitled preview {}", index + 1);
                                let selected_title = title.clone();
                                div()
                                    .w(px(148.))
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(148.))
                                            .h(px(218.))
                                            .rounded_lg()
                                            .bg(rgb(0x1B2226))
                                            .border_1()
                                            .border_color(rgb(0x263238))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(div().text_xs().text_color(rgb(0x65707A)).child("POSTER")),
                                    )
                                    .child(div().text_xs().text_color(rgb(0xA1A1AA)).child("Artwork placeholder"))
                                    .child(dynamic_choice_chip(
                                        "View details".into(),
                                        false,
                                        move |this, _cx| {
                                            this.selected_title = Some(selected_title.clone());
                                            this.active_page = AppPage::Details;
                                        },
                                        cx,
                                    ))
                            })),
                    ),
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
            this.child(div().p_4().rounded_lg().bg(rgb(0x151719)).flex().flex_col().gap_3()
                .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child("Installed add-on catalogs"))
                .child(div().text_xs().text_color(rgb(0x9299A1)).child("Catalogs are loaded from each enabled add-on manifest."))
                .child(div().flex().flex_wrap().gap_2().children(catalog_chips))
                .child(div().text_xs().text_color(rgb(0x9299A1)).child(addon_catalog_status.clone()))
                .child(div().flex().flex_wrap().gap_2().children(catalog_item_chips)))
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
                            .child(dynamic_choice_chip(
                                "Paste manifest URL".into(),
                                false,
                                |this, cx| this.preview_addon_from_clipboard(cx),
                                cx,
                            ))
                            .when(addon_manifest_preview.is_some(), |panel| {
                                let preview = addon_manifest_preview.clone().expect("preview exists");
                                panel.child(
                                    div()
                                        .p_4()
                                        .rounded_md()
                                        .bg(rgb(0x101315))
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(div().text_base().font_weight(gpui::FontWeight::SEMIBOLD).child(preview.manifest.name.clone()))
                                        .child(div().text_xs().text_color(rgb(0xA1A1AA)).child(format!("{} · version {}", preview.manifest.id, preview.manifest.version)))
                                        .child(div().text_sm().text_color(rgb(0x9299A1)).child(preview.manifest.description.clone().unwrap_or_else(|| "No description supplied".into())))
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
        .when(page == AppPage::Search, |this| {
            this.child(
                div()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(0x151719))
                    .child(div().text_sm().text_color(rgb(0x9299A1)).child("Enter a title to search the movie and TV catalogues. Live results and keyboard input are part of the upcoming TMDB integration.")),
            )
        })
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
    match TorrentSessionDirectory::recover_default_root() {
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
