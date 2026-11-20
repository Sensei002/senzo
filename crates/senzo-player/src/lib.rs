//! Single-owner native libmpv engine and typed playback contract.
use libloading::Library;
use std::{
    env,
    ffi::{CStr, CString, c_char, c_double, c_int, c_void},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, RwLock,
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use thiserror::Error;
use url::Url;

const F_FLAG: c_int = 3;
const F_INT64: c_int = 4;
const F_DOUBLE: c_int = 5;
const EV_SHUTDOWN: c_int = 1;
const EV_LOG: c_int = 2;
const EV_START: c_int = 6;
const EV_END: c_int = 7;
const EV_LOADED: c_int = 8;
const EV_PAUSE: c_int = 12;
const EV_UNPAUSE: c_int = 13;
const EV_RESTART: c_int = 21;
const EV_PROPERTY: c_int = 22;
const EV_IDLE: c_int = 11;
const POLL: f64 = 0.04;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 8192;
const MAX_TRACKS: usize = 64;
const MAX_DIAGNOSTICS: usize = 128;
const HTTP_HEADER_OPTION_PROPERTY: &str = "options/http-header-fields";

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MpvError {
    #[error("libmpv could not be loaded: {0}")]
    LibraryUnavailable(String),
    #[error("libmpv initialization failed: {0}")]
    Initialization(String),
    #[error("libmpv command failed: {0}")]
    Command(String),
    #[error("invalid playback source URL")]
    InvalidUrl,
    #[error("invalid request headers or header limits exceeded")]
    InvalidHeaders,
    #[error("playback control value is outside its supported range")]
    InvalidValue,
    #[error("MPV owner thread stopped")]
    WorkerStopped,
    #[error("MPV operation timed out")]
    Timeout,
    #[error("could not start MPV owner thread: {0}")]
    WorkerStart(String),
}

#[derive(Debug, Clone)]
pub struct MpvConfig {
    pub library_path: Option<PathBuf>,
    /// Native parent handle (HWND or X11 XID). None lets libmpv open its own surface.
    pub window_id: Option<u64>,
    pub start_paused: bool,
    /// Optional libmpv video output override (for example, "gpu" in headless CI).
    pub video_output: Option<String>,
    /// Optional libmpv audio output override (for example, "pulse" in headless CI).
    pub audio_output: Option<String>,
    /// Optional native output-window visibility override.
    pub force_window: Option<bool>,
    /// Optional renderer API override (for headless CI, use "opengl").
    pub gpu_api: Option<String>,
    /// Optional platform GPU context override (for Xvfb, use "x11egl").
    pub gpu_context: Option<String>,
}
impl Default for MpvConfig {
    fn default() -> Self {
        Self {
            library_path: env::var_os("SENZO_LIBMPV_PATH").map(PathBuf::from),
            window_id: None,
            start_paused: false,
            video_output: None,
            audio_output: None,
            force_window: None,
            gpu_api: None,
            gpu_context: None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Idle,
    Loading,
    Playing,
    Paused,
    Stopped,
    Ended,
    Error,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Subtitle,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTrack {
    pub id: i64,
    pub kind: TrackKind,
    pub language: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub selected: bool,
    pub external: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackChapter {
    pub index: usize,
    pub start_seconds: f64,
    pub title: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeMode {
    Fit,
    Fill,
    Stretch,
    Zoom,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackSnapshot {
    pub status: PlaybackStatus,
    pub media_title: Option<String>,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub volume_percent: f64,
    pub muted: bool,
    pub paused: bool,
    pub speed: f64,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_output_driver: Option<String>,
    pub audio_device_setting: Option<String>,
    pub audio_input_channels: Option<String>,
    pub audio_output_channels: Option<String>,
    pub audio_output_sample_rate: Option<i64>,
    pub audio_output_sample_format: Option<String>,
    pub audio_track_id: Option<i64>,
    pub video_width: Option<i64>,
    pub video_height: Option<i64>,
    pub buffering: bool,
    pub buffered_percent: f64,
    pub tracks: Vec<MediaTrack>,
    pub chapters: Vec<PlaybackChapter>,
    pub diagnostics: Vec<PlayerDiagnostic>,
}
impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Idle,
            media_title: None,
            position_seconds: None,
            duration_seconds: None,
            volume_percent: 100.0,
            muted: false,
            paused: false,
            speed: 1.0,
            video_codec: None,
            audio_codec: None,
            audio_output_driver: None,
            audio_device_setting: None,
            audio_input_channels: None,
            audio_output_channels: None,
            audio_output_sample_rate: None,
            audio_output_sample_format: None,
            audio_track_id: None,
            video_width: None,
            video_height: None,
            buffering: false,
            buffered_percent: 100.0,
            tracks: Vec::new(),
            chapters: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    Snapshot(Box<PlaybackSnapshot>),
    Diagnostic(PlayerDiagnostic),
}

enum Command {
    Load {
        url: String,
        headers: Vec<(String, String)>,
        reply: SyncSender<Result<(), MpvError>>,
    },
    Set {
        property: &'static str,
        value: String,
        reply: SyncSender<Result<(), MpvError>>,
    },
    CyclePause(SyncSender<Result<(), MpvError>>),
    Seek {
        seconds: f64,
        relative: bool,
        reply: SyncSender<Result<(), MpvError>>,
    },
    Select {
        kind: TrackKind,
        id: Option<i64>,
        reply: SyncSender<Result<(), MpvError>>,
    },
    AddSubtitle {
        url: String,
        headers: Vec<(String, String)>,
        reply: SyncSender<Result<(), MpvError>>,
    },
    ScreenshotToFile {
        path: String,
        window: bool,
        reply: SyncSender<Result<(), MpvError>>,
    },
    Stop(Option<SyncSender<Result<(), MpvError>>>),
    Shutdown(Option<SyncSender<Result<(), MpvError>>>),
}

/// All libmpv entry points are called by one dedicated thread, never by GPUI.
#[derive(Clone)]
pub struct MpvPlayer {
    inner: Arc<MpvInner>,
}
struct MpvInner {
    tx: Sender<Command>,
    snapshot: Arc<RwLock<PlaybackSnapshot>>,
    listeners: Arc<Mutex<Vec<Sender<PlayerEvent>>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for MpvInner {
    fn drop(&mut self) {
        let worker = match self.worker.get_mut() {
            Ok(slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(worker) = worker {
            let _ = self.tx.send(Command::Shutdown(None));
            // Do not join the native owner on the GPUI thread. Its loop wakes
            // at least every 40 ms and exits after mpv_terminate_destroy.
            let _ = thread::Builder::new()
                .name("senzo-mpv-reaper".into())
                .spawn(move || {
                    let _ = worker.join();
                });
        }
    }
}
impl MpvPlayer {
    pub fn spawn(config: MpvConfig) -> Result<Self, MpvError> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let snapshot = Arc::new(RwLock::new(PlaybackSnapshot::default()));
        let listeners = Arc::new(Mutex::new(Vec::new()));
        let s = snapshot.clone();
        let l = listeners.clone();
        let worker = thread::Builder::new()
            .name("senzo-mpv-owner".into())
            .spawn(move || worker_main(config, rx, ready_tx, s, l))
            .map_err(|e| MpvError::WorkerStart(e.to_string()))?;
        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(Self {
                inner: Arc::new(MpvInner {
                    tx,
                    snapshot,
                    listeners,
                    worker: Mutex::new(Some(worker)),
                }),
            }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                drop(tx);
                let _ = worker.join();
                Err(MpvError::Timeout)
            }
        }
    }
    pub fn snapshot(&self) -> PlaybackSnapshot {
        self.inner
            .snapshot
            .read()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
    pub fn subscribe(&self) -> Receiver<PlayerEvent> {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut v) = self.inner.listeners.lock() {
            v.push(tx);
        }
        rx
    }
    pub fn load(
        &self,
        url: &str,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), MpvError> {
        let url = validate_url(url)?;
        let headers = validate_headers(headers)?;
        self.request(|reply| Command::Load {
            url,
            headers,
            reply,
        })
    }
    pub fn play(&self) -> Result<(), MpvError> {
        self.set("pause", "no")
    }
    pub fn pause(&self) -> Result<(), MpvError> {
        self.set("pause", "yes")
    }
    pub fn toggle_pause(&self) -> Result<(), MpvError> {
        self.request(Command::CyclePause)
    }
    pub fn seek_absolute(&self, seconds: f64) -> Result<(), MpvError> {
        self.seek(seconds, false)
    }
    pub fn seek_relative(&self, seconds: f64) -> Result<(), MpvError> {
        self.seek(seconds, true)
    }
    fn seek(&self, seconds: f64, relative: bool) -> Result<(), MpvError> {
        validate_seek(seconds)?;
        self.request(|reply| Command::Seek {
            seconds,
            relative,
            reply,
        })
    }
    pub fn set_volume(&self, value: f64) -> Result<(), MpvError> {
        validate_volume(value)?;
        self.set("volume", &format!("{value:.2}"))
    }
    pub fn set_muted(&self, value: bool) -> Result<(), MpvError> {
        self.set("mute", if value { "yes" } else { "no" })
    }
    pub fn set_speed(&self, value: f64) -> Result<(), MpvError> {
        validate_speed(value)?;
        self.set("speed", &format!("{value:.3}"))
    }
    pub fn select_audio_track(&self, id: Option<i64>) -> Result<(), MpvError> {
        self.select(TrackKind::Audio, id)
    }
    pub fn select_subtitle_track(&self, id: Option<i64>) -> Result<(), MpvError> {
        self.select(TrackKind::Subtitle, id)
    }
    pub fn add_external_subtitle(
        &self,
        url: &str,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), MpvError> {
        let url = validate_url(url)?;
        let headers = validate_headers(headers)?;
        self.request(|reply| Command::AddSubtitle {
            url,
            headers,
            reply,
        })
    }
    pub fn set_subtitle_font_size(&self, size: u32) -> Result<(), MpvError> {
        if !(12..=72).contains(&size) {
            return Err(MpvError::InvalidValue);
        }
        self.set("options/sub-font-size", &size.to_string())
    }
    pub fn set_subtitle_delay(&self, seconds: f64) -> Result<(), MpvError> {
        if !seconds.is_finite() || !(-60.0..=60.0).contains(&seconds) {
            return Err(MpvError::InvalidValue);
        }
        self.set("sub-delay", &format!("{seconds:.3}"))
    }
    pub fn set_subtitles_visible(&self, visible: bool) -> Result<(), MpvError> {
        self.set("sub-visibility", if visible { "yes" } else { "no" })
    }
    pub fn set_resize_mode(&self, mode: ResizeMode) -> Result<(), MpvError> {
        let (keep_aspect, panscan) = match mode {
            ResizeMode::Fit => ("yes", "0"),
            ResizeMode::Fill => ("yes", "1"),
            ResizeMode::Stretch => ("no", "0"),
            ResizeMode::Zoom => ("yes", "1"),
        };
        self.set("options/keepaspect", keep_aspect)?;
        self.set("options/panscan", panscan)
    }
    pub fn set_fullscreen(&self, fullscreen: bool) -> Result<(), MpvError> {
        self.set("fullscreen", if fullscreen { "yes" } else { "no" })
    }
    pub fn set_picture_in_picture(&self, enabled: bool) -> Result<(), MpvError> {
        self.set("ontop", if enabled { "yes" } else { "no" })?;
        self.set("options/border", if enabled { "no" } else { "yes" })?;
        self.set("window-scale", if enabled { "0.42" } else { "1.0" })
    }
    pub fn seek_chapter(&self, index: usize) -> Result<(), MpvError> {
        if index > 100_000 {
            return Err(MpvError::InvalidValue);
        }
        self.set("chapter", &index.to_string())
    }
    /// Save the current MPV output as an image. With window=true, libmpv
    /// captures its output window; otherwise it captures the video frame.
    pub fn screenshot_to_file(&self, path: &Path, window: bool) -> Result<(), MpvError> {
        let path = validate_screenshot_path(path)?;
        self.request(|reply| Command::ScreenshotToFile {
            path,
            window,
            reply,
        })
    }
    fn select(&self, kind: TrackKind, id: Option<i64>) -> Result<(), MpvError> {
        validate_track_id(id)?;
        self.request(|reply| Command::Select { kind, id, reply })
    }
    fn set(&self, property: &'static str, value: &str) -> Result<(), MpvError> {
        self.request(|reply| Command::Set {
            property,
            value: value.to_owned(),
            reply,
        })
    }
    fn request(
        &self,
        f: impl FnOnce(SyncSender<Result<(), MpvError>>) -> Command,
    ) -> Result<(), MpvError> {
        let (reply, rx) = mpsc::sync_channel(1);
        self.inner
            .tx
            .send(f(reply))
            .map_err(|_| MpvError::WorkerStopped)?;
        rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| MpvError::Timeout)?
    }
    /// Queue stop without waiting. Safe to call from a UI callback.
    pub fn request_stop(&self) -> Result<(), MpvError> {
        self.inner
            .tx
            .send(Command::Stop(None))
            .map_err(|_| MpvError::WorkerStopped)
    }
    /// Wait for MPV's end-file/idle acknowledgement. Call from a blocking worker.
    pub fn stop(&self, timeout: Duration) -> Result<(), MpvError> {
        let (reply, rx) = mpsc::sync_channel(1);
        self.inner
            .tx
            .send(Command::Stop(Some(reply)))
            .map_err(|_| MpvError::WorkerStopped)?;
        rx.recv_timeout(timeout).map_err(|_| MpvError::Timeout)?
    }
    pub fn shutdown(&self) -> Result<(), MpvError> {
        let worker = {
            let mut slot = self
                .inner
                .worker
                .lock()
                .map_err(|_| MpvError::WorkerStopped)?;
            slot.take()
        };
        let Some(worker) = worker else {
            return Ok(());
        };
        let (reply, rx) = mpsc::sync_channel(1);
        let send = self
            .inner
            .tx
            .send(Command::Shutdown(Some(reply)))
            .map_err(|_| MpvError::WorkerStopped);
        let result = match send {
            Ok(()) => match rx.recv_timeout(Duration::from_secs(8)) {
                Ok(result) => result,
                Err(_) => Err(MpvError::Timeout),
            },
            Err(error) => Err(error),
        };
        let joined = worker.join();
        if joined.is_err() && result.is_ok() {
            return Err(MpvError::WorkerStopped);
        }
        result
    }
}

fn worker_main(
    config: MpvConfig,
    rx: Receiver<Command>,
    ready: SyncSender<Result<(), MpvError>>,
    snapshot: Arc<RwLock<PlaybackSnapshot>>,
    listeners: Arc<Mutex<Vec<Sender<PlayerEvent>>>>,
) {
    let api = match Api::load(config.library_path.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let ctx = unsafe { (api.create)() };
    if ctx.is_null() {
        let _ = ready.send(Err(MpvError::Initialization(
            "mpv_create returned null".into(),
        )));
        return;
    }
    for (key, value) in options(&config) {
        if let Err(e) = api.option(ctx, &key, &value) {
            unsafe { (api.destroy)(ctx) };
            let _ = ready.send(Err(e));
            return;
        }
    }
    let initialized = unsafe { (api.initialize)(ctx) };
    if initialized < 0 {
        let e = api.error(initialized);
        unsafe { (api.destroy)(ctx) };
        let _ = ready.send(Err(MpvError::Initialization(e)));
        return;
    }
    if let Some(logs) = api.request_logs
        && let Ok(level) = CString::new("warn")
    {
        unsafe {
            let _ = logs(ctx, level.as_ptr());
        }
    }
    let _ = ready.send(Ok(()));
    diagnostic(
        &snapshot,
        &listeners,
        DiagnosticLevel::Info,
        "libmpv initialized on its owner thread",
    );
    let mut state = PlaybackStatus::Idle;
    let mut stop_waiter: Option<SyncSender<Result<(), MpvError>>> = None;
    let mut shutdown_waiter: Option<SyncSender<Result<(), MpvError>>> = None;
    let mut last_snapshot = Instant::now();
    let mut quitting = false;
    while !quitting {
        loop {
            match rx.try_recv() {
                Ok(command) => {
                    quitting = process(
                        command,
                        &api,
                        ctx,
                        &snapshot,
                        &listeners,
                        &mut state,
                        &mut stop_waiter,
                        &mut shutdown_waiter,
                    )
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    quitting = true;
                    break;
                }
            }
            if quitting {
                break;
            }
        }
        if quitting {
            break;
        }
        let event = unsafe { (api.wait_event)(ctx, POLL) };
        if !event.is_null() {
            let id = unsafe { (*event).event_id };
            event_update(
                id,
                &api,
                ctx,
                &snapshot,
                &listeners,
                &mut state,
                &mut stop_waiter,
            );
        }
        if last_snapshot.elapsed() >= Duration::from_millis(200) {
            update_snapshot(&api, ctx, &snapshot, &listeners, state);
            last_snapshot = Instant::now();
        }
    }
    unsafe { (api.destroy)(ctx) };
    if let Some(reply) = stop_waiter.take() {
        let _ = reply.send(Ok(()));
    }
    if let Some(reply) = shutdown_waiter.take() {
        let _ = reply.send(Ok(()));
    }
    if let Ok(mut s) = snapshot.write() {
        s.status = PlaybackStatus::Stopped;
        emit(&listeners, PlayerEvent::Snapshot(Box::new(s.clone())));
    }
}

#[allow(clippy::too_many_arguments)]
fn process(
    command: Command,
    api: &Api,
    ctx: *mut c_void,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    listeners: &Arc<Mutex<Vec<Sender<PlayerEvent>>>>,
    state: &mut PlaybackStatus,
    stop_waiter: &mut Option<SyncSender<Result<(), MpvError>>>,
    shutdown_waiter: &mut Option<SyncSender<Result<(), MpvError>>>,
) -> bool {
    match command {
        Command::Load {
            url,
            headers,
            reply,
        } => {
            let value = headers
                .iter()
                .map(|(k, v)| format!("{k}: {}", mpv_escape(v)))
                .collect::<Vec<_>>()
                .join(",");
            let result = api
                .set(ctx, HTTP_HEADER_OPTION_PROPERTY, &value)
                .and_then(|_| api.cmd(ctx, &["loadfile", &url, "replace"]));
            match result {
                Ok(()) => {
                    *state = PlaybackStatus::Loading;
                    set_state(snapshot, listeners, *state);
                    diagnostic(
                        snapshot,
                        listeners,
                        DiagnosticLevel::Info,
                        "MPV accepted a source replacement",
                    );
                    let _ = reply.send(Ok(()));
                }
                Err(e) => {
                    *state = PlaybackStatus::Error;
                    set_state(snapshot, listeners, *state);
                    diagnostic(
                        snapshot,
                        listeners,
                        DiagnosticLevel::Error,
                        &format!("MPV source load failed: {e}"),
                    );
                    let _ = reply.send(Err(e));
                }
            }
            false
        }
        Command::Set {
            property,
            value,
            reply,
        } => send_reply(api.set(ctx, property, &value), reply, snapshot, listeners),
        Command::CyclePause(reply) => send_reply(
            api.cmd(ctx, &["cycle", "pause"]),
            reply,
            snapshot,
            listeners,
        ),
        Command::Seek {
            seconds,
            relative,
            reply,
        } => {
            let mode = if relative {
                "relative+exact"
            } else {
                "absolute+exact"
            };
            send_reply(
                api.cmd(ctx, &["seek", &format!("{seconds:.3}"), mode]),
                reply,
                snapshot,
                listeners,
            )
        }
        Command::Select { kind, id, reply } => {
            let prop = if kind == TrackKind::Audio {
                "aid"
            } else {
                "sid"
            };
            let value = id.map_or_else(|| "no".to_owned(), |v| v.to_string());
            send_reply(api.set(ctx, prop, &value), reply, snapshot, listeners)
        }
        Command::AddSubtitle {
            url,
            headers,
            reply,
        } => {
            let value = headers
                .iter()
                .map(|(key, value)| format!("{key}: {}", mpv_escape(value)))
                .collect::<Vec<_>>()
                .join(",");
            let result = api
                .set(ctx, HTTP_HEADER_OPTION_PROPERTY, &value)
                .and_then(|_| api.cmd(ctx, &["sub-add", &url, "select"]));
            send_reply(result, reply, snapshot, listeners)
        }
        Command::ScreenshotToFile {
            path,
            window,
            reply,
        } => {
            let target = if window { "window" } else { "video" };
            send_reply(
                api.cmd(ctx, &["screenshot-to-file", &path, target]),
                reply,
                snapshot,
                listeners,
            )
        }
        Command::Stop(reply) => {
            if matches!(
                *state,
                PlaybackStatus::Idle | PlaybackStatus::Stopped | PlaybackStatus::Ended
            ) {
                if let Some(tx) = reply {
                    let _ = tx.send(Ok(()));
                }
            } else {
                if let Some(previous) = stop_waiter.take() {
                    let _ = previous.send(Ok(()));
                }
                *stop_waiter = reply;
                if let Err(e) = api.cmd(ctx, &["stop"]) {
                    if let Some(tx) = stop_waiter.take() {
                        let _ = tx.send(Err(e.clone()));
                    }
                    diagnostic(
                        snapshot,
                        listeners,
                        DiagnosticLevel::Error,
                        &format!("MPV stop failed: {e}"),
                    );
                }
            }
            false
        }
        Command::Shutdown(reply) => {
            if let Some(tx) = stop_waiter.take() {
                let _ = tx.send(Ok(()));
            }
            *shutdown_waiter = reply;
            true
        }
    }
}
fn send_reply(
    result: Result<(), MpvError>,
    reply: SyncSender<Result<(), MpvError>>,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    listeners: &Arc<Mutex<Vec<Sender<PlayerEvent>>>>,
) -> bool {
    if let Err(e) = &result {
        diagnostic(
            snapshot,
            listeners,
            DiagnosticLevel::Error,
            &format!("MPV command failed: {e}"),
        );
    }
    let _ = reply.send(result);
    false
}
fn event_update(
    id: c_int,
    api: &Api,
    ctx: *mut c_void,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    listeners: &Arc<Mutex<Vec<Sender<PlayerEvent>>>>,
    state: &mut PlaybackStatus,
    stop_waiter: &mut Option<SyncSender<Result<(), MpvError>>>,
) {
    match id {
        EV_START => {
            *state = PlaybackStatus::Loading;
            set_state(snapshot, listeners, *state);
        }
        EV_LOADED | EV_RESTART => {
            *state = if api.flag(ctx, "pause").unwrap_or(false) {
                PlaybackStatus::Paused
            } else {
                PlaybackStatus::Playing
            };
            set_state(snapshot, listeners, *state);
        }
        EV_PAUSE => {
            *state = PlaybackStatus::Paused;
            set_state(snapshot, listeners, *state);
        }
        EV_UNPAUSE => {
            *state = PlaybackStatus::Playing;
            set_state(snapshot, listeners, *state);
        }
        EV_END | EV_IDLE => {
            if stop_waiter.is_some() {
                *state = PlaybackStatus::Stopped;
            } else if matches!(
                *state,
                PlaybackStatus::Loading | PlaybackStatus::Playing | PlaybackStatus::Paused
            ) {
                *state = PlaybackStatus::Ended;
            }
            set_state(snapshot, listeners, *state);
            if let Some(tx) = stop_waiter.take() {
                let _ = tx.send(Ok(()));
            }
        }
        EV_SHUTDOWN => {
            *state = PlaybackStatus::Stopped;
            set_state(snapshot, listeners, *state);
        }
        EV_LOG => diagnostic(
            snapshot,
            listeners,
            DiagnosticLevel::Warning,
            "libmpv emitted a warning/error log event; consult the native MPV log",
        ),
        EV_PROPERTY => update_snapshot(api, ctx, snapshot, listeners, *state),
        _ => {}
    }
}
fn synchronized_status(state: PlaybackStatus, paused: bool) -> PlaybackStatus {
    match state {
        PlaybackStatus::Playing | PlaybackStatus::Paused => {
            if paused {
                PlaybackStatus::Paused
            } else {
                PlaybackStatus::Playing
            }
        }
        other => other,
    }
}

fn update_snapshot(
    api: &Api,
    ctx: *mut c_void,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    listeners: &Arc<Mutex<Vec<Sender<PlayerEvent>>>>,
    state: PlaybackStatus,
) {
    let mut next = match snapshot.read() {
        Ok(v) => v.clone(),
        Err(_) => return,
    };
    next.media_title = api.string(ctx, "media-title");
    next.position_seconds = api
        .double(ctx, "time-pos")
        .filter(|v| v.is_finite() && *v >= 0.0);
    next.duration_seconds = api
        .double(ctx, "duration")
        .filter(|v| v.is_finite() && *v >= 0.0);
    next.volume_percent = api.double(ctx, "volume").unwrap_or(100.0).clamp(0.0, 100.0);
    next.muted = api.flag(ctx, "mute").unwrap_or(false);
    next.paused = api.flag(ctx, "pause").unwrap_or(false);
    next.status = synchronized_status(state, next.paused);
    next.speed = api.double(ctx, "speed").unwrap_or(1.0).clamp(0.25, 4.0);
    next.video_codec = api.string(ctx, "video-codec");
    next.audio_codec = api.string(ctx, "audio-codec");
    next.audio_output_driver = api.string(ctx, "current-ao");
    next.audio_device_setting = api.string(ctx, "audio-device");
    next.audio_input_channels = api.string(ctx, "audio-params/hr-channels");
    next.audio_output_channels = api.string(ctx, "audio-out-params/hr-channels");
    next.audio_output_sample_rate = api.int(ctx, "audio-out-params/samplerate");
    next.audio_output_sample_format = api.string(ctx, "audio-out-params/format");
    next.audio_track_id = api.int(ctx, "aid");
    next.video_width = api.int(ctx, "width");
    next.video_height = api.int(ctx, "height");
    next.buffered_percent = api
        .double(ctx, "cache-buffering-state")
        .unwrap_or(100.0)
        .clamp(0.0, 100.0);
    next.buffering = api.flag(ctx, "paused-for-cache").unwrap_or(false)
        || (matches!(state, PlaybackStatus::Loading | PlaybackStatus::Playing)
            && next.buffered_percent < 100.0);
    next.tracks = tracks(api, ctx);
    next.chapters = chapters(api, ctx);
    if let Ok(mut current) = snapshot.write() {
        let audio_route_changed = current.audio_output_driver != next.audio_output_driver
            || current.audio_device_setting != next.audio_device_setting
            || current.audio_input_channels != next.audio_input_channels
            || current.audio_output_channels != next.audio_output_channels
            || current.audio_output_sample_rate != next.audio_output_sample_rate
            || current.audio_output_sample_format != next.audio_output_sample_format
            || current.audio_track_id != next.audio_track_id;
        if audio_route_changed {
            tracing::info!(
                output_driver = next.audio_output_driver.as_deref().unwrap_or("not active"),
                device_setting = next.audio_device_setting.as_deref().unwrap_or("not reported"),
                input_channels = next.audio_input_channels.as_deref().unwrap_or("not reported"),
                output_channels = next.audio_output_channels.as_deref().unwrap_or("not reported"),
                output_sample_rate = ?next.audio_output_sample_rate,
                output_sample_format = next.audio_output_sample_format.as_deref().unwrap_or("not reported"),
                audio_track_id = ?next.audio_track_id,
                codec = next.audio_codec.as_deref().unwrap_or("not reported"),
                muted = next.muted,
                volume_percent = next.volume_percent,
                "MPV audio output route changed"
            );
        }
        if *current != next {
            *current = next.clone();
            emit(listeners, PlayerEvent::Snapshot(Box::new(next)));
        }
    }
}
fn chapters(api: &Api, ctx: *mut c_void) -> Vec<PlaybackChapter> {
    let count = api
        .int(ctx, "chapter-list/count")
        .unwrap_or(0)
        .clamp(0, 512) as usize;
    (0..count)
        .filter_map(|index| {
            let base = format!("chapter-list/{index}");
            let start_seconds = api.double(ctx, &format!("{base}/time"))?;
            if !start_seconds.is_finite() || start_seconds < 0.0 {
                return None;
            }
            Some(PlaybackChapter {
                index,
                start_seconds,
                title: api.string(ctx, &format!("{base}/title")),
            })
        })
        .collect()
}
fn tracks(api: &Api, ctx: *mut c_void) -> Vec<MediaTrack> {
    let count = api
        .int(ctx, "track-list/count")
        .unwrap_or(0)
        .clamp(0, MAX_TRACKS as i64) as usize;
    let mut result = Vec::new();
    for i in 0..count {
        let base = format!("track-list/{i}");
        let kind = match api.string(ctx, &format!("{base}/type")).as_deref() {
            Some("audio") => TrackKind::Audio,
            Some("sub") => TrackKind::Subtitle,
            _ => continue,
        };
        let Some(id) = api.int(ctx, &format!("{base}/id")) else {
            continue;
        };
        result.push(MediaTrack {
            id,
            kind,
            language: api.string(ctx, &format!("{base}/lang")),
            title: api.string(ctx, &format!("{base}/title")),
            codec: api.string(ctx, &format!("{base}/codec")),
            selected: api.flag(ctx, &format!("{base}/selected")).unwrap_or(false),
            external: api.flag(ctx, &format!("{base}/external")).unwrap_or(false),
        });
    }
    result
}
fn set_state(
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    listeners: &Arc<Mutex<Vec<Sender<PlayerEvent>>>>,
    state: PlaybackStatus,
) {
    if let Ok(mut s) = snapshot.write()
        && s.status != state
    {
        s.status = state;
        emit(listeners, PlayerEvent::Snapshot(Box::new(s.clone())));
    }
}
fn diagnostic(
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    listeners: &Arc<Mutex<Vec<Sender<PlayerEvent>>>>,
    level: DiagnosticLevel,
    message: &str,
) {
    let item = PlayerDiagnostic {
        level,
        message: message.chars().take(512).collect(),
    };
    if let Ok(mut s) = snapshot.write() {
        if s.diagnostics.len() >= MAX_DIAGNOSTICS {
            s.diagnostics.remove(0);
        }
        s.diagnostics.push(item.clone());
    }
    emit(listeners, PlayerEvent::Diagnostic(item));
}
fn emit(listeners: &Arc<Mutex<Vec<Sender<PlayerEvent>>>>, event: PlayerEvent) {
    if let Ok(mut v) = listeners.lock() {
        v.retain(|tx| tx.send(event.clone()).is_ok());
    }
}
fn options(config: &MpvConfig) -> Vec<(String, String)> {
    let mut v = vec![
        ("config".into(), "no".into()),
        ("terminal".into(), "no".into()),
        ("idle".into(), "yes".into()),
        ("keep-open".into(), "no".into()),
        (
            "force-window".into(),
            if config.force_window.unwrap_or(false) {
                "yes"
            } else {
                "no"
            }
            .into(),
        ),
        (
            "vo".into(),
            config.video_output.clone().unwrap_or_else(|| "gpu".into()),
        ),
        (
            "ao".into(),
            config.audio_output.clone().unwrap_or_else(|| "auto".into()),
        ),
        ("hwdec".into(), "auto-safe".into()),
        // A stereo-first output avoids sending 5.1 AC-3 to a typical two-channel
        // Windows endpoint. The AC-3/AAC/DTS decoder is asked to downmix too.
        ("audio-channels".into(), "stereo".into()),
        ("ad-lavc-downmix".into(), "yes".into()),
        ("osc".into(), "no".into()),
        ("osd-level".into(), "0".into()),
        ("input-default-bindings".into(), "yes".into()),
        ("audio-display".into(), "no".into()),
    ];
    if let Some(id) = config.window_id {
        v.push(("wid".into(), id.to_string()));
    }
    if let Some(api) = &config.gpu_api {
        v.push(("gpu-api".into(), api.clone()));
    }
    if let Some(context) = &config.gpu_context {
        v.push(("gpu-context".into(), context.clone()));
    }
    if config.start_paused {
        v.push(("pause".into(), "yes".into()));
    }
    // Opt-in native MPV log file for CI/runtime diagnosis. Disabled by default
    // because MPV logs may contain user-supplied stream locations.
    if let Some(path) = env::var_os("SENZO_MPV_LOG_FILE") {
        let path = PathBuf::from(path).to_string_lossy().into_owned();
        if !path.is_empty() && path.len() <= 4096 {
            v.push(("log-file".into(), path));
            v.push(("msg-level".into(), "all=debug".into()));
        }
    }
    v
}
fn validate_screenshot_path(path: &Path) -> Result<String, MpvError> {
    let raw = path.to_str().ok_or(MpvError::InvalidValue)?;
    if !path.is_absolute()
        || raw.is_empty()
        || raw.len() > 4096
        || raw.chars().any(char::is_control)
    {
        return Err(MpvError::InvalidValue);
    }
    Ok(raw.to_owned())
}

fn validate_url(raw: &str) -> Result<String, MpvError> {
    if raw.len() > 8192 || raw.chars().any(char::is_control) {
        return Err(MpvError::InvalidUrl);
    }
    let u = Url::parse(raw.trim()).map_err(|_| MpvError::InvalidUrl)?;
    if u.scheme() == "file" {
        let path = u.to_file_path().map_err(|_| MpvError::InvalidUrl)?;
        if u.host_str().is_some_and(|host| !host.is_empty()) || !path.is_file() {
            return Err(MpvError::InvalidUrl);
        }
        return Ok(u.to_string());
    }
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err(MpvError::InvalidUrl);
    }
    Ok(u.to_string())
}
fn validate_headers(
    headers: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<(String, String)>, MpvError> {
    if headers.len() > MAX_HEADERS {
        return Err(MpvError::InvalidHeaders);
    }
    let mut total = 0usize;
    let mut v = Vec::with_capacity(headers.len());
    for (k, value) in headers {
        if k.is_empty()
            || k.len() > 128
            || !k
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.len() > 4096
            || value.chars().any(|c| c.is_control() && c != '\t')
        {
            return Err(MpvError::InvalidHeaders);
        }
        total = total.saturating_add(k.len()).saturating_add(value.len());
        if total > MAX_HEADER_BYTES {
            return Err(MpvError::InvalidHeaders);
        }
        v.push((k.clone(), value.trim().to_owned()));
    }
    Ok(v)
}
fn mpv_escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace(',', "\\,")
}
fn cstring(v: &str) -> Result<CString, MpvError> {
    CString::new(v).map_err(|_| MpvError::InvalidValue)
}

#[repr(C)]
struct Event {
    event_id: c_int,
    _error: c_int,
    _reply_userdata: u64,
    _data: *mut c_void,
}
type Create = unsafe extern "C" fn() -> *mut c_void;
type SetOption = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type Initialize = unsafe extern "C" fn(*mut c_void) -> c_int;
type CommandFn = unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int;
type SetProperty = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type GetProperty = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int;
type GetPropertyString = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char;
type Free = unsafe extern "C" fn(*mut c_void);
type WaitEvent = unsafe extern "C" fn(*mut c_void, c_double) -> *const Event;
type Destroy = unsafe extern "C" fn(*mut c_void);
type ErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type RequestLogs = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
struct Api {
    _lib: Library,
    create: Create,
    option_fn: SetOption,
    initialize: Initialize,
    command: CommandFn,
    set_property: SetProperty,
    get_property: GetProperty,
    get_string: GetPropertyString,
    free: Free,
    wait_event: WaitEvent,
    destroy: Destroy,
    error_string: ErrorString,
    request_logs: Option<RequestLogs>,
}
impl Api {
    fn load(explicit: Option<&Path>) -> Result<Self, MpvError> {
        let executable = env::current_exe().ok();
        let candidates = library_candidates(explicit, executable.as_deref());
        let mut lib = None;
        let mut errors = Vec::new();
        for p in candidates {
            match unsafe { Library::new(&p) } {
                Ok(v) => {
                    lib = Some(v);
                    break;
                }
                Err(e) => errors.push(format!("{}: {e}", p.display())),
            }
        }
        let lib = lib.ok_or_else(|| MpvError::LibraryUnavailable(errors.join("; ")))?;
        unsafe fn sym<T: Copy>(lib: &Library, name: &'static [u8]) -> Result<T, MpvError> {
            // SAFETY: every name/signature pair below mirrors libmpv's documented C API.
            unsafe { lib.get::<T>(name) }
                .map(|s| *s)
                .map_err(|e| MpvError::LibraryUnavailable(e.to_string()))
        }
        // SAFETY: all resolved symbols use the libmpv C ABI definitions.
        Ok(unsafe {
            Self {
                create: sym(&lib, b"mpv_create\0")?,
                option_fn: sym(&lib, b"mpv_set_option_string\0")?,
                initialize: sym(&lib, b"mpv_initialize\0")?,
                command: sym(&lib, b"mpv_command\0")?,
                set_property: sym(&lib, b"mpv_set_property_string\0")?,
                get_property: sym(&lib, b"mpv_get_property\0")?,
                get_string: sym(&lib, b"mpv_get_property_string\0")?,
                free: sym(&lib, b"mpv_free\0")?,
                wait_event: sym(&lib, b"mpv_wait_event\0")?,
                destroy: sym(&lib, b"mpv_terminate_destroy\0")?,
                error_string: sym(&lib, b"mpv_error_string\0")?,
                request_logs: lib
                    .get::<RequestLogs>(b"mpv_request_log_messages\0")
                    .ok()
                    .map(|s| *s),
                _lib: lib,
            }
        })
    }
    fn check(&self, code: c_int) -> Result<(), MpvError> {
        if code >= 0 {
            Ok(())
        } else {
            Err(MpvError::Command(self.error(code)))
        }
    }
    fn error(&self, code: c_int) -> String {
        let p = unsafe { (self.error_string)(code) };
        if p.is_null() {
            return format!("libmpv error {code}");
        }
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
    fn option(&self, ctx: *mut c_void, k: &str, v: &str) -> Result<(), MpvError> {
        let k = cstring(k)?;
        let v = cstring(v)?;
        self.check(unsafe { (self.option_fn)(ctx, k.as_ptr(), v.as_ptr()) })
    }
    fn set(&self, ctx: *mut c_void, k: &str, v: &str) -> Result<(), MpvError> {
        let k = cstring(k)?;
        let v = cstring(v)?;
        self.check(unsafe { (self.set_property)(ctx, k.as_ptr(), v.as_ptr()) })
    }
    fn cmd(&self, ctx: *mut c_void, args: &[&str]) -> Result<(), MpvError> {
        let c = args
            .iter()
            .map(|v| cstring(v))
            .collect::<Result<Vec<_>, _>>()?;
        let mut p = c.iter().map(|v| v.as_ptr()).collect::<Vec<_>>();
        p.push(ptr::null());
        self.check(unsafe { (self.command)(ctx, p.as_ptr()) })
    }
    fn string(&self, ctx: *mut c_void, key: &str) -> Option<String> {
        let k = CString::new(key).ok()?;
        let p = unsafe { (self.get_string)(ctx, k.as_ptr()) };
        if p.is_null() {
            return None;
        }
        let v = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        unsafe { (self.free)(p.cast()) };
        (!v.is_empty()).then_some(v)
    }
    fn double(&self, ctx: *mut c_void, key: &str) -> Option<f64> {
        let k = CString::new(key).ok()?;
        let mut v = 0.0f64;
        let e =
            unsafe { (self.get_property)(ctx, k.as_ptr(), F_DOUBLE, (&mut v as *mut f64).cast()) };
        (e >= 0 && v.is_finite()).then_some(v)
    }
    fn int(&self, ctx: *mut c_void, key: &str) -> Option<i64> {
        let k = CString::new(key).ok()?;
        let mut v = 0i64;
        let e =
            unsafe { (self.get_property)(ctx, k.as_ptr(), F_INT64, (&mut v as *mut i64).cast()) };
        (e >= 0).then_some(v)
    }
    fn flag(&self, ctx: *mut c_void, key: &str) -> Option<bool> {
        let k = CString::new(key).ok()?;
        let mut v = 0i32;
        let e =
            unsafe { (self.get_property)(ctx, k.as_ptr(), F_FLAG, (&mut v as *mut i32).cast()) };
        (e >= 0).then_some(v != 0)
    }
}
fn library_candidates(explicit: Option<&Path>, executable: Option<&Path>) -> Vec<PathBuf> {
    if let Some(path) = explicit {
        return vec![path.to_path_buf()];
    }
    let mut candidates = Vec::new();
    if let Some(executable_directory) = executable.and_then(Path::parent) {
        for name in library_names() {
            let bundled = executable_directory.join(name);
            if bundled.is_file() {
                candidates.push(bundled);
            }
        }
    }
    candidates.extend(library_names().iter().map(PathBuf::from));
    candidates
}

#[cfg(target_os = "windows")]
fn library_names() -> &'static [&'static str] {
    &["mpv-2.dll", "libmpv-2.dll", "mpv-1.dll"]
}
#[cfg(target_os = "linux")]
fn library_names() -> &'static [&'static str] {
    &["libmpv.so.2", "libmpv.so"]
}
#[cfg(target_os = "macos")]
fn library_names() -> &'static [&'static str] {
    &["libmpv.2.dylib", "libmpv.dylib"]
}

fn validate_seek(seconds: f64) -> Result<(), MpvError> {
    if seconds.is_finite() && seconds.abs() <= 31_536_000.0 {
        Ok(())
    } else {
        Err(MpvError::InvalidValue)
    }
}
fn validate_volume(value: f64) -> Result<(), MpvError> {
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        Ok(())
    } else {
        Err(MpvError::InvalidValue)
    }
}
fn validate_speed(value: f64) -> Result<(), MpvError> {
    if value.is_finite() && (0.25..=4.0).contains(&value) {
        Ok(())
    } else {
        Err(MpvError::InvalidValue)
    }
}
fn validate_track_id(id: Option<i64>) -> Result<(), MpvError> {
    if id.is_some_and(|value| value < 0) {
        Err(MpvError::InvalidValue)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_mpv_library_is_searched_beside_the_application() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("senzo.exe");
        std::fs::write(&executable, b"test executable").unwrap();
        let bundled_library = directory.path().join(library_names()[0]);
        std::fs::write(&bundled_library, b"test library").unwrap();
        let candidates = library_candidates(None, Some(&executable));
        assert_eq!(candidates.first(), Some(&bundled_library));
    }

    #[test]
    fn explicit_mpv_library_path_is_respected_without_fallbacks() {
        let directory = tempfile::tempdir().unwrap();
        let explicit = directory.path().join("custom-mpv.dll");
        let candidates = library_candidates(Some(&explicit), None);
        assert_eq!(candidates, vec![explicit]);
    }

    #[test]
    fn packaged_windows_libmpv_runtime_initializes_headlessly() {
        let Some(path) = env::var_os("SENZO_LIBMPV_TEST_PATH").map(PathBuf::from) else {
            return;
        };
        let player = MpvPlayer::spawn(MpvConfig {
            library_path: Some(path),
            video_output: Some("null".to_owned()),
            audio_output: Some("null".to_owned()),
            force_window: Some(false),
            ..Default::default()
        })
        .expect("the packaged libmpv runtime must load and initialize");
        player
            .shutdown()
            .expect("the packaged libmpv runtime must shut down cleanly");
    }

    #[test]
    fn accepts_direct_sources() {
        assert!(validate_url("https://cdn.example/video.mkv").is_ok());
        assert!(validate_url("http://127.0.0.1/video").is_ok());
    }
    #[test]
    fn local_files_must_exist_before_they_can_be_played() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("offline.mp4");
        std::fs::write(&path, b"test media").unwrap();
        let url = Url::from_file_path(&path).unwrap();
        assert!(validate_url(url.as_str()).is_ok());
        let missing = Url::from_file_path(directory.path().join("missing.mp4")).unwrap();
        assert!(validate_url(missing.as_str()).is_err());
        assert!(validate_url("file://remote-host/share/video.mp4").is_err());
    }

    #[test]
    fn rejects_unsafe_source_targets() {
        for v in [
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
            "https://user:pass@host/video",
            "https://host/video#frag",
            "javascript:alert(1)",
        ] {
            assert!(validate_url(v).is_err());
        }
    }
    #[test]
    fn validates_header_safety_and_bounds() {
        let ok = std::collections::BTreeMap::from([
            ("Referer".into(), "https://provider.example/".into()),
            ("Authorization".into(), "Bearer abc".into()),
        ]);
        assert_eq!(validate_headers(&ok).unwrap().len(), 2);
        let bad =
            std::collections::BTreeMap::from([("X-Test".into(), "ok\r\nInjected: yes".into())]);
        assert!(validate_headers(&bad).is_err());
        let name = std::collections::BTreeMap::from([("Bad Header".into(), "x".into())]);
        assert!(validate_headers(&name).is_err());
    }
    #[test]
    fn rejects_out_of_range_controls_and_track_ids() {
        assert!(validate_seek(0.0).is_ok());
        assert!(validate_seek(f64::NAN).is_err());
        assert!(validate_seek(31_536_001.0).is_err());
        assert!(validate_volume(100.0).is_ok());
        assert!(validate_volume(100.1).is_err());
        assert!(validate_speed(0.25).is_ok());
        assert!(validate_speed(4.01).is_err());
        assert!(validate_track_id(Some(0)).is_ok());
        assert!(validate_track_id(Some(-1)).is_err());
    }
    #[test]
    fn screenshot_paths_must_be_absolute_and_free_of_control_characters() {
        assert!(validate_screenshot_path(Path::new("relative.png")).is_err());
        let valid = env::temp_dir().join("senzo-frame.png");
        assert_eq!(
            validate_screenshot_path(&valid).unwrap(),
            valid.to_string_lossy().into_owned()
        );
        let invalid = env::temp_dir().join("bad\nframe.png");
        assert!(validate_screenshot_path(&invalid).is_err());
        let too_long = env::temp_dir().join("x".repeat(4097));
        assert!(validate_screenshot_path(&too_long).is_err());
    }

    #[test]
    fn default_config_reads_optional_libmpv_path() {
        assert_eq!(MpvConfig::default().window_id, None);
        assert!(!MpvConfig::default().start_paused);
    }
    #[test]
    fn libmpv_event_ids_match_the_documented_client_abi() {
        assert_eq!(EV_END, 7);
        assert_eq!(EV_IDLE, 11);
        assert_eq!(EV_RESTART, 21);
    }
    #[test]
    fn snapshots_keep_pause_status_consistent_with_libmpv_property() {
        assert_eq!(
            synchronized_status(PlaybackStatus::Playing, true),
            PlaybackStatus::Paused
        );
        assert_eq!(
            synchronized_status(PlaybackStatus::Paused, false),
            PlaybackStatus::Playing
        );
        assert_eq!(
            synchronized_status(PlaybackStatus::Loading, true),
            PlaybackStatus::Loading
        );
        assert_eq!(
            synchronized_status(PlaybackStatus::Stopped, false),
            PlaybackStatus::Stopped
        );
    }

    #[test]
    fn request_headers_use_the_runtime_option_property() {
        assert_eq!(HTTP_HEADER_OPTION_PROPERTY, "options/http-header-fields");
    }
    #[test]
    fn escapes_mpv_header_list_delimiters() {
        assert_eq!(mpv_escape(r"a,b\c"), r"a\,b\\c");
    }
    #[test]
    fn config_can_select_virtual_video_and_audio_outputs() {
        let opts = options(&MpvConfig {
            video_output: Some("gpu".into()),
            audio_output: Some("pulse".into()),
            force_window: Some(true),
            gpu_api: Some("opengl".into()),
            gpu_context: Some("x11egl".into()),
            ..MpvConfig::default()
        });
        assert!(opts.contains(&("vo".into(), "gpu".into())));
        assert!(opts.contains(&("ao".into(), "pulse".into())));
        assert!(opts.contains(&("force-window".into(), "yes".into())));
        assert!(opts.contains(&("gpu-api".into(), "opengl".into())));
        assert!(opts.contains(&("gpu-context".into(), "x11egl".into())));
    }

    #[test]
    fn config_creates_native_gpu_output_options() {
        let opts = options(&MpvConfig {
            library_path: None,
            window_id: Some(42),
            start_paused: true,
            ..MpvConfig::default()
        });
        assert!(opts.contains(&("vo".into(), "gpu".into())));
        assert!(opts.contains(&("wid".into(), "42".into())));
        assert!(opts.contains(&("pause".into(), "yes".into())));
        assert!(opts.contains(&("config".into(), "no".into())));
        assert!(opts.contains(&("keep-open".into(), "no".into())));
        assert!(opts.contains(&("force-window".into(), "no".into())));
    }
}
