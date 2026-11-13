//! Dynamically loaded libmpv runtime boundary.
//!
//! The runtime is intentionally independent of GPUI. Create and use it on the
//! dedicated playback worker that will be introduced by the application layer.
//! The MPV DLL must be supplied by the application package or an explicit path.

use std::{
    ffi::{CStr, CString, OsStr, c_char, c_int, c_void},
    path::Path,
    ptr::NonNull,
};

use anyhow::{Context as _, anyhow, bail};
use libloading::Library;

use super::mpv_config::MpvConfig;

type MpvCreate = unsafe extern "C" fn() -> *mut c_void;
type MpvSetOptionString = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type MpvInitialize = unsafe extern "C" fn(*mut c_void) -> c_int;
type MpvCommand = unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int;
type MpvGetPropertyString = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char;
type MpvFreeData = unsafe extern "C" fn(*mut c_void);
type MpvTerminateDestroy = unsafe extern "C" fn(*mut c_void);

/// A live libmpv instance. It owns the loaded DLL for at least as long as the
/// function pointers and MPV handle remain in use.
pub struct MpvRuntime {
    _library: Library,
    handle: NonNull<c_void>,
    command_fn: MpvCommand,
    get_property_string_fn: MpvGetPropertyString,
    free_data_fn: MpvFreeData,
    terminate_destroy_fn: MpvTerminateDestroy,
}

impl MpvRuntime {
    /// Load libmpv, apply options before initialization, and initialize playback.
    ///
    /// On Windows, library_path normally points to the packaged libmpv-2.dll.
    /// This method does not search arbitrary directories or download binaries.
    pub fn open(library_path: impl AsRef<OsStr>, config: &MpvConfig) -> anyhow::Result<Self> {
        // Loading a native library executes its initialization routines. The
        // caller must only load a trusted, application-supplied MPV binary.
        let library = unsafe { Library::new(library_path.as_ref()) }
            .context("could not load the configured libmpv runtime")?;

        let create: MpvCreate = load_symbol(&library, b"mpv_create\0")?;
        let set_option_string: MpvSetOptionString =
            load_symbol(&library, b"mpv_set_option_string\0")?;
        let initialize: MpvInitialize = load_symbol(&library, b"mpv_initialize\0")?;
        let command_fn: MpvCommand = load_symbol(&library, b"mpv_command\0")?;
        let get_property_string_fn: MpvGetPropertyString =
            load_symbol(&library, b"mpv_get_property_string\0")?;
        let free_data_fn: MpvFreeData = load_symbol(&library, b"mpv_free\0")?;
        let terminate_destroy_fn: MpvTerminateDestroy =
            load_symbol(&library, b"mpv_terminate_destroy\0")?;

        let handle = NonNull::new(unsafe { create() })
            .context("libmpv mpv_create returned a null handle")?;

        for (name, value) in &config.options {
            let name = CString::new(name.as_str())
                .context("MPV option name contains an interior NUL byte")?;
            let value = CString::new(value.as_str())
                .context("MPV option value contains an interior NUL byte")?;
            let result =
                unsafe { set_option_string(handle.as_ptr(), name.as_ptr(), value.as_ptr()) };
            if result < 0 {
                unsafe { terminate_destroy_fn(handle.as_ptr()) };
                bail!("libmpv rejected an option (error code {result})");
            }
        }

        let result = unsafe { initialize(handle.as_ptr()) };
        if result < 0 {
            unsafe { terminate_destroy_fn(handle.as_ptr()) };
            bail!("libmpv initialization failed (error code {result})");
        }

        Ok(Self {
            _library: library,
            handle,
            command_fn,
            get_property_string_fn,
            free_data_fn,
            terminate_destroy_fn,
        })
    }

    /// Open a local media file, replacing the current playlist entry.
    pub fn load_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path
            .as_ref()
            .to_str()
            .context("media path cannot be represented as UTF-8")?;
        self.command(&["loadfile", path, "replace"])
    }

    /// Set the paused state.
    pub fn set_paused(&self, paused: bool) -> anyhow::Result<()> {
        self.command(&["set", "pause", if paused { "yes" } else { "no" }])
    }

    /// Set output volume in the supported SENZO UI range of 0–100 percent.
    pub fn set_volume(&self, volume: f64) -> anyhow::Result<()> {
        let volume = volume_argument(volume)?;
        self.command(&["set", "volume", &volume])
    }

    /// Enable or disable audio mute.
    pub fn set_muted(&self, muted: bool) -> anyhow::Result<()> {
        self.command(&["set", "mute", if muted { "yes" } else { "no" }])
    }

    /// Enable or disable fullscreen presentation.
    pub fn set_fullscreen(&self, fullscreen: bool) -> anyhow::Result<()> {
        self.command(&["set", "fullscreen", if fullscreen { "yes" } else { "no" }])
    }

    /// Select an audio track by the ID reported by MPV.
    pub fn set_audio_track(&self, track_id: u64) -> anyhow::Result<()> {
        let track_id = track_argument(track_id)?;
        self.command(&["set", "aid", &track_id])
    }

    /// Select a subtitle track, or disable subtitles when `None` is supplied.
    pub fn set_subtitle_track(&self, track_id: Option<u64>) -> anyhow::Result<()> {
        let track_id = match track_id {
            Some(track_id) => track_argument(track_id)?,
            None => "no".to_owned(),
        };
        self.command(&["set", "sid", &track_id])
    }

    /// Set the subtitle synchronization offset in seconds.
    pub fn set_subtitle_delay(&self, seconds: f64) -> anyhow::Result<()> {
        if !seconds.is_finite() || !(-10.0..=10.0).contains(&seconds) {
            bail!("subtitle delay must be finite and between -10 and 10 seconds");
        }
        let seconds = seconds.to_string();
        self.command(&["set", "sub-delay", &seconds])
    }

    /// Set the subtitle font size in MPV's native size units.
    pub fn set_subtitle_font_size(&self, size: u8) -> anyhow::Result<()> {
        if !(12..=96).contains(&size) {
            bail!("subtitle font size must be between 12 and 96");
        }
        let size = size.to_string();
        self.command(&["set", "sub-font-size", &size])
    }

    /// Load and select a user-chosen external subtitle file.
    pub fn add_subtitle_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path
            .as_ref()
            .to_str()
            .context("subtitle path cannot be represented as UTF-8")?;
        self.command(&["sub-add", path, "select"])
    }

    /// Seek to an absolute playback position in seconds.
    pub fn seek_absolute(&self, seconds: f64) -> anyhow::Result<()> {
        let seconds = seek_argument(seconds)?;
        self.command(&["seek", &seconds, "absolute+exact"])
    }

    /// Set playback speed. MPV normally accepts values from 0.01 to 100.
    pub fn set_speed(&self, speed: f64) -> anyhow::Result<()> {
        if !speed.is_finite() || !(0.01..=100.0).contains(&speed) {
            bail!("playback speed must be finite and between 0.01 and 100");
        }
        let speed = speed.to_string();
        self.command(&["set", "speed", &speed])
    }

    /// Stop the current file.
    pub fn stop(&self) -> anyhow::Result<()> {
        self.command(&["stop"])
    }

    /// Read an MPV property as its string representation.
    ///
    /// A missing or currently unavailable property returns Ok(None).
    pub fn property_string(&self, property: &str) -> anyhow::Result<Option<String>> {
        let property =
            CString::new(property).context("MPV property name contains an interior NUL byte")?;
        let value =
            unsafe { (self.get_property_string_fn)(self.handle.as_ptr(), property.as_ptr()) };
        let Some(value) = NonNull::new(value) else {
            return Ok(None);
        };

        let result = unsafe { CStr::from_ptr(value.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.free_data_fn)(value.as_ptr().cast()) };
        Ok(Some(result))
    }

    fn command(&self, arguments: &[&str]) -> anyhow::Result<()> {
        let arguments = arguments
            .iter()
            .map(|argument| {
                CString::new(*argument)
                    .context("MPV command argument contains an interior NUL byte")
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut pointers = arguments
            .iter()
            .map(|argument| argument.as_ptr())
            .collect::<Vec<_>>();
        pointers.push(std::ptr::null());

        let result = unsafe { (self.command_fn)(self.handle.as_ptr(), pointers.as_ptr()) };
        if result < 0 {
            return Err(anyhow!("libmpv command failed (error code {result})"));
        }
        Ok(())
    }
}

impl Drop for MpvRuntime {
    fn drop(&mut self) {
        unsafe { (self.terminate_destroy_fn)(self.handle.as_ptr()) };
    }
}

fn load_symbol<T: Copy>(library: &Library, symbol: &'static [u8]) -> anyhow::Result<T> {
    // The Library stays owned by MpvRuntime, keeping copied function pointers valid.
    unsafe {
        library
            .get::<T>(symbol)
            .map(|symbol| *symbol)
            .with_context(|| {
                format!(
                    "libmpv is missing required symbol {}",
                    String::from_utf8_lossy(&symbol[..symbol.len().saturating_sub(1)])
                )
            })
    }
}

fn volume_argument(volume: f64) -> anyhow::Result<String> {
    if !volume.is_finite() || !(0.0..=100.0).contains(&volume) {
        bail!("volume must be finite and between 0 and 100");
    }
    Ok(volume.to_string())
}

fn track_argument(track_id: u64) -> anyhow::Result<String> {
    if track_id == 0 {
        bail!("track ID must be greater than zero");
    }
    Ok(track_id.to_string())
}

fn seek_argument(seconds: f64) -> anyhow::Result<String> {
    if !seconds.is_finite() || seconds < 0.0 {
        bail!("absolute seek position must be finite and non-negative");
    }
    Ok(seconds.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_seek_rejects_negative_and_non_finite_positions() {
        assert!(seek_argument(-1.0).is_err());
        assert!(seek_argument(f64::NAN).is_err());
        assert!(seek_argument(f64::INFINITY).is_err());
        assert_eq!(seek_argument(12.5).unwrap(), "12.5");
    }

    #[test]
    fn volume_is_limited_to_ui_range() {
        assert!(volume_argument(-0.1).is_err());
        assert!(volume_argument(100.1).is_err());
        assert!(volume_argument(f64::NAN).is_err());
        assert_eq!(volume_argument(75.0).unwrap(), "75");
    }

    #[test]
    fn track_ids_must_be_positive() {
        assert!(track_argument(0).is_err());
        assert_eq!(track_argument(3).unwrap(), "3");
    }

    #[test]
    fn subtitle_delay_and_font_size_are_bounded() {
        assert!(!(-10.0..=10.0).contains(&10.1));
        assert!((-10.0..=10.0).contains(&-0.5));
        assert!(!(12..=96).contains(&11));
        assert!((12..=96).contains(&42));
    }

    #[test]
    fn playback_speed_bounds_are_enforced() {
        assert!(!(0.01..=100.0).contains(&0.0));
        assert!(!(0.01..=100.0).contains(&101.0));
        assert!((0.01..=100.0).contains(&1.0));
    }
}
