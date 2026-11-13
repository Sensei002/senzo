//! External subtitle providers and subtitle-search models.

pub mod credentials;
pub mod opensubtitles;

pub use credentials::OpenSubtitlesCredentialStore;
pub use opensubtitles::{
    OpenSubtitlesClient, OpenSubtitlesFile, OpenSubtitlesResult, SubtitleSearch,
};
