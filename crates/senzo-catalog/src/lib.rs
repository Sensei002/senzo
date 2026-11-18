//! Native catalogue and Home layout models shared by the desktop UI.
//!
//! Home preferences are persisted through the existing settings repository so
//! the catalogue layer does not own a second database or depend on GPUI.

use std::collections::HashSet;

use senzo_domain::{DomainError, RepositoryError, SettingKey, SettingsRepository};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const HOME_SETTINGS_KEY: &str = "home.catalogs";
pub const HOME_SETTINGS_VERSION: u16 = 1;
pub const MAX_HOME_SECTIONS: usize = 32;
pub const MAX_HOME_SECTION_TITLE_CHARS: usize = 80;
pub const MAX_HERO_SOURCES: usize = 2;
pub const BUILT_IN_HOME_CATALOGUE_DESCRIPTOR_VERSION: u32 = 1;

/// Provider-neutral identity of a catalogue request. Its revision is part of
/// the identity so changing a provider's request contract invalidates stale
/// request/cache entries without changing a user's stable section key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeCatalogDescriptor {
    pub provider: String,
    pub media_type: String,
    pub catalog_id: String,
    pub revision: u32,
}

impl HomeCatalogDescriptor {
    pub fn new(
        provider: impl Into<String>,
        media_type: impl Into<String>,
        catalog_id: impl Into<String>,
        revision: u32,
    ) -> Result<Self, HomeCatalogDefinitionError> {
        let descriptor = Self {
            provider: provider.into(),
            media_type: media_type.into(),
            catalog_id: catalog_id.into(),
            revision,
        };
        if !valid_descriptor_component(&descriptor.provider, 48)
            || !valid_descriptor_component(&descriptor.media_type, 24)
            || !valid_descriptor_component(&descriptor.catalog_id, 128)
            || descriptor.revision == 0
        {
            return Err(HomeCatalogDefinitionError::InvalidDescriptor);
        }
        Ok(descriptor)
    }

    /// Length-prefixed identity is deterministic and safe from delimiter ambiguity.
    pub fn identity(&self) -> String {
        format!(
            "v{}|p{}:{}|t{}:{}|c{}:{}",
            self.revision,
            self.provider.len(),
            self.provider,
            self.media_type.len(),
            self.media_type,
            self.catalog_id.len(),
            self.catalog_id
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltInHomeCatalog {
    Trending,
    PopularMovies,
    PopularTv,
}

impl BuiltInHomeCatalog {
    pub const ALL: [Self; 3] = [Self::Trending, Self::PopularMovies, Self::PopularTv];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trending => "trending",
            Self::PopularMovies => "popular-movies",
            Self::PopularTv => "popular-tv",
        }
    }

    pub const fn default_title(self) -> &'static str {
        match self {
            Self::Trending => "Trending this week",
            Self::PopularMovies => "Popular movies",
            Self::PopularTv => "Popular TV shows",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "trending" => Some(Self::Trending),
            "popular-movies" => Some(Self::PopularMovies),
            "popular-tv" => Some(Self::PopularTv),
            _ => None,
        }
    }
}

/// Runtime definition of one Home shelf. The descriptor, not the display
/// title or order, determines the request/cache identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeCatalogDefinition {
    pub key: String,
    pub default_title: String,
    pub descriptor: HomeCatalogDescriptor,
    pub supports_pagination: bool,
}

impl HomeCatalogDefinition {
    pub fn built_in(catalog: BuiltInHomeCatalog) -> Self {
        let (media_type, catalog_id) = match catalog {
            BuiltInHomeCatalog::Trending => ("all", "trending-week"),
            BuiltInHomeCatalog::PopularMovies => ("movie", "popular"),
            BuiltInHomeCatalog::PopularTv => ("tv", "popular"),
        };
        Self {
            key: catalog.as_str().to_owned(),
            default_title: catalog.default_title().to_owned(),
            descriptor: HomeCatalogDescriptor {
                provider: "senzo-tmdb".to_owned(),
                media_type: media_type.to_owned(),
                catalog_id: catalog_id.to_owned(),
                revision: BUILT_IN_HOME_CATALOGUE_DESCRIPTOR_VERSION,
            },
            supports_pagination: true,
        }
    }

    pub fn built_ins() -> [Self; 3] {
        BuiltInHomeCatalog::ALL.map(Self::built_in)
    }

    pub fn built_in_catalog(&self) -> Option<BuiltInHomeCatalog> {
        if self.descriptor.provider != "senzo-tmdb" {
            return None;
        }
        match (self.descriptor.media_type.as_str(), self.descriptor.catalog_id.as_str()) {
            ("all", "trending-week") => Some(BuiltInHomeCatalog::Trending),
            ("movie", "popular") => Some(BuiltInHomeCatalog::PopularMovies),
            ("tv", "popular") => Some(BuiltInHomeCatalog::PopularTv),
            _ => None,
        }
    }

    pub fn cache_identity(&self) -> String {
        format!(
            "home:{}|{}|pagination={}",
            self.key,
            self.descriptor.identity(),
            self.supports_pagination
        )
    }
}

#[derive(Debug, Error)]
pub enum HomeCatalogDefinitionError {
    #[error("Home catalogue descriptor contains an invalid or oversized field")]
    InvalidDescriptor,
}

fn valid_descriptor_component(value: &str, max_bytes: usize) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= max_bytes
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-_".contains(byte)
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeSectionPreference {
    /// Stable provider-qualified identifier. Future add-ons can use namespaced identifiers.
    pub key: String,
    /// Optional user title. None uses the provider's default title.
    pub title: Option<String>,
    pub enabled: bool,
    pub hero_source_enabled: bool,
    pub order: u32,
}

impl Default for HomeSectionPreference {
    fn default() -> Self {
        Self {
            key: String::new(),
            title: None,
            enabled: true,
            hero_source_enabled: false,
            order: 0,
        }
    }
}

impl HomeSectionPreference {
    pub fn display_title(&self) -> &str {
        self.title.as_deref().unwrap_or(match self.key.as_str() {
            "trending" => "Trending this week",
            "popular-movies" => "Popular movies",
            "popular-tv" => "Popular TV shows",
            _ => self.key.as_str(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeSettings {
    pub schema_version: u16,
    pub hero_enabled: bool,
    pub sections: Vec<HomeSectionPreference>,
}

impl Default for HomeSettings {
    fn default() -> Self {
        Self {
            schema_version: HOME_SETTINGS_VERSION,
            hero_enabled: true,
            sections: vec![
                HomeSectionPreference {
                    key: "trending".to_owned(),
                    title: None,
                    enabled: true,
                    hero_source_enabled: true,
                    order: 0,
                },
                HomeSectionPreference {
                    key: "popular-movies".to_owned(),
                    title: None,
                    enabled: true,
                    hero_source_enabled: false,
                    order: 1,
                },
                HomeSectionPreference {
                    key: "popular-tv".to_owned(),
                    title: None,
                    enabled: true,
                    hero_source_enabled: false,
                    order: 2,
                },
            ],
        }
    }
}

impl HomeSettings {
    /// Missing settings use defaults. Malformed persisted JSON is returned as an
    /// error so the UI can log recovery instead of silently overwriting data.
    pub fn load(repository: &dyn SettingsRepository) -> Result<Self, HomeSettingsError> {
        let key = SettingKey::new(HOME_SETTINGS_KEY)?;
        let Some(payload) = repository.get(&key)? else {
            return Ok(Self::default());
        };
        let settings: Self = serde_json::from_str(&payload)?;
        if settings.schema_version > HOME_SETTINGS_VERSION {
            return Err(HomeSettingsError::UnsupportedVersion {
                found: settings.schema_version,
                supported: HOME_SETTINGS_VERSION,
            });
        }
        Ok(settings.normalized())
    }

    /// Persist a normalized, versioned payload using the existing local settings store.
    pub fn save(&self, repository: &dyn SettingsRepository) -> Result<(), HomeSettingsError> {
        let key = SettingKey::new(HOME_SETTINGS_KEY)?;
        let payload = serde_json::to_string(&self.clone().normalized())?;
        repository.set(&key, &payload)?;
        Ok(())
    }

    /// Preserve valid provider preferences, deduplicate keys and add defaults
    /// for built-in sections introduced by newer application versions.
    pub fn normalized(mut self) -> Self {
        let mut seen = HashSet::new();
        let mut sections = Vec::with_capacity(self.sections.len().min(MAX_HOME_SECTIONS));
        for mut section in self.sections.drain(..).take(MAX_HOME_SECTIONS) {
            if !valid_section_key(&section.key) || !seen.insert(section.key.clone()) {
                continue;
            }
            section.title = normalized_title(section.title);
            if !section.enabled {
                section.hero_source_enabled = false;
            }
            sections.push(section);
        }

        let defaults = Self::default().sections;
        let missing_defaults = defaults
            .iter()
            .filter(|section| !seen.contains(&section.key))
            .count();
        sections.truncate(MAX_HOME_SECTIONS.saturating_sub(missing_defaults));
        seen = sections.iter().map(|section| section.key.clone()).collect();

        for mut section in defaults {
            if seen.insert(section.key.clone()) && sections.len() < MAX_HOME_SECTIONS {
                section.order = sections
                    .iter()
                    .map(|item| item.order)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                sections.push(section);
            }
        }

        sections.sort_by_key(|section| section.order);
        let mut hero_sources = 0;
        for (index, section) in sections.iter_mut().enumerate() {
            section.order = index as u32;
            if section.hero_source_enabled {
                if !section.enabled || hero_sources >= MAX_HERO_SOURCES {
                    section.hero_source_enabled = false;
                } else {
                    hero_sources += 1;
                }
            }
        }

        self.schema_version = HOME_SETTINGS_VERSION;
        self.sections = sections;
        self
    }

    pub fn set_hero_enabled(&mut self, enabled: bool) {
        self.hero_enabled = enabled;
    }

    pub fn set_section_enabled(&mut self, key: &str, enabled: bool) -> bool {
        let Some(section) = self.sections.iter_mut().find(|section| section.key == key) else {
            return false;
        };
        section.enabled = enabled;
        if !enabled {
            section.hero_source_enabled = false;
        }
        true
    }

    /// Select a section as a hero source. At most two enabled sections can supply hero cards.
    pub fn set_hero_source_enabled(&mut self, key: &str, enabled: bool) -> bool {
        let Some(index) = self.sections.iter().position(|section| section.key == key) else {
            return false;
        };
        if enabled {
            let selected = self
                .sections
                .iter()
                .filter(|section| {
                    section.enabled && section.hero_source_enabled && section.key != key
                })
                .count();
            if selected >= MAX_HERO_SOURCES || !self.sections[index].enabled {
                return false;
            }
        }
        self.sections[index].hero_source_enabled = enabled;
        true
    }

    pub fn set_section_title(
        &mut self,
        key: &str,
        title: Option<&str>,
    ) -> Result<(), HomeSettingsError> {
        let normalized = match title.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => {
                if value.chars().count() > MAX_HOME_SECTION_TITLE_CHARS
                    || value.chars().any(char::is_control)
                {
                    return Err(HomeSettingsError::InvalidTitle);
                }
                Some(value.to_owned())
            }
            None => None,
        };
        let Some(section) = self.sections.iter_mut().find(|section| section.key == key) else {
            return Err(HomeSettingsError::UnknownSection(key.to_owned()));
        };
        section.title = normalized;
        Ok(())
    }

    /// Move a section by one position while keeping stable identifiers and contiguous order.
    pub fn move_section(&mut self, key: &str, direction: isize) -> bool {
        if direction == 0 {
            return false;
        }
        let Some(from) = self.sections.iter().position(|section| section.key == key) else {
            return false;
        };
        let to = from as isize + direction.signum();
        if !(0..self.sections.len() as isize).contains(&to) {
            return false;
        }
        let section = self.sections.remove(from);
        self.sections.insert(to as usize, section);
        for (index, section) in self.sections.iter_mut().enumerate() {
            section.order = index as u32;
        }
        true
    }
}

#[derive(Debug, Error)]
pub enum HomeSettingsError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("Home settings version {found} is newer than supported version {supported}")]
    UnsupportedVersion { found: u16, supported: u16 },
    #[error(
        "Home section title must be at most {MAX_HOME_SECTION_TITLE_CHARS} characters and contain no control characters"
    )]
    InvalidTitle,
    #[error("Home section '{0}' does not exist")]
    UnknownSection(String),
}

fn valid_section_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 192
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".:_-".contains(byte)
        })
        && !key.contains("..")
        && !key.contains("::")
        && !key.contains("//")
}

fn normalized_title(title: Option<String>) -> Option<String> {
    title.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty()
            || trimmed.chars().count() > MAX_HOME_SECTION_TITLE_CHARS
            || trimmed.chars().any(char::is_control)
        {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    #[derive(Default)]
    struct MemorySettings(Mutex<HashMap<String, String>>);

    impl SettingsRepository for MemorySettings {
        fn get(&self, key: &SettingKey) -> Result<Option<String>, RepositoryError> {
            let values = self.0.lock().map_err(|_| {
                RepositoryError::Unavailable("test settings lock poisoned".to_owned())
            })?;
            Ok(values.get(key.as_str()).cloned())
        }

        fn set(&self, key: &SettingKey, value: &str) -> Result<(), RepositoryError> {
            let mut values = self.0.lock().map_err(|_| {
                RepositoryError::Unavailable("test settings lock poisoned".to_owned())
            })?;
            values.insert(key.as_str().to_owned(), value.to_owned());
            Ok(())
        }
    }

    #[test]
    fn built_in_catalogue_definitions_have_stable_keys_and_cache_identities() {
        let definitions = HomeCatalogDefinition::built_ins();
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.key.as_str())
                .collect::<Vec<_>>(),
            vec!["trending", "popular-movies", "popular-tv"]
        );
        assert_eq!(
            definitions[0].built_in_catalog(),
            Some(BuiltInHomeCatalog::Trending)
        );
        assert_eq!(
            definitions[1].built_in_catalog(),
            Some(BuiltInHomeCatalog::PopularMovies)
        );
        assert_eq!(
            definitions[2].built_in_catalog(),
            Some(BuiltInHomeCatalog::PopularTv)
        );
        assert_eq!(
            HomeCatalogDefinition::built_in(BuiltInHomeCatalog::Trending).cache_identity(),
            HomeCatalogDefinition::built_in(BuiltInHomeCatalog::Trending).cache_identity()
        );
        assert_ne!(
            HomeCatalogDefinition::built_in(BuiltInHomeCatalog::Trending).cache_identity(),
            HomeCatalogDefinition::built_in(BuiltInHomeCatalog::PopularMovies).cache_identity()
        );
    }

    #[test]
    fn catalogue_descriptor_identity_changes_with_revision_and_source_fields() {
        let base = HomeCatalogDescriptor::new("provider-a", "movie", "popular", 1).unwrap();
        let same = HomeCatalogDescriptor::new("provider-a", "movie", "popular", 1).unwrap();
        let newer = HomeCatalogDescriptor::new("provider-a", "movie", "popular", 2).unwrap();
        let other_provider =
            HomeCatalogDescriptor::new("provider-b", "movie", "popular", 1).unwrap();
        let other_type = HomeCatalogDescriptor::new("provider-a", "tv", "popular", 1).unwrap();

        assert_eq!(base.identity(), same.identity());
        assert_ne!(base.identity(), newer.identity());
        assert_ne!(base.identity(), other_provider.identity());
        assert_ne!(base.identity(), other_type.identity());
    }

    #[test]
    fn catalogue_descriptors_reject_unsafe_or_unbounded_fields() {
        assert!(HomeCatalogDescriptor::new("Bad Provider", "movie", "popular", 1).is_err());
        assert!(HomeCatalogDescriptor::new("provider", "movie/type", "popular", 1).is_err());
        assert!(HomeCatalogDescriptor::new("provider", "movie", "x".repeat(129), 1).is_err());
        assert!(HomeCatalogDescriptor::new("provider", "movie", "popular", 0).is_err());
    }

    #[test]
    fn defaults_include_three_sections_and_one_hero_source() {
        let settings = HomeSettings::default();
        assert!(settings.hero_enabled);
        assert_eq!(settings.sections.len(), 3);
        assert_eq!(settings.sections[0].key, "trending");
        assert!(settings.sections[0].hero_source_enabled);
        assert_eq!(
            settings.sections.iter().filter(|item| item.enabled).count(),
            3
        );
    }

    #[test]
    fn moving_sections_keeps_stable_keys_and_contiguous_order() {
        let mut settings = HomeSettings::default();
        assert!(settings.move_section("popular-tv", -1));
        assert_eq!(settings.sections[1].key, "popular-tv");
        assert_eq!(
            settings
                .sections
                .iter()
                .map(|item| item.order)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(!settings.move_section("trending", -1));
        assert!(!settings.move_section("missing", -1));
    }

    #[test]
    fn disabled_sections_cannot_remain_hero_sources() {
        let mut settings = HomeSettings::default();
        assert!(settings.set_section_enabled("trending", false));
        assert!(!settings.sections[0].hero_source_enabled);
    }

    #[test]
    fn at_most_two_enabled_sections_can_supply_the_hero() {
        let mut settings = HomeSettings::default();
        assert!(settings.set_hero_source_enabled("popular-movies", true));
        assert!(!settings.set_hero_source_enabled("popular-tv", true));
        assert!(settings.set_hero_source_enabled("popular-movies", false));
        assert!(settings.set_hero_source_enabled("popular-tv", true));
    }

    #[test]
    fn custom_titles_are_trimmed_and_bounded() {
        let mut settings = HomeSettings::default();
        settings
            .set_section_title("trending", Some("  My picks  "))
            .unwrap();
        assert_eq!(settings.sections[0].display_title(), "My picks");
        assert!(matches!(
            settings.set_section_title("trending", Some(&"x".repeat(81))),
            Err(HomeSettingsError::InvalidTitle)
        ));
    }

    #[test]
    fn persistence_round_trip_preserves_layout_preferences() {
        let repository = MemorySettings::default();
        let mut settings = HomeSettings::default();
        settings.move_section("popular-tv", -1);
        settings.set_section_enabled("popular-movies", false);
        settings
            .set_section_title("trending", Some("For tonight"))
            .unwrap();
        settings.save(&repository).unwrap();

        let loaded = HomeSettings::load(&repository).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn normalization_restores_missing_builtins_even_when_preferences_reach_the_capacity_limit() {
        let settings = HomeSettings {
            schema_version: HOME_SETTINGS_VERSION,
            hero_enabled: true,
            sections: (0..MAX_HOME_SECTIONS)
                .map(|index| HomeSectionPreference {
                    key: format!("addon:catalog-{index}"),
                    title: None,
                    enabled: true,
                    hero_source_enabled: false,
                    order: index as u32,
                })
                .collect(),
        };

        let normalized = settings.normalized();
        assert_eq!(normalized.sections.len(), MAX_HOME_SECTIONS);
        for key in ["trending", "popular-movies", "popular-tv"] {
            assert!(normalized.sections.iter().any(|section| section.key == key));
        }
    }

    #[test]
    fn normalization_restores_missing_builtins_and_deduplicates_keys() {
        let payload = r#"{
            "schema_version": 1,
            "hero_enabled": true,
            "sections": [
                {"key":"trending","enabled":true,"hero_source_enabled":true,"order":0},
                {"key":"trending","enabled":false,"hero_source_enabled":false,"order":1},
                {"key":"custom:addon:catalog","enabled":true,"hero_source_enabled":false,"order":4}
            ]
        }"#;
        let settings: HomeSettings = serde_json::from_str(payload).unwrap();
        let settings = settings.normalized();
        assert_eq!(
            settings
                .sections
                .iter()
                .filter(|item| item.key == "trending")
                .count(),
            1
        );
        assert!(
            settings
                .sections
                .iter()
                .any(|item| item.key == "popular-movies")
        );
        assert!(
            settings
                .sections
                .iter()
                .any(|item| item.key == "popular-tv")
        );
        assert!(
            settings
                .sections
                .iter()
                .any(|item| item.key == "custom:addon:catalog")
        );
    }
}
