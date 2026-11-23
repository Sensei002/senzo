//! Sandboxed execution of Nuvio-compatible provider scripts.
//!
//! The runtime is intentionally isolated from the rest of SENZO: a provider
//! receives a TMDB id and returns plain stream descriptors. It cannot access
//! the filesystem, the database, the OAuth/debrid stores, or the app UI.

mod crypto;
mod dom;
mod engine;
pub(crate) mod net;

pub use engine::ExecutionOutcome;

/// The single input a provider script receives.
#[derive(Clone, Debug)]
pub struct ProviderQuery {
    pub tmdb_id: String,
    pub media_type: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

/// A stream descriptor projected from a provider result.
#[derive(Clone, Debug)]
pub struct PluginStream {
    pub provider_key: String,
    pub provider_name: String,
    pub name: String,
    pub description: String,
    pub url: Option<String>,
    pub info_hash: Option<String>,
    pub file_idx: Option<u16>,
    pub headers: Vec<(String, String)>,
    pub subtitles: Vec<String>,
    pub seeders: Option<u32>,
}

impl PluginStream {
    pub fn is_torrent(&self) -> bool {
        self.info_hash.is_some()
    }

    /// A provider result is only useful to the drawer and player when it is
    /// either a torrent or an HTTP URL that points at a media object. Link
    /// generators and HTML download pages must not become dead stream rows.
    pub fn is_playable(&self) -> bool {
        if self.is_torrent() {
            return true;
        }
        let Some(url) = self.url.as_deref() else {
            return false;
        };
        let Ok(url) = url::Url::parse(url) else {
            return false;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return false;
        }
        let path = url.path().to_ascii_lowercase();
        if has_media_extension(&path) {
            return true;
        }
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if host.ends_with(".r2.cloudflarestorage.com") || host.ends_with(".r2.dev") {
            return true;
        }
        url.query()
            .is_some_and(|query| has_media_extension(&query.to_ascii_lowercase()))
    }
}

fn has_media_extension(value: &str) -> bool {
    [
        ".mp4", ".mkv", ".webm", ".avi", ".mov", ".m4v", ".ts", ".m3u8", ".mpd",
        ".flv", ".wmv",
    ]
    .iter()
    .any(|extension| value.contains(extension))
}

/// Execute one provider. Call from `spawn_blocking`; the QuickJS runtime is
/// not `Send`, while the provider worker owns it for its lifetime.
pub fn execute(
    provider_key: &str,
    provider_name: &str,
    script: &str,
    settings_json: &str,
    query: &ProviderQuery,
) -> ExecutionOutcome {
    engine::execute(
        provider_key,
        provider_name,
        script,
        settings_json,
        &engine::ProviderRequest {
            tmdb_id: query.tmdb_id.clone(),
            media_type: query.media_type.clone(),
            season: query.season,
            episode: query.episode,
        },
    )
}

/// Load a script without calling `getStreams`. Used by the Plugins test button
/// so an invalid or incompatible script is reported before playback.
pub fn validate(provider_key: &str, script: &str, settings_json: &str) -> Result<(), String> {
    engine::validate(provider_key, script, settings_json)
}

#[cfg(test)]
mod tests {
    use super::PluginStream;

    fn stream(url: Option<&str>, info_hash: Option<&str>) -> PluginStream {
        PluginStream {
            provider_key: "test".to_owned(),
            provider_name: "Test".to_owned(),
            name: "Test".to_owned(),
            description: String::new(),
            url: url.map(str::to_owned),
            info_hash: info_hash.map(str::to_owned),
            file_idx: None,
            headers: Vec::new(),
            subtitles: Vec::new(),
            seeders: None,
        }
    }

    #[test]
    fn only_playable_http_and_torrent_results_are_kept() {
        assert!(stream(Some("https://example.com/movie.mkv"), None).is_playable());
        assert!(stream(
            Some("https://example.com/download?filename=movie.mp4"),
            None
        )
        .is_playable());
        assert!(stream(
            Some("https://bucket.r2.cloudflarestorage.com/hub/abc"),
            None
        )
        .is_playable());
        assert!(stream(
            None,
            Some("0123456789abcdef0123456789abcdef01234567")
        )
        .is_playable());
        assert!(!stream(Some("https://pixel.hubcloud.mov/?id=abc"), None).is_playable());
        assert!(!stream(Some("https://example.com/download-page"), None).is_playable());
    }
}
