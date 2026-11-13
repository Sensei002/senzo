//! Playback backend adapters and capability-aware configuration.

pub mod mpv_config;
pub mod mpv_runtime;
pub mod mpv_worker;
pub use mpv_worker::PlaybackTrack;

#[cfg(windows)]
pub mod windows_surface;
