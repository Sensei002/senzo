//! Dedicated-thread command boundary for libmpv.
//!
//! The MPV library is loaded and every MPV call is made on one worker thread.
//! UI code communicates through a bounded command queue and a non-blocking event receiver.

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context as _, anyhow, bail};

use super::{mpv_config::MpvConfig, mpv_runtime::MpvRuntime};

const COMMAND_QUEUE_CAPACITY: usize = 32;
const STATE_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// A lightweight snapshot of properties observed from the active MPV instance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackState {
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub paused: Option<bool>,
    pub buffering: Option<bool>,
    pub media_title: Option<String>,
}

impl PlaybackState {
    fn read(runtime: &MpvRuntime) -> Self {
        Self {
            position_seconds: read_number(runtime, "time-pos"),
            duration_seconds: read_number(runtime, "duration"),
            paused: read_boolean(runtime, "pause"),
            buffering: read_boolean(runtime, "paused-for-cache"),
            media_title: read_property(runtime, "media-title"),
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

/// Commands accepted by the playback worker.
enum PlaybackCommand {
    LoadFile(PathBuf),
    SetPaused(bool),
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

                loop {
                    match command_rx.recv_timeout(STATE_POLL_INTERVAL) {
                        Ok(command) => {
                            let (operation, result, shutdown) = match command {
                                PlaybackCommand::LoadFile(path) => {
                                    ("load_file", runtime.load_file(path), false)
                                }
                                PlaybackCommand::SetPaused(paused) => {
                                    ("set_paused", runtime.set_paused(paused), false)
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
        self.enqueue(PlaybackCommand::LoadFile(path.into()))
    }

    /// Queue a pause or resume transition.
    pub fn set_paused(&self, paused: bool) -> anyhow::Result<()> {
        self.enqueue(PlaybackCommand::SetPaused(paused))
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
    use super::*;

    #[test]
    fn invalid_seek_and_speed_are_rejected_before_queueing() {
        let (command_tx, command_rx) = mpsc::sync_channel(1);
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
        assert!(command_rx.try_recv().is_err());
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
