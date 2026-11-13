//! External subtitle providers and subtitle-search models.

pub mod opensubtitles;

pub use opensubtitles::{
    OpenSubtitlesClient, OpenSubtitlesFile, OpenSubtitlesResult, SubtitleSearch,
};
