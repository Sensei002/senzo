//! Stable domain types and service contracts shared by SENZO crates.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Shared active-profile identifier. Managers and providers keep a clone of this
/// handle so switching profiles changes every profile-scoped lookup atomically.
#[derive(Clone)]
pub struct ProfileScope(std::sync::Arc<std::sync::RwLock<String>>);

impl ProfileScope {
    pub fn new(profile_id: impl Into<String>) -> Self {
        Self(std::sync::Arc::new(std::sync::RwLock::new(
            profile_id.into(),
        )))
    }

    pub fn current(&self) -> String {
        self.0
            .read()
            .map(|value| value.clone())
            .unwrap_or_else(|_| "default".to_owned())
    }

    pub fn set(&self, profile_id: impl Into<String>) {
        if let Ok(mut current) = self.0.write() {
            *current = profile_id.into();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SettingKey(String);

impl SettingKey {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 128
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
            && !value.starts_with('.')
            && !value.ends_with('.');
        if !valid {
            return Err(DomainError::InvalidSettingKey);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MediaId(String);

impl MediaId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(DomainError::InvalidMediaId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Movie,
    Series,
    Episode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaReference {
    pub id: String,
    pub kind: MediaKind,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppPage {
    Home,
    Movies,
    TvShows,
    Search,
    Library,
    Addons,
    Settings,
}

impl AppPage {
    pub const ALL: [Self; 7] = [
        Self::Home,
        Self::Movies,
        Self::TvShows,
        Self::Search,
        Self::Library,
        Self::Addons,
        Self::Settings,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Movies => "Movies",
            Self::TvShows => "TV Shows",
            Self::Search => "Search",
            Self::Library => "Library",
            Self::Addons => "Add-ons",
            Self::Settings => "Settings",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Home => "HM",
            Self::Movies => "MV",
            Self::TvShows => "TV",
            Self::Search => "SR",
            Self::Library => "LB",
            Self::Addons => "AD",
            Self::Settings => "ST",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Home => "Your entertainment, brought together.",
            Self::Movies => "Explore movie discovery and curated collections.",
            Self::TvShows => "Browse series, seasons and episodes.",
            Self::Search => "Find movies and television across your providers.",
            Self::Library => "Your saved titles and personal collections.",
            Self::Addons => "Manage the providers you choose to install.",
            Self::Settings => "Make SENZO feel right for you.",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Movies => "movies",
            Self::TvShows => "tv-shows",
            Self::Search => "search",
            Self::Library => "library",
            Self::Addons => "addons",
            Self::Settings => "settings",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("setting key must use lowercase ASCII letters, digits, dots, underscores or hyphens")]
    InvalidSettingKey,
    #[error("media identifier is empty, too long or contains control characters")]
    InvalidMediaId,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("repository operation failed: {0}")]
    Unavailable(String),
    #[error("profile name must contain between 1 and 64 non-control characters")]
    InvalidProfileName,
    #[error("library item or collection data is invalid")]
    InvalidLibraryEntry,
    #[error("profile PIN must contain between 4 and 12 ASCII digits")]
    InvalidProfilePin,
    #[error("profile PIN is temporarily locked")]
    ProfilePinLocked,
    #[error("requested library item or collection was not found")]
    NotFound,
    #[error("playback progress identity or values are invalid")]
    InvalidPlaybackProgress,
}

pub trait SettingsRepository: Send + Sync {
    fn get(&self, key: &SettingKey) -> Result<Option<String>, RepositoryError>;
    fn set(&self, key: &SettingKey, value: &str) -> Result<(), RepositoryError>;
}

pub trait ProfileRepository: Send + Sync {
    fn list_profiles(&self) -> Result<Vec<Profile>, RepositoryError>;
    fn create_profile(&self, name: &str) -> Result<Profile, RepositoryError>;
    fn update_profile(&self, _profile_id: &str, _name: &str) -> Result<Profile, RepositoryError> {
        Err(RepositoryError::Unavailable(
            "profile editing is not supported by this repository".to_owned(),
        ))
    }
    fn delete_profile(&self, _profile_id: &str) -> Result<(), RepositoryError> {
        Err(RepositoryError::Unavailable(
            "profile deletion is not supported by this repository".to_owned(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub profile_id: String,
    pub media_id: String,
    pub media_kind: MediaKind,
    pub title: String,
    pub overview: Option<String>,
    pub release_year: Option<u16>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub added_at_ms: i64,
    pub updated_at_ms: i64,
    pub is_favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCollection {
    pub id: String,
    pub profile_id: String,
    pub title: String,
    pub description: Option<String>,
    pub sort_order: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCollectionItem {
    pub collection_id: String,
    pub profile_id: String,
    pub media_id: String,
    pub media_kind: MediaKind,
    pub sort_order: i64,
    pub added_at_ms: i64,
    pub updated_at_ms: i64,
}

pub trait LibraryRepository: Send + Sync {
    fn list_library_items(&self, profile_id: &str) -> Result<Vec<LibraryItem>, RepositoryError>;
    fn get_library_item(
        &self,
        profile_id: &str,
        media_id: &str,
        media_kind: MediaKind,
    ) -> Result<Option<LibraryItem>, RepositoryError>;
    fn save_library_item(&self, item: &LibraryItem) -> Result<(), RepositoryError>;
    fn remove_library_item(
        &self,
        profile_id: &str,
        media_id: &str,
        media_kind: MediaKind,
        removed_at_ms: i64,
    ) -> Result<(), RepositoryError>;
    fn list_library_collections(
        &self,
        profile_id: &str,
    ) -> Result<Vec<LibraryCollection>, RepositoryError>;
    fn create_library_collection(
        &self,
        profile_id: &str,
        title: &str,
        description: Option<&str>,
        created_at_ms: i64,
    ) -> Result<LibraryCollection, RepositoryError>;
    fn update_library_collection(
        &self,
        profile_id: &str,
        collection_id: &str,
        title: &str,
        description: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<LibraryCollection, RepositoryError>;
    fn delete_library_collection(
        &self,
        profile_id: &str,
        collection_id: &str,
        deleted_at_ms: i64,
    ) -> Result<(), RepositoryError>;
    fn list_collection_items(
        &self,
        profile_id: &str,
        collection_id: &str,
    ) -> Result<Vec<LibraryItem>, RepositoryError>;
    fn set_collection_item(
        &self,
        profile_id: &str,
        collection_id: &str,
        media_id: &str,
        media_kind: MediaKind,
        included: bool,
        changed_at_ms: i64,
    ) -> Result<(), RepositoryError>;
    fn reorder_collection_items(
        &self,
        profile_id: &str,
        collection_id: &str,
        ordered: &[(String, MediaKind)],
        changed_at_ms: i64,
    ) -> Result<(), RepositoryError>;
}

pub trait ProfileSettingsRepository: Send + Sync {
    fn get_profile_setting(
        &self,
        profile_id: &str,
        key: &SettingKey,
    ) -> Result<Option<String>, RepositoryError>;
    fn set_profile_setting(
        &self,
        profile_id: &str,
        key: &SettingKey,
        value: &str,
    ) -> Result<(), RepositoryError>;
}

pub trait ProfileSecurityRepository: Send + Sync {
    fn has_profile_pin(&self, profile_id: &str) -> Result<bool, RepositoryError>;
    fn set_profile_pin(&self, profile_id: &str, pin: &str) -> Result<(), RepositoryError>;
    fn verify_profile_pin(
        &self,
        profile_id: &str,
        pin: &str,
        now_ms: i64,
    ) -> Result<bool, RepositoryError>;
    fn clear_profile_pin(&self, profile_id: &str) -> Result<(), RepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMutation {
    pub id: String,
    pub profile_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub payload_json: String,
    pub changed_at_ms: i64,
    pub attempt_count: u32,
    pub next_attempt_at_ms: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSyncMutation {
    pub cursor: i64,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub payload_json: String,
    pub changed_at_ms: i64,
}

pub trait SyncOutboxRepository: Send + Sync {
    fn apply_remote_sync_page(
        &self,
        profile_id: &str,
        provider: &str,
        next_cursor: &str,
        mutations: &[RemoteSyncMutation],
        updated_at_ms: i64,
    ) -> Result<(), RepositoryError>;
    fn list_pending_sync_mutations(
        &self,
        profile_id: &str,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<SyncMutation>, RepositoryError>;
    fn mark_sync_mutations_applied(
        &self,
        profile_id: &str,
        mutation_ids: &[String],
    ) -> Result<(), RepositoryError>;
    fn record_sync_mutation_failure(
        &self,
        profile_id: &str,
        mutation_id: &str,
        error: &str,
        next_attempt_at_ms: i64,
    ) -> Result<(), RepositoryError>;
    fn get_sync_checkpoint(
        &self,
        profile_id: &str,
        provider: &str,
    ) -> Result<Option<String>, RepositoryError>;
    fn set_sync_checkpoint(
        &self,
        profile_id: &str,
        provider: &str,
        cursor: &str,
        updated_at_ms: i64,
    ) -> Result<(), RepositoryError>;
}

/// A local playback checkpoint. Series progress is keyed by parent series and
/// season/episode; movies use neither season nor episode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackProgress {
    pub profile_id: String,
    pub media_id: String,
    pub media_kind: MediaKind,
    pub title: String,
    pub season_number: Option<u32>,
    pub episode_number: Option<u32>,
    pub episode_title: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub still_path: Option<String>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub is_completed: bool,
    pub updated_at_ms: i64,
}

/// An explicit watched marker for a movie or a specific TV episode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedItem {
    pub profile_id: String,
    pub media_id: String,
    pub media_kind: MediaKind,
    pub season_number: Option<u32>,
    pub episode_number: Option<u32>,
    pub marked_at_ms: i64,
}

/// Matches Nuvio Desktop's playback-progress policy. Durations shorter than
/// 121 seconds are treated as placeholder/error clips, not real content.
pub const MIN_REAL_MEDIA_DURATION_MS: u64 = 121_000;
pub const MIN_PROGRESS_POSITION_MS: u64 = 1_000;
pub const WATCHED_COMPLETION_FRACTION: f64 = 0.90;

pub fn should_store_playback_progress(position_ms: u64, duration_ms: u64) -> bool {
    position_ms >= MIN_PROGRESS_POSITION_MS
        && !(duration_ms > 0 && duration_ms < MIN_REAL_MEDIA_DURATION_MS)
}

pub fn playback_is_complete(position_ms: u64, duration_ms: u64, is_ended: bool) -> bool {
    if duration_ms > 0 && duration_ms < MIN_REAL_MEDIA_DURATION_MS {
        return false;
    }
    if is_ended {
        return true;
    }
    duration_ms > 0 && (position_ms as f64 / duration_ms as f64) >= WATCHED_COMPLETION_FRACTION
}

pub trait PlaybackProgressRepository: Send + Sync {
    fn get_playback_progress(
        &self,
        profile_id: &str,
        media_id: &str,
        media_kind: MediaKind,
        season_number: Option<u32>,
        episode_number: Option<u32>,
    ) -> Result<Option<PlaybackProgress>, RepositoryError>;

    fn save_playback_progress(&self, progress: &PlaybackProgress) -> Result<(), RepositoryError>;

    fn list_continue_watching(
        &self,
        profile_id: &str,
        limit: usize,
    ) -> Result<Vec<PlaybackProgress>, RepositoryError>;

    /// Most-recent completed TV episode per series, excluding series with
    /// a newer in-progress checkpoint. Used to project the Up Next shelf.
    fn list_recent_completed_series(
        &self,
        profile_id: &str,
        limit: usize,
    ) -> Result<Vec<PlaybackProgress>, RepositoryError>;

    fn is_watched(&self, item: &WatchedItem) -> Result<bool, RepositoryError>;

    fn set_watched(&self, item: &WatchedItem, watched: bool) -> Result<(), RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_scope_updates_all_clones_and_keeps_profiles_independent() {
        let scope = ProfileScope::new("default");
        let other_view = scope.clone();
        assert_eq!(scope.current(), "default");
        other_view.set("guest");
        assert_eq!(scope.current(), "guest");
        scope.set("family");
        assert_eq!(other_view.current(), "family");
        let independent = ProfileScope::new("default");
        assert_eq!(independent.current(), "default");
    }

    #[test]
    fn setting_keys_are_validated() {
        assert_eq!(
            SettingKey::new("player.audio_language").unwrap().as_str(),
            "player.audio_language"
        );
        assert!(SettingKey::new("../secret").is_err());
        assert!(SettingKey::new("Uppercase").is_err());
    }

    #[test]
    fn media_ids_reject_control_characters() {
        assert!(MediaId::new("tmdb:123").is_ok());
        assert!(MediaId::new("bad\nvalue").is_err());
    }

    #[test]
    fn playback_progress_policy_ignores_placeholders_and_marks_at_ninety_percent() {
        assert!(!should_store_playback_progress(30_000, 120_000));
        assert!(should_store_playback_progress(1_000, 121_000));
        assert!(!playback_is_complete(119_000, 120_000, true));
        assert!(!playback_is_complete(890_000, 1_000_000, false));
        assert!(playback_is_complete(900_000, 1_000_000, false));
        assert!(playback_is_complete(1_000, 0, true));
    }

    #[test]
    fn primary_pages_have_stable_titles_and_keys() {
        assert_eq!(AppPage::ALL.len(), 7);
        assert_eq!(AppPage::TvShows.title(), "TV Shows");
        assert_eq!(AppPage::Addons.key(), "addons");
        assert_eq!(AppPage::Settings.short_label(), "ST");
    }
}
