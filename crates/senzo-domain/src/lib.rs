//! Stable domain types and service contracts shared by SENZO crates.

use serde::{Deserialize, Serialize};
use thiserror::Error;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPage {
    Home,
    Library,
    Settings,
}

impl AppPage {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Library => "Library",
            Self::Settings => "Settings",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Home => "The native SENZO application foundation is ready.",
            Self::Library => "Your local library will live here in a later phase.",
            Self::Settings => {
                "Application preferences and provider settings will be added in later phases."
            }
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
}

pub trait SettingsRepository: Send + Sync {
    fn get(&self, key: &SettingKey) -> Result<Option<String>, RepositoryError>;
    fn set(&self, key: &SettingKey, value: &str) -> Result<(), RepositoryError>;
}

pub trait ProfileRepository: Send + Sync {
    fn list_profiles(&self) -> Result<Vec<Profile>, RepositoryError>;
    fn create_profile(&self, name: &str) -> Result<Profile, RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
