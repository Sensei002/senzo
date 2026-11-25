//! Shared, serializable types for the Direct Sources subsystem.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The persisted Direct Sources configuration. It is stored inside the normal
/// local `AppConfig`, so it follows the existing profile/settings lifecycle and
/// never requires an account or a cloud service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectSourcesConfig {
    /// Master switch. New installs start enabled, but only vetted built-ins are
    /// enabled inside this default.
    pub enabled: bool,
    /// State is stored per provider so a provider the user disabled is never
    /// silently re-enabled by a later app update.
    pub builtin: Vec<BuiltinSourceState>,
    pub custom: Vec<CustomSourceConfig>,
}

impl Default for DirectSourcesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            builtin: default_builtin_states(),
            custom: Vec::new(),
        }
    }
}

impl DirectSourcesConfig {
    /// Add missing built-ins without changing the enabled state of any
    /// provider that already has a record. Used by the versioned migration and
    /// when reading an older config that predates Direct Sources.
    pub fn ensure_builtin_defaults(&mut self) {
        if self.builtin.is_empty() && self.custom.is_empty() {
            self.builtin = default_builtin_states();
            return;
        }
        for definition in builtin_definitions() {
            if !self.builtin.iter().any(|state| state.id == definition.id) {
                self.builtin.push(BuiltinSourceState {
                    id: definition.id.to_owned(),
                    enabled: definition.enabled_by_default,
                });
            }
        }
        self.builtin
            .retain(|state| builtin_definitions().iter().any(|d| d.id == state.id));
    }

    pub fn builtin_enabled(&self, id: &str) -> bool {
        self.builtin
            .iter()
            .find(|state| state.id == id)
            .is_some_and(|state| state.enabled)
    }

    pub fn set_builtin_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(state) = self.builtin.iter_mut().find(|state| state.id == id) {
            state.enabled = enabled;
            return true;
        }
        false
    }

    pub fn custom_source(&self, id: &str) -> Option<&CustomSourceConfig> {
        self.custom.iter().find(|source| source.id == id)
    }

    pub fn enabled_custom_sources(&self) -> impl Iterator<Item = &CustomSourceConfig> {
        self.custom.iter().filter(|source| source.enabled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BuiltinSourceState {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustomSourceKind {
    /// An authorized M3U/M3U8 playlist.
    M3u,
    /// A small JSON feed in the documented SENZO Direct Sources schema.
    JsonFeed,
}

impl CustomSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::M3u => "M3U playlist",
            Self::JsonFeed => "JSON feed",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::M3u => "m3u",
            Self::JsonFeed => "json",
        }
    }

    pub fn from_short(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "m3u" | "m3u8" | "m3u playlist" => Some(Self::M3u),
            "json" | "json feed" => Some(Self::JsonFeed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomSourceConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    pub kind: CustomSourceKind,
    /// Free-form labels shown in the manager, e.g. `movie`, `series`.
    pub content_types: Vec<String>,
    /// Expected container/stream formats, e.g. `m3u8`, `mp4`, `mkv`.
    pub formats: Vec<String>,
    pub enabled: bool,
}

impl Default for CustomSourceConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            kind: CustomSourceKind::M3u,
            content_types: vec!["movie".to_owned(), "series".to_owned()],
            formats: vec!["m3u8".to_owned(), "mp4".to_owned()],
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BuiltinDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub content_types: &'static [&'static str],
    pub languages: &'static [&'static str],
    pub formats: &'static [&'static str],
    pub configurable: bool,
    pub enabled_by_default: bool,
}

pub const INTERNET_ARCHIVE: &str = "internet-archive";
pub const WIKIMEDIA_COMMONS: &str = "wikimedia-commons";

pub fn builtin_definitions() -> &'static [BuiltinDefinition] {
    &[
        BuiltinDefinition {
            id: INTERNET_ARCHIVE,
            name: "Internet Archive",
            description: "Public-domain and openly licensed films from archive.org. Only items with an explicit open license are offered.",
            content_types: &["movie"],
            languages: &["multiple"],
            formats: &["mp4", "webm"],
            configurable: false,
            enabled_by_default: true,
        },
        BuiltinDefinition {
            id: WIKIMEDIA_COMMONS,
            name: "Wikimedia Commons",
            description: "Freely licensed video from Wikimedia Commons. Files link back to their Commons description page for attribution.",
            content_types: &["movie"],
            languages: &["multiple"],
            formats: &["webm", "ogv"],
            configurable: false,
            enabled_by_default: true,
        },
    ]
}

pub fn default_builtin_states() -> Vec<BuiltinSourceState> {
    builtin_definitions()
        .iter()
        .map(|definition| BuiltinSourceState {
            id: definition.id.to_owned(),
            enabled: definition.enabled_by_default,
        })
        .collect()
}

pub fn builtin_definition(id: &str) -> Option<&'static BuiltinDefinition> {
    builtin_definitions()
        .iter()
        .find(|definition| definition.id == id)
}

/// What the details page is currently asking for. The request keeps the
/// media-kind/season/episode identity intact so providers never have to fall
/// back to title-only matching when a stronger TMDB/IMDb identity is present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectRequest {
    /// IMDb id (`tt...`) when the metadata exposes one.
    pub imdb_id: Option<String>,
    /// The Stremio/TMDB meta id used by the existing core model.
    pub meta_id: String,
    /// `movie` or `series`.
    pub media_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub episode_title: Option<String>,
    pub video_id: Option<String>,
}

impl DirectRequest {
    pub fn cache_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.media_type,
            self.meta_id,
            self.season.unwrap_or_default(),
            self.episode.unwrap_or_default()
        )
    }

    pub fn is_movie(&self) -> bool {
        self.media_type == "movie"
    }
}

/// Provider-independent stream result. URLs are treated as untrusted and are
/// validated again before they are handed to the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectStream {
    pub provider_id: String,
    pub provider_name: String,
    pub title: String,
    pub media_type: String,
    pub imdb_id: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub url: String,
    pub quality: String,
    pub language: String,
    pub video_format: String,
    pub audio: String,
    pub subtitles: Vec<String>,
    /// Authorized headers supplied by the provider (for example a referer).
    /// These are passed to the player and are never logged.
    pub headers: BTreeMap<String, String>,
    pub expires_at: Option<i64>,
    pub metadata: BTreeMap<String, String>,
}

impl DirectStream {
    pub fn new(
        provider_id: &str,
        provider_name: &str,
        request: &DirectRequest,
        url: String,
        format: &str,
    ) -> Self {
        Self {
            provider_id: provider_id.to_owned(),
            provider_name: provider_name.to_owned(),
            title: request.title.clone(),
            media_type: request.media_type.clone(),
            imdb_id: request.imdb_id.clone(),
            season: request.season,
            episode: request.episode,
            url,
            quality: "auto".to_owned(),
            language: "unknown".to_owned(),
            video_format: format.to_owned(),
            audio: "unknown".to_owned(),
            subtitles: Vec::new(),
            headers: BTreeMap::new(),
            expires_at: None,
            metadata: BTreeMap::new(),
        }
    }
}

/// A candidate found by the domain-restricted Discover screen. Candidates are
/// never added automatically; they always require an explicit user action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryCandidate {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub method: String,
    pub content_types: String,
    pub formats: String,
    pub status: String,
    pub required_config: String,
    pub url: String,
    pub kind: Option<CustomSourceKind>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderHealth {
    pub status: String,
    pub checked_at: Option<i64>,
}
