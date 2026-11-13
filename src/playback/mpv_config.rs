use std::collections::BTreeMap;

use crate::settings::{
    DecoderPreference, HardwareDecoding, HdrMode, PlaybackSettings, RendererPreference,
    ScalingAlgorithm,
};

/// Capabilities reported by the active MPV build and output device.
/// Unknown capabilities must remain false; MPV's own automatic defaults remain available.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MpvCapabilities {
    pub gpu_available: bool,
    pub gpu_next_available: bool,
    pub d3d11va_available: bool,
    pub dxva2_available: bool,
    pub hdr_output_supported: bool,
    pub hdr_tone_mapping_supported: bool,
}

/// A user preference that could not be applied exactly in the current runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFallback {
    pub setting: &'static str,
    pub requested: String,
    pub applied: String,
    pub reason: &'static str,
}

/// String-valued options ready to pass to MPV's option/property API.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MpvConfig {
    pub options: BTreeMap<String, String>,
    pub fallbacks: Vec<ConfigFallback>,
}

impl MpvConfig {
    fn set(&mut self, key: &str, value: impl ToString) {
        self.options.insert(key.to_owned(), value.to_string());
    }

    fn fallback(
        &mut self,
        setting: &'static str,
        requested: impl ToString,
        applied: impl ToString,
        reason: &'static str,
    ) {
        self.fallbacks.push(ConfigFallback {
            setting,
            requested: requested.to_string(),
            applied: applied.to_string(),
            reason,
        });
    }
}

/// Translate persisted preferences into conservative MPV options.
///
/// This function does not load MPV or claim that an option was accepted. The caller
/// must apply the returned options to the live MPV instance and handle API errors.
pub fn build_mpv_config(settings: &PlaybackSettings, capabilities: &MpvCapabilities) -> MpvConfig {
    let mut config = MpvConfig::default();

    apply_hardware_decoding(&mut config, settings, capabilities);
    apply_renderer(&mut config, settings.renderer, capabilities);
    apply_picture_quality(&mut config, settings, capabilities);
    apply_buffering(&mut config, settings);
    apply_subtitles(&mut config, settings);

    config
}

fn apply_hardware_decoding(
    config: &mut MpvConfig,
    settings: &PlaybackSettings,
    capabilities: &MpvCapabilities,
) {
    let requested = settings.decoder;
    let hwdec = match (settings.hardware_decoding, requested) {
        (HardwareDecoding::Disabled, _) | (_, DecoderPreference::Software) => "no",
        (HardwareDecoding::Auto, DecoderPreference::Auto) => "auto-safe",
        (HardwareDecoding::Enabled, DecoderPreference::Auto) => "auto",
        (_, DecoderPreference::D3d11va) if capabilities.d3d11va_available => "d3d11va",
        (_, DecoderPreference::Dxva2) if capabilities.dxva2_available => "dxva2",
        (_, DecoderPreference::D3d11va | DecoderPreference::Dxva2) => {
            config.fallback(
                "decoder",
                format!("{requested:?}"),
                "auto",
                "The requested Windows hardware decoder is not available in this MPV runtime.",
            );
            "auto"
        }
    };
    config.set("hwdec", hwdec);
}

fn apply_renderer(
    config: &mut MpvConfig,
    preference: RendererPreference,
    capabilities: &MpvCapabilities,
) {
    let selected = match preference {
        RendererPreference::Auto if capabilities.gpu_next_available => Some("gpu-next"),
        RendererPreference::Auto if capabilities.gpu_available => Some("gpu"),
        RendererPreference::Auto => None,
        RendererPreference::GpuNext if capabilities.gpu_next_available => Some("gpu-next"),
        RendererPreference::Gpu if capabilities.gpu_available => Some("gpu"),
        RendererPreference::GpuNext if capabilities.gpu_available => {
            config.fallback(
                "renderer",
                "gpu-next",
                "gpu",
                "GPU Next is unavailable; the compatible GPU renderer is available.",
            );
            Some("gpu")
        }
        RendererPreference::GpuNext | RendererPreference::Gpu => {
            config.fallback(
                "renderer",
                format!("{preference:?}"),
                "MPV default",
                "The requested video output is not reported by this runtime; MPV will choose its default.",
            );
            None
        }
    };

    if let Some(renderer) = selected {
        config.set("vo", renderer);
    }
}

fn scaling_name(algorithm: ScalingAlgorithm) -> &'static str {
    match algorithm {
        ScalingAlgorithm::Bilinear => "bilinear",
        ScalingAlgorithm::Bicubic => "bicubic",
        ScalingAlgorithm::Lanczos => "lanczos",
        ScalingAlgorithm::Spline36 => "spline36",
        ScalingAlgorithm::Mitchell => "mitchell",
    }
}

fn apply_picture_quality(
    config: &mut MpvConfig,
    settings: &PlaybackSettings,
    capabilities: &MpvCapabilities,
) {
    config.set("scale", scaling_name(settings.upscale));
    config.set("dscale", scaling_name(settings.downscale));
    config.set("deband", if settings.deband { "yes" } else { "no" });
    config.set(
        "interpolation",
        if settings.interpolation { "yes" } else { "no" },
    );

    match settings.hdr {
        HdrMode::Auto => {}
        HdrMode::ToneMap if capabilities.hdr_tone_mapping_supported => {
            config.set("tone-mapping", "auto");
        }
        HdrMode::Passthrough
            if capabilities.hdr_output_supported && capabilities.gpu_next_available =>
        {
            config.set("target-colorspace-hint", "yes");
        }
        HdrMode::ToneMap => config.fallback(
            "hdr",
            "tone_map",
            "auto",
            "HDR tone mapping is not reported as supported; MPV's automatic output handling is retained.",
        ),
        HdrMode::Passthrough => config.fallback(
            "hdr",
            "passthrough",
            "auto",
            "HDR passthrough requires a reported HDR output and the GPU Next renderer; MPV's automatic handling is retained.",
        ),
        HdrMode::Disabled if capabilities.gpu_next_available => {
            config.set("target-trc", "bt.1886");
            config.set("target-colorspace-hint", "yes");
        }
        HdrMode::Disabled => config.fallback(
            "hdr",
            "disabled",
            "auto",
            "Explicit SDR tone mapping requires the GPU Next renderer; MPV's automatic handling is retained.",
        ),
    }
}

fn apply_buffering(config: &mut MpvConfig, settings: &PlaybackSettings) {
    let forward_seconds = settings.buffer.forward_seconds.min(300);
    let back_seconds = settings.buffer.back_seconds.min(300);
    let memory_bytes = u64::from(settings.buffer.memory_limit_mib.clamp(32, 2048)) * 1024 * 1024;
    let total_seconds = u64::from(forward_seconds) + u64::from(back_seconds);
    let back_bytes = if total_seconds == 0 {
        0
    } else {
        memory_bytes * u64::from(back_seconds) / total_seconds
    };
    let forward_bytes = memory_bytes - back_bytes;

    config.set("cache", "yes");
    config.set("cache-secs", forward_seconds);
    config.set("demuxer-max-bytes", forward_bytes);
    config.set("demuxer-max-back-bytes", back_bytes);
    config.set("demuxer-readahead-secs", forward_seconds);
}

fn apply_subtitles(config: &mut MpvConfig, settings: &PlaybackSettings) {
    config.set("sub-font-size", settings.subtitles.font_size.clamp(12, 96));
    config.set("sub-margin-y", settings.subtitles.vertical_margin.min(100));
    config.set(
        "sub-delay",
        settings.subtitles.delay_seconds.clamp(-10.0, 10.0),
    );
    config.set(
        "sub-bold",
        if settings.subtitles.bold { "yes" } else { "no" },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{BufferSettings, SubtitleSettings};

    fn full_capabilities() -> MpvCapabilities {
        MpvCapabilities {
            gpu_available: true,
            gpu_next_available: true,
            d3d11va_available: true,
            dxva2_available: true,
            hdr_output_supported: true,
            hdr_tone_mapping_supported: true,
        }
    }

    #[test]
    fn default_preferences_use_automatic_decoding_and_available_renderer() {
        let config = build_mpv_config(&PlaybackSettings::default(), &full_capabilities());
        assert_eq!(
            config.options.get("hwdec").map(String::as_str),
            Some("auto-safe")
        );
        assert_eq!(
            config.options.get("vo").map(String::as_str),
            Some("gpu-next")
        );
        assert!(config.fallbacks.is_empty());
    }

    #[test]
    fn disabled_hardware_decoding_always_selects_software() {
        let settings = PlaybackSettings {
            hardware_decoding: HardwareDecoding::Disabled,
            decoder: DecoderPreference::D3d11va,
            ..PlaybackSettings::default()
        };
        let config = build_mpv_config(&settings, &full_capabilities());
        assert_eq!(config.options.get("hwdec").map(String::as_str), Some("no"));
    }

    #[test]
    fn unavailable_explicit_decoder_falls_back_to_auto_with_reason() {
        let settings = PlaybackSettings {
            hardware_decoding: HardwareDecoding::Enabled,
            decoder: DecoderPreference::D3d11va,
            ..PlaybackSettings::default()
        };
        let config = build_mpv_config(&settings, &MpvCapabilities::default());
        assert_eq!(
            config.options.get("hwdec").map(String::as_str),
            Some("auto")
        );
        assert_eq!(config.fallbacks.len(), 1);
        assert_eq!(config.fallbacks[0].setting, "decoder");
    }

    #[test]
    fn unavailable_gpu_next_uses_gpu_renderer_when_possible() {
        let settings = PlaybackSettings {
            renderer: RendererPreference::GpuNext,
            ..PlaybackSettings::default()
        };
        let capabilities = MpvCapabilities {
            gpu_available: true,
            ..MpvCapabilities::default()
        };
        let config = build_mpv_config(&settings, &capabilities);
        assert_eq!(config.options.get("vo").map(String::as_str), Some("gpu"));
        assert_eq!(config.fallbacks[0].applied, "gpu");
    }

    #[test]
    fn hdr_requests_fall_back_when_output_capabilities_are_missing() {
        let settings = PlaybackSettings {
            hdr: HdrMode::Passthrough,
            ..PlaybackSettings::default()
        };
        let config = build_mpv_config(&settings, &MpvCapabilities::default());
        assert!(!config.options.contains_key("target-colorspace-hint"));
        assert_eq!(config.fallbacks[0].setting, "hdr");
    }

    #[test]
    fn image_quality_and_subtitle_preferences_map_to_mpv_options() {
        let settings = PlaybackSettings {
            upscale: ScalingAlgorithm::Spline36,
            downscale: ScalingAlgorithm::Mitchell,
            deband: true,
            interpolation: true,
            subtitles: SubtitleSettings {
                font_size: 52,
                vertical_margin: 18,
                bold: true,
            },
            ..PlaybackSettings::default()
        };
        let config = build_mpv_config(&settings, &full_capabilities());
        assert_eq!(
            config.options.get("scale").map(String::as_str),
            Some("spline36")
        );
        assert_eq!(
            config.options.get("dscale").map(String::as_str),
            Some("mitchell")
        );
        assert_eq!(
            config.options.get("deband").map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            config.options.get("interpolation").map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            config.options.get("sub-font-size").map(String::as_str),
            Some("52")
        );
        assert_eq!(
            config.options.get("sub-margin-y").map(String::as_str),
            Some("18")
        );
        assert_eq!(
            config.options.get("sub-delay").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            config.options.get("sub-bold").map(String::as_str),
            Some("yes")
        );
    }

    #[test]
    fn buffer_limits_are_bounded_and_memory_budget_is_split() {
        let settings = PlaybackSettings {
            buffer: BufferSettings {
                forward_seconds: 600,
                back_seconds: 600,
                memory_limit_mib: 256,
            },
            ..PlaybackSettings::default()
        };
        let config = build_mpv_config(&settings, &full_capabilities());
        let forward = config.options["demuxer-max-bytes"].parse::<u64>().unwrap();
        let back = config.options["demuxer-max-back-bytes"]
            .parse::<u64>()
            .unwrap();
        assert_eq!(config.options["cache-secs"], "300");
        assert_eq!(config.options["demuxer-readahead-secs"], "300");
        assert!(!config.options.contains_key("demuxer-back-buffer"));
        assert_eq!(forward + back, 256 * 1024 * 1024);
        assert_eq!(back, 256 * 1024 * 1024 / 2);

        let defaults = build_mpv_config(&PlaybackSettings::default(), &full_capabilities());
        let default_back = defaults.options["demuxer-max-back-bytes"]
            .parse::<u64>()
            .unwrap();
        assert_eq!(default_back, 256 * 1024 * 1024 / 3);
    }
}
