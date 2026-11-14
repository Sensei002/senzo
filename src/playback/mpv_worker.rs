//! Dedicated-thread command boundary for libmpv.
//!
//! The MPV library is loaded and every MPV call is made on one worker thread.
//! UI code communicates through a bounded command queue and a non-blocking event receiver.

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};

use super::{mpv_config::MpvConfig, mpv_runtime::MpvRuntime};

const COMMAND_QUEUE_CAPACITY: usize = 32;
const STATE_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// A lightweight snapshot of properties observed from the active MPV instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackTrack {
    pub id: u64,
    pub kind: String,
    pub title: Option<String>,
    pub language: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackState {
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub paused: Option<bool>,
    pub buffering: Option<bool>,
    pub media_title: Option<String>,
    pub volume: Option<f64>,
    pub muted: Option<bool>,
    pub fullscreen: Option<bool>,
    pub speed: Option<f64>,
    pub diagnostics: PlaybackDiagnostics,
    pub tracks: Vec<PlaybackTrack>,
}

/// Decoder and video-output information reported by the active MPV instance.
/// Values remain optional because MPV may not know them until a video is loaded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackDiagnostics {
    pub video_codec: Option<String>,
    pub video_decoder: Option<String>,
    pub hardware_decoder: Option<String>,
    pub video_output: Option<String>,
    pub video_width: Option<u32>,
    pub video_height: Option<u32>,
    pub frame_rate: Option<f64>,
    pub dropped_frames: Option<u64>,
}

impl PlaybackState {
    fn read(runtime: &MpvRuntime) -> Self {
        Self {
            position_seconds: read_number(runtime, "time-pos"),
            duration_seconds: read_number(runtime, "duration"),
            paused: read_boolean(runtime, "pause"),
            buffering: read_boolean(runtime, "paused-for-cache"),
            media_title: read_property(runtime, "media-title"),
            volume: read_number(runtime, "volume"),
            muted: read_boolean(runtime, "mute"),
            fullscreen: read_boolean(runtime, "fullscreen"),
            speed: read_number(runtime, "speed"),
            diagnostics: PlaybackDiagnostics {
                video_codec: read_property(runtime, "video-codec"),
                video_decoder: read_property(runtime, "video-dec-params/codec"),
                hardware_decoder: read_property(runtime, "hwdec-current"),
                video_output: read_property(runtime, "current-vo"),
                video_width: read_integer(runtime, "video-out-params/w"),
                video_height: read_integer(runtime, "video-out-params/h"),
                frame_rate: read_number(runtime, "estimated-vf-fps"),
                dropped_frames: read_unsigned(runtime, "frame-drop-count"),
            },
            tracks: read_tracks(runtime),
        }
    }
}

fn read_property(runtime: &MpvRuntime, name: &str) -> Option<String> {
    runtime.property_string(name).ok().flatten()
}

fn read_number(runtime: &MpvRuntime, name: &str) -> Option<f64> {
    read_property(runtime, name)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn read_boolean(runtime: &MpvRuntime, name: &str) -> Option<bool> {
    read_property(runtime, name).and_then(|value| match value.as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    })
}

fn read_integer(runtime: &MpvRuntime, name: &str) -> Option<u32> {
    read_property(runtime, name).and_then(|value| value.parse::<u32>().ok())
}

fn read_unsigned(runtime: &MpvRuntime, name: &str) -> Option<u64> {
    read_property(runtime, name).and_then(|value| value.parse::<u64>().ok())
}

fn read_tracks(runtime: &MpvRuntime) -> Vec<PlaybackTrack> {
    let count = read_number(runtime, "track-list/count")
        .filter(|count| *count >= 0.0 && *count <= 256.0)
        .map(|count| count as usize)
        .unwrap_or(0);

    (0..count)
        .filter_map(|index| {
            let property =
                |name: &str| read_property(runtime, &format!("track-list/{index}/{name}"));
            let kind = property("type")?;
            if kind != "audio" && kind != "sub" {
                return None;
            }
            let id = property("id")?.parse::<u64>().ok().filter(|id| *id > 0)?;
            Some(PlaybackTrack {
                id,
                kind,
                title: property("title").filter(|value| !value.is_empty()),
                language: property("lang").filter(|value| !value.is_empty()),
                selected: property("selected")
                    .is_some_and(|value| matches!(value.as_str(), "yes" | "true" | "1")),
            })
        })
        .collect()
}

/// Commands accepted by the playback worker.
enum PlaybackCommand {
    LoadFile(PathBuf, Option<f64>),
    LoadUrl(String),
    SetPaused(bool),
    SetVolume(f64),
    SetMuted(bool),
    SetFullscreen(bool),
    SetAudioTrack(u64),
    SetSubtitleTrack(Option<u64>),
    SetSubtitleDelay(f64),
    SetSubtitleFontSize(u8),
    SetSubtitleMargin(u8),
    SetSubtitleBold(bool),
    AddSubtitleFile(PathBuf),
    SeekAbsolute(f64),
    SetSpeed(f64),
    Stop,
    Shutdown,
}

/// Events delivered to the application without blocking its UI thread.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEvent {
    Ready,
    StateChanged(PlaybackState),
    CommandCompleted {
        operation: &'static str,
    },
    CommandFailed {
        operation: &'static str,
        message: String,
    },
    Failed {
        message: String,
    },
    Stopped,
}

struct PendingResume {
    position: f64,
    expected_path: String,
    not_before: Instant,
    deadline: Instant,
}

fn resume_target(position: f64, duration: f64) -> f64 {
    if !position.is_finite() || position < 0.0 || !duration.is_finite() || duration <= 0.0 {
        return 0.0;
    }
    position.min((duration - 0.5).max(0.0))
}

fn validate_local_stream_url(input: &str) -> anyhow::Result<String> {
    let url = url::Url::parse(input).context("local stream URL is invalid")?;
    let loopback = url
        .host()
        .is_some_and(|host| matches!(host, url::Host::Ipv4(ip) if ip.is_loopback()));
    if url.scheme() != "http"
        || !loopback
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
        || url.path_segments().is_none_or(|segments| segments.count() != 3)
    {
        bail!("local playback URLs must use the authenticated SENZO loopback stream endpoint");
    }
    Ok(url.to_string())
}

fn validate_playback_url(input: &str) -> anyhow::Result<String> {
    let url = url::Url::parse(input).context("stream URL is invalid")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        bail!("playback URLs must be absolute HTTP(S) URLs without credentials or fragments");
    }
    match url.host().expect("host was validated") {
        url::Host::Domain(host) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            if host == "localhost"
                || host.ends_with(".localhost")
                || host.ends_with(".local")
                || host.ends_with(".internal")
            {
                bail!("playback URLs cannot target local network hostnames");
            }
        }
        url::Host::Ipv4(ip)
            if ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast() =>
        {
            bail!("playback URLs cannot target local or reserved IP addresses")
        }
        url::Host::Ipv6(ip)
            if (ip.segments()[0] & 0xe000) != 0x2000
                || ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80 =>
        {
            bail!("playback URLs cannot target local or reserved IP addresses")
        }
        url::Host::Ipv4(_) | url::Host::Ipv6(_) => {}
    }
    Ok(url.to_string())
}

/// Handle for a single MPV instance running on its own thread.
///
/// The command queue is bounded so a stalled player cannot accumulate an
/// unbounded backlog. Calls return immediately with an error when the queue is full.
pub struct PlaybackWorker {
    commands: SyncSender<PlaybackCommand>,
    events: Receiver<PlaybackEvent>,
    thread: Option<JoinHandle<()>>,
}

impl PlaybackWorker {
    /// Start a worker and asynchronously load the explicitly supplied MPV DLL.
    ///
    /// The library is never searched for or downloaded automatically. The config
    /// is applied before MPV initialization on the worker thread.
    pub fn spawn(library_path: PathBuf, config: MpvConfig) -> anyhow::Result<Self> {
        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("senzo-playback".into())
            .spawn(move || {
                let runtime = match MpvRuntime::open(&library_path, &config) {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = event_tx.send(PlaybackEvent::Failed {
                            message: format!("{error:#}"),
                        });
                        return;
                    }
                };

                let _ = event_tx.send(PlaybackEvent::Ready);
                let mut previous_state: Option<PlaybackState> = None;
                let mut pending_resume: Option<PendingResume> = None;

                loop {
                    match command_rx.recv_timeout(STATE_POLL_INTERVAL) {
                        Ok(command) => {
                            let (operation, result, shutdown) = match command {
                                PlaybackCommand::LoadUrl(url) => {
                                    pending_resume = None;
                                    ("load_url", runtime.load_url(&url), false)
                                }
                                PlaybackCommand::LoadFile(path, resume_position) => {
                                    let expected_path = path.to_string_lossy().into_owned();
                                    pending_resume = None;
                                    let result = runtime.load_file(path);
                                    if result.is_ok() {
                                        pending_resume =
                                            resume_position.map(|position| PendingResume {
                                                position,
                                                expected_path,
                                                not_before: Instant::now()
                                                    + Duration::from_millis(500),
                                                deadline: Instant::now() + Duration::from_secs(30),
                                            });
                                    }
                                    ("load_file", result, false)
                                }
                                PlaybackCommand::SetPaused(paused) => {
                                    ("set_paused", runtime.set_paused(paused), false)
                                }
                                PlaybackCommand::SetVolume(volume) => {
                                    ("set_volume", runtime.set_volume(volume), false)
                                }
                                PlaybackCommand::SetMuted(muted) => {
                                    ("set_muted", runtime.set_muted(muted), false)
                                }
                                PlaybackCommand::SetFullscreen(fullscreen) => {
                                    ("set_fullscreen", runtime.set_fullscreen(fullscreen), false)
                                }
                                PlaybackCommand::SetAudioTrack(track_id) => {
                                    ("set_audio_track", runtime.set_audio_track(track_id), false)
                                }
                                PlaybackCommand::SetSubtitleTrack(track_id) => (
                                    "set_subtitle_track",
                                    runtime.set_subtitle_track(track_id),
                                    false,
                                ),
                                PlaybackCommand::SetSubtitleDelay(seconds) => (
                                    "set_subtitle_delay",
                                    runtime.set_subtitle_delay(seconds),
                                    false,
                                ),
                                PlaybackCommand::SetSubtitleFontSize(size) => (
                                    "set_subtitle_font_size",
                                    runtime.set_subtitle_font_size(size),
                                    false,
                                ),
                                PlaybackCommand::SetSubtitleMargin(margin) => (
                                    "set_subtitle_margin",
                                    runtime.set_subtitle_margin(margin),
                                    false,
                                ),
                                PlaybackCommand::SetSubtitleBold(bold) => {
                                    ("set_subtitle_bold", runtime.set_subtitle_bold(bold), false)
                                }
                                PlaybackCommand::AddSubtitleFile(path) => {
                                    ("add_subtitle_file", runtime.add_subtitle_file(path), false)
                                }
                                PlaybackCommand::SeekAbsolute(seconds) => {
                                    ("seek_absolute", runtime.seek_absolute(seconds), false)
                                }
                                PlaybackCommand::SetSpeed(speed) => {
                                    ("set_speed", runtime.set_speed(speed), false)
                                }
                                PlaybackCommand::Stop => ("stop", runtime.stop(), false),
                                PlaybackCommand::Shutdown => ("shutdown", Ok(()), true),
                            };

                            if shutdown {
                                break;
                            }

                            let event = match result {
                                Ok(()) => PlaybackEvent::CommandCompleted { operation },
                                Err(error) => PlaybackEvent::CommandFailed {
                                    operation,
                                    message: format!("{error:#}"),
                                },
                            };
                            let _ = event_tx.send(event);
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }

                    let state = PlaybackState::read(&runtime);
                    if let Some(pending) = pending_resume.as_ref() {
                        let current_path = read_property(&runtime, "path");
                        if Instant::now() >= pending.not_before
                            && current_path.as_deref().is_some_and(|path| {
                                path.eq_ignore_ascii_case(&pending.expected_path)
                            })
                            && let Some(duration) =
                                state.duration_seconds.filter(|value| *value > 0.0)
                        {
                            let result =
                                runtime.seek_absolute(resume_target(pending.position, duration));
                            let event = match result {
                                Ok(()) => PlaybackEvent::CommandCompleted {
                                    operation: "resume_seek",
                                },
                                Err(error) => PlaybackEvent::CommandFailed {
                                    operation: "resume_seek",
                                    message: format!("{error:#}"),
                                },
                            };
                            let _ = event_tx.send(event);
                            pending_resume = None;
                        } else if Instant::now() >= pending.deadline {
                            let _ = event_tx.send(PlaybackEvent::CommandFailed {
                                operation: "resume_seek",
                                message: "the media did not become ready within 30 seconds".into(),
                            });
                            pending_resume = None;
                        }
                    }
                    if previous_state.as_ref() != Some(&state) {
                        previous_state = Some(state.clone());
                        let _ = event_tx.send(PlaybackEvent::StateChanged(state));
                    }
                }

                drop(runtime);
                let _ = event_tx.send(PlaybackEvent::Stopped);
            })
            .context("could not start the SENZO playback worker thread")?;

        Ok(Self {
            commands: command_tx,
            events: event_rx,
            thread: Some(thread),
        })
    }

    /// Queue a local media file for playback.
    pub fn load_file(&self, path: impl Into<PathBuf>) -> anyhow::Result<()> {
        self.load_file_with_resume(path, None)
    }

    /// Queue a file and defer resume until MPV reports the new path and duration.
    pub fn load_file_with_resume(
        &self,
        path: impl Into<PathBuf>,
        resume_position: Option<f64>,
    ) -> anyhow::Result<()> {
        if resume_position.is_some_and(|position| !position.is_finite() || position < 0.0) {
            bail!("resume position must be finite and non-negative");
        }
        self.enqueue(PlaybackCommand::LoadFile(path.into(), resume_position))
    }

    /// Queue a validated HTTP(S) stream URL for MPV playback.
    pub fn load_url(&self, input: &str) -> anyhow::Result<()> {
        let url = validate_playback_url(input)?;
        self.enqueue(PlaybackCommand::LoadUrl(url))
    }

    /// Queue a URL served by SENZO's token-protected loopback torrent server.
    /// This narrow path permits 127.0.0.1; general provider URLs do not.
    pub fn load_local_url(&self, input: &str) -> anyhow::Result<()> {
        let url = validate_local_stream_url(input)?;
        self.enqueue(PlaybackCommand::LoadUrl(url))
    }

    /// Queue a pause or resume transition.
    pub fn set_paused(&self, paused: bool) -> anyhow::Result<()> {
        self.enqueue(PlaybackCommand::SetPaused(paused))
    }

    /// Queue an output volume change from 0 to 100 percent.
    pub fn set_volume(&self, volume: f64) -> anyhow::Result<()> {
        if !volume.is_finite() || !(0.0..=100.0).contains(&volume) {
            bail!("volume must be finite and between 0 and 100");
        }
        self.enqueue(PlaybackCommand::SetVolume(volume))
    }

    /// Queue an audio mute transition.
    pub fn set_muted(&self, muted: bool) -> anyhow::Result<()> {
        self.enqueue(PlaybackCommand::SetMuted(muted))
    }

    /// Queue a fullscreen transition.
    pub fn set_fullscreen(&self, fullscreen: bool) -> anyhow::Result<()> {
        self.enqueue(PlaybackCommand::SetFullscreen(fullscreen))
    }

    /// Queue selection of an audio track using its MPV track ID.
    pub fn set_audio_track(&self, track_id: u64) -> anyhow::Result<()> {
        if track_id == 0 {
            bail!("track ID must be greater than zero");
        }
        self.enqueue(PlaybackCommand::SetAudioTrack(track_id))
    }

    /// Queue subtitle selection; `None` disables subtitles.
    pub fn set_subtitle_track(&self, track_id: Option<u64>) -> anyhow::Result<()> {
        if track_id == Some(0) {
            bail!("track ID must be greater than zero");
        }
        self.enqueue(PlaybackCommand::SetSubtitleTrack(track_id))
    }

    /// Queue a subtitle timing offset in seconds.
    pub fn set_subtitle_delay(&self, seconds: f64) -> anyhow::Result<()> {
        if !seconds.is_finite() || !(-10.0..=10.0).contains(&seconds) {
            bail!("subtitle delay must be finite and between -10 and 10 seconds");
        }
        self.enqueue(PlaybackCommand::SetSubtitleDelay(seconds))
    }

    /// Queue a subtitle font size change.
    pub fn set_subtitle_font_size(&self, size: u8) -> anyhow::Result<()> {
        if !(12..=96).contains(&size) {
            bail!("subtitle font size must be between 12 and 96");
        }
        self.enqueue(PlaybackCommand::SetSubtitleFontSize(size))
    }

    /// Queue a subtitle vertical margin from 0 to 100 pixels.
    pub fn set_subtitle_margin(&self, margin: u8) -> anyhow::Result<()> {
        if margin > 100 {
            bail!("subtitle vertical margin must be between 0 and 100 pixels");
        }
        self.enqueue(PlaybackCommand::SetSubtitleMargin(margin))
    }

    /// Queue the subtitle bold preference.
    pub fn set_subtitle_bold(&self, bold: bool) -> anyhow::Result<()> {
        self.enqueue(PlaybackCommand::SetSubtitleBold(bold))
    }

    /// Queue a user-selected external subtitle file.
    pub fn add_subtitle_file(&self, path: impl Into<PathBuf>) -> anyhow::Result<()> {
        self.enqueue(PlaybackCommand::AddSubtitleFile(path.into()))
    }

    /// Queue an absolute seek position in seconds.
    pub fn seek_absolute(&self, seconds: f64) -> anyhow::Result<()> {
        if !seconds.is_finite() || seconds < 0.0 {
            bail!("absolute seek position must be finite and non-negative");
        }
        self.enqueue(PlaybackCommand::SeekAbsolute(seconds))
    }

    /// Queue a playback-speed change.
    pub fn set_speed(&self, speed: f64) -> anyhow::Result<()> {
        if !speed.is_finite() || !(0.01..=100.0).contains(&speed) {
            bail!("playback speed must be finite and between 0.01 and 100");
        }
        self.enqueue(PlaybackCommand::SetSpeed(speed))
    }

    /// Queue a stop command.
    pub fn stop(&self) -> anyhow::Result<()> {
        self.enqueue(PlaybackCommand::Stop)
    }

    /// Read one pending event without waiting for the playback thread.
    pub fn try_event(&self) -> anyhow::Result<Option<PlaybackEvent>> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn enqueue(&self, command: PlaybackCommand) -> anyhow::Result<()> {
        match self.commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                Err(anyhow!("playback command queue is full; try again shortly"))
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err(anyhow!("playback worker is no longer available"))
            }
        }
    }
}

impl Drop for PlaybackWorker {
    fn drop(&mut self) {
        // A blocking send is intentional here: it waits for a queue slot while
        // the worker drains prior commands, then guarantees orderly shutdown.
        let _ = self.commands.send(PlaybackCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_playback_url;
    #[test]
    fn accepts_only_the_authenticated_loopback_stream_shape() {
        assert!(validate_local_stream_url("http://127.0.0.1:3456/token/1/0").is_ok());
        assert!(validate_local_stream_url("http://localhost:3456/token/1/0").is_err());
        assert!(validate_local_stream_url("http://127.0.0.1:3456/token/1/0?x=1").is_err());
        assert!(validate_local_stream_url("https://127.0.0.1:3456/token/1/0").is_err());
        assert!(validate_local_stream_url("http://192.168.1.2:3456/token/1/0").is_err());
    }

    fn validates_direct_playback_urls() {
        assert!(validate_playback_url("https://cdn.example.org/video.mp4").is_ok());
        assert!(validate_playback_url("file:///C:/video.mp4").is_err());
        assert!(validate_playback_url("http://127.0.0.1/video.mp4").is_err());
        assert!(validate_playback_url("http://localhost/video.mp4").is_err());
        assert!(validate_playback_url("https://user:pass@example.org/video.mp4").is_err());
        assert!(validate_playback_url("https://example.org/video.mp4#fragment").is_err());
    }

    use super::*;

    #[test]
    fn invalid_seek_speed_volume_and_track_ids_are_rejected_before_queueing() {
        let (command_tx, command_rx) = mpsc::sync_channel(8);
        let (_event_tx, event_rx) = mpsc::channel();
        let worker = PlaybackWorker {
            commands: command_tx,
            events: event_rx,
            thread: None,
        };

        assert!(worker.seek_absolute(-1.0).is_err());
        assert!(worker.seek_absolute(f64::NAN).is_err());
        assert!(worker.set_speed(0.0).is_err());
        assert!(worker.set_speed(101.0).is_err());
        assert!(worker.set_volume(-1.0).is_err());
        assert!(worker.set_volume(101.0).is_err());
        assert!(worker.set_audio_track(0).is_err());
        assert!(worker.set_subtitle_track(Some(0)).is_err());
        assert!(worker.set_subtitle_track(None).is_ok());
        assert!(worker.set_subtitle_delay(0.5).is_ok());
        assert!(worker.set_subtitle_delay(11.0).is_err());
        assert!(worker.set_subtitle_font_size(42).is_ok());
        assert!(worker.set_subtitle_font_size(100).is_err());
        assert!(worker.set_subtitle_margin(24).is_ok());
        assert!(worker.set_subtitle_margin(101).is_err());
        assert!(worker.set_subtitle_bold(true).is_ok());
        for _ in 0..5 {
            assert!(command_rx.try_recv().is_ok());
        }
        assert!(command_rx.try_recv().is_err());
    }

    #[test]
    fn resume_target_stays_inside_the_loaded_media_duration() {
        assert_eq!(resume_target(45.0, 120.0), 45.0);
        assert_eq!(resume_target(180.0, 120.0), 119.5);
        assert_eq!(resume_target(180.0, 0.25), 0.0);
        assert_eq!(resume_target(f64::NAN, 120.0), 0.0);
    }

    #[test]
    fn missing_runtime_is_reported_as_an_event_instead_of_panicking() {
        let worker = PlaybackWorker::spawn(
            PathBuf::from("this-file-does-not-exist-mpv-2.dll"),
            MpvConfig::default(),
        )
        .unwrap();

        let event = worker.events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(event, PlaybackEvent::Failed { .. }));
    }

    #[test]
    fn boolean_properties_accept_mpv_boolean_representations() {
        assert_eq!(parse_boolean("yes"), Some(true));
        assert_eq!(parse_boolean("false"), Some(false));
        assert_eq!(parse_boolean("1"), Some(true));
        assert_eq!(parse_boolean("0"), Some(false));
        assert_eq!(parse_boolean("unknown"), None);
    }

    #[test]
    fn integer_diagnostics_reject_invalid_and_negative_values() {
        assert_eq!("1920".parse::<u32>().ok(), Some(1920));
        assert_eq!("-1".parse::<u32>().ok(), None);
        assert_eq!("unknown".parse::<u64>().ok(), None);
    }

    #[test]
    fn numeric_properties_reject_non_finite_values() {
        assert_eq!(parse_number("12.5"), Some(12.5));
        assert_eq!(parse_number("NaN"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("not-a-number"), None);
    }

    fn parse_boolean(value: &str) -> Option<bool> {
        match value {
            "yes" | "true" | "1" => Some(true),
            "no" | "false" | "0" => Some(false),
            _ => None,
        }
    }

    fn parse_number(value: &str) -> Option<f64> {
        value.parse::<f64>().ok().filter(|value| value.is_finite())
    }
}
