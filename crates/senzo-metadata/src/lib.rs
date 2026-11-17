//! Typed client for the hosted SENZO metadata API.
//!
//! The desktop is a public client. It never receives or stores a shared TMDB
//! credential; all TMDB authentication remains on the hosted SENZO API.

mod cache;
mod client;
mod models;

pub use client::{DEFAULT_API_BASE_URL, MetadataClient, MetadataClientConfig, MetadataError};
pub use models::{
    ApiHealth, CastMember, Credits, EpisodeDetails, ExternalIds, Genre, ImageItem, Images,
    MediaItem, MediaListResponse, MediaType, MovieDetails, Person, ProductionCompany,
    ProductionCountry, SearchType, SeasonDetails, SeasonSummary, TvDetails, VideoItem, Videos,
};
