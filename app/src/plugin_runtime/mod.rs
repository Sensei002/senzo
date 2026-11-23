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
}

impl PluginStream {
    pub fn is_torrent(&self) -> bool {
        self.info_hash.is_some()
    }
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
