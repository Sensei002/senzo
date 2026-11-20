//! Typed client for the hosted SENZO metadata API.
//!
//! The desktop is a public client. It never receives or stores a shared TMDB
//! credential; all TMDB authentication remains on the hosted SENZO API.

mod cache;
mod client;
mod models;

pub use client::{
    DEFAULT_API_BASE_URL, DiscoverRequest, MetadataClient, MetadataClientConfig, MetadataError,
};
pub use models::{
    ApiHealth, CastMember, Credits, DiscoverSort, EpisodeDetails, ExternalIds, Genre,
    GenreListResponse, ImageItem, Images, MediaItem, MediaListResponse, MediaType, MovieDetails,
    Person, PersonCredit, PersonCredits, ProductionCompany, ProductionCountry, SearchType,
    SeasonDetails, SeasonSummary, TvDetails, VideoItem, Videos,
};
