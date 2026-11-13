use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context as _;
use gpui::{
    App, ClipboardItem, Context, QuitMode, Render, Task, Window, WindowOptions, div, prelude::*,
    px, rgb,
};

use senzo::{
    diagnostics,
    playback::{
        windows_surface::gpui_parent_hwnd,
        mpv_config::{MpvCapabilities, build_mpv_config},
        mpv_worker::{PlaybackEvent, PlaybackState, PlaybackWorker},
    },
    settings::{
        DecoderPreference, HardwareDecoding, HdrMode, PlaybackSettings, RendererPreference,
        ScalingAlgorithm,
    },
    storage::LocalDatabase,
    subtitles::{
        OpenSubtitlesClient, OpenSubtitlesCredentialStore, OpenSubtitlesResult, SubtitleSearch,
    },
};

struct SenzoApp {
    database: LocalDatabase,
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
    subtitle_task: Option<Task<()>>,
    native_parent_hwnd: Option<usize>,
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
            subtitle_task: None,
            native_parent_hwnd,
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

        let config = build_mpv_config(&self.settings, &MpvCapabilities::default());
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
                worker.load_file(path)?;
                if let Some(position) = resume_position {
                    worker.seek_absolute(position)?;
                }
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
            Ok(()) => self.status = "Saved locally".into(),
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
                            .child(nav_item("Home", false))
                            .child(nav_item("Discover", false))
                            .child(nav_item("Library", false))
                            .child(nav_item("Add-ons", false))
                            .child(nav_item("Settings", true)),
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
                        "Playback engine",
                        "The MPV worker runs separately from the UI. A trusted runtime DLL must be placed beside SENZO in the runtime folder.",
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
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x858C94))
                                    .child(format!(
                                        "Position: {} s  ·  Duration: {} s  ·  Volume: {}%",
                                        self.playback_state.position_seconds
                                            .map(|value| format!("{value:.1}"))
                                            .unwrap_or_else(|| "—".into()),
                                        self.playback_state.duration_seconds
                                            .map(|value| format!("{value:.1}"))
                                            .unwrap_or_else(|| "—".into()),
                                        self.playback_state.volume
                                            .map(|value| format!("{value:.0}"))
                                            .unwrap_or_else(|| "—".into()),
                                    )),
                            )
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
                                "Renderer selection will be applied when the playback backend is connected.",
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
                                    .child(choice_chip("Small · 32", settings.subtitles.font_size == 32, |this, _cx| this.settings.subtitles.font_size = 32, cx))
                                    .child(choice_chip("Default · 42", settings.subtitles.font_size == 42, |this, _cx| this.settings.subtitles.font_size = 42, cx))
                                    .child(choice_chip("Large · 52", settings.subtitles.font_size == 52, |this, _cx| this.settings.subtitles.font_size = 52, cx))
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
                                "Distance from the bottom edge, expressed as a percentage.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("12%", settings.subtitles.vertical_margin == 12, |this, _cx| this.settings.subtitles.vertical_margin = 12, cx))
                                    .child(choice_chip("24%", settings.subtitles.vertical_margin == 24, |this, _cx| this.settings.subtitles.vertical_margin = 24, cx))
                                    .child(choice_chip("36%", settings.subtitles.vertical_margin == 36, |this, _cx| this.settings.subtitles.vertical_margin = 36, cx)),
                            ))
                            .child(toggle_row(
                                "Bold subtitles",
                                "Use a heavier subtitle weight for improved legibility.",
                                settings.subtitles.bold,
                                |this, _cx| this.settings.subtitles.bold = !this.settings.subtitles.bold,
                                cx,
                            )),
                    ))
                    .child(section(
                        "Buffering",
                        "Limits are normalized before being stored. These preferences are not yet connected to a streaming engine.",
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
                                    .child("Playback engine integration is planned"),
                            ),
                    ),
            )
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
                            diagnostics::record(diagnostics::Level::Info, "playback", "acquired GPUI native Win32 parent HWND");
                            Some(hwnd)
                        }
                        Err(error) => {
                            diagnostics::record(diagnostics::Level::Warn, "playback", &format!("native video surface unavailable: {error:#}"));
                            None
                        }
                    };
                    cx.new(move |cx| {
                        let mut app = SenzoApp::new(database, native_parent_hwnd);
                        app.start_playback(cx);
                        app
                    })
                })
                .expect("failed to open SENZO window");

                cx.activate(true);
            }
        });
}
