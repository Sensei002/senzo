use serde::{Deserialize, Serialize};

/// User-controlled playback and picture-processing preferences.
///
/// These values describe intent, not guaranteed hardware capabilities. The playback
/// backend must validate each option against the active MPV build, GPU, display, and
/// media before applying it. Unsupported options must fall back safely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackSettings {
    pub hardware_decoding: HardwareDecoding,
    pub decoder: DecoderPreference,
    pub renderer: RendererPreference,
    pub upscale: ScalingAlgorithm,
    pub downscale: ScalingAlgorithm,
    pub deband: bool,
    pub interpolation: bool,
    pub hdr: HdrMode,
    pub buffer: BufferSettings,
    pub subtitles: SubtitleSettings,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            hardware_decoding: HardwareDecoding::Auto,
            decoder: DecoderPreference::Auto,
            renderer: RendererPreference::Auto,
            upscale: ScalingAlgorithm::Lanczos,
            downscale: ScalingAlgorithm::Mitchell,
            deband: false,
            interpolation: false,
            hdr: HdrMode::Auto,
            buffer: BufferSettings::default(),
            subtitles: SubtitleSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HardwareDecoding {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DecoderPreference {
    #[default]
    Auto,
    D3d11va,
    Dxva2,
    Software,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RendererPreference {
    #[default]
    Auto,
    GpuNext,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScalingAlgorithm {
    Bilinear,
    Bicubic,
    #[default]
    Lanczos,
    Spline36,
    Mitchell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HdrMode {
    #[default]
    Auto,
    ToneMap,
    Passthrough,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BufferSettings {
    /// Target forward buffer in seconds. Zero delegates buffering to the backend.
    pub forward_seconds: u16,
    /// Amount of already-played media retained for backward seeking.
    pub back_seconds: u16,
    /// Soft memory budget in MiB for playback buffering.
    pub memory_limit_mib: u16,
}

impl Default for BufferSettings {
    fn default() -> Self {
        Self {
            forward_seconds: 30,
            back_seconds: 15,
            memory_limit_mib: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubtitleSettings {
    pub font_size: u8,
    pub vertical_margin: u8,
    pub bold: bool,
}

impl Default for SubtitleSettings {
    fn default() -> Self {
        Self {
            font_size: 42,
            vertical_margin: 24,
            bold: false,
        }
    }
}

impl PlaybackSettings {
    /// Normalize persisted/user-provided values to conservative resource limits.
    pub fn normalize(&mut self) {
        self.buffer.forward_seconds = self.buffer.forward_seconds.min(300);
        self.buffer.back_seconds = self.buffer.back_seconds.min(300);
        self.buffer.memory_limit_mib = self.buffer.memory_limit_mib.clamp(32, 2048);
        self.subtitles.font_size = self.subtitles.font_size.clamp(12, 96);
        self.subtitles.vertical_margin = self.subtitles.vertical_margin.min(100);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_prefer_automatic_hardware_and_lightweight_processing() {
        let settings = PlaybackSettings::default();
        assert_eq!(settings.hardware_decoding, HardwareDecoding::Auto);
        assert_eq!(settings.decoder, DecoderPreference::Auto);
        assert_eq!(settings.renderer, RendererPreference::Auto);
        assert!(!settings.deband);
        assert!(!settings.interpolation);
        assert_eq!(settings.hdr, HdrMode::Auto);
    }

    #[test]
    fn normalize_enforces_safe_resource_and_display_limits() {
        let mut settings = PlaybackSettings::default();
        settings.buffer.forward_seconds = u16::MAX;
        settings.buffer.back_seconds = u16::MAX;
        settings.buffer.memory_limit_mib = 1;
        settings.subtitles.font_size = 1;
        settings.subtitles.vertical_margin = u8::MAX;

        settings.normalize();

        assert_eq!(settings.buffer.forward_seconds, 300);
        assert_eq!(settings.buffer.back_seconds, 300);
        assert_eq!(settings.buffer.memory_limit_mib, 32);
        assert_eq!(settings.subtitles.font_size, 12);
        assert_eq!(settings.subtitles.vertical_margin, 100);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let settings = PlaybackSettings {
            hardware_decoding: HardwareDecoding::Enabled,
            decoder: DecoderPreference::D3d11va,
            deband: true,
            ..PlaybackSettings::default()
        };
        let json = serde_json::to_string(&settings).expect("serialize settings");
        let restored: PlaybackSettings = serde_json::from_str(&json).expect("deserialize settings");
        assert_eq!(restored, settings);
    }

    #[test]
    fn older_or_partial_settings_receive_defaults() {
        let restored: PlaybackSettings = serde_json::from_str(r#"{"interpolation":true}"#)
            .expect("deserialize partial settings");
        assert!(restored.interpolation);
        assert_eq!(restored.hardware_decoding, HardwareDecoding::Auto);
        assert_eq!(restored.buffer, BufferSettings::default());
    }
}
