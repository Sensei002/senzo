use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiHealth {
    pub service: String,
    pub status: String,
    pub version: Option<String>,
    pub tmdb_configured: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    Multi,
    Movie,
    Tv,
}

impl SearchType {
    pub const fn as_api_value(self) -> &'static str {
        match self {
            Self::Multi => "multi",
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Movie,
    Tv,
    Person,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaListResponse {
    pub page: u32,
    #[serde(default)]
    pub results: Vec<MediaItem>,
    pub total_pages: Option<u32>,
    pub total_results: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: u64,
    pub media_type: Option<MediaType>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub original_title: Option<String>,
    pub original_name: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub profile_path: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<u64>,
    pub popularity: Option<f64>,
    pub adult: Option<bool>,
    pub original_language: Option<String>,
}

impl MediaItem {
    pub fn display_title(&self) -> &str {
        self.title
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or("Untitled")
    }

    pub fn release_year(&self) -> Option<&str> {
        self.release_date
            .as_deref()
            .or(self.first_air_date.as_deref())
            .and_then(|date| date.get(..4))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovieDetails {
    pub id: u64,
    pub title: String,
    pub original_title: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub status: Option<String>,
    pub release_date: Option<String>,
    pub runtime: Option<u32>,
    pub adult: Option<bool>,
    pub homepage: Option<String>,
    pub imdb_id: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<u64>,
    #[serde(default)]
    pub genres: Vec<Genre>,
    #[serde(default)]
    pub production_companies: Vec<ProductionCompany>,
    #[serde(default)]
    pub production_countries: Vec<ProductionCountry>,
    pub credits: Option<Credits>,
    pub videos: Option<Videos>,
    pub images: Option<Images>,
    pub external_ids: Option<ExternalIds>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TvDetails {
    pub id: u64,
    pub name: String,
    pub original_name: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub status: Option<String>,
    pub first_air_date: Option<String>,
    pub last_air_date: Option<String>,
    pub number_of_seasons: Option<u32>,
    pub number_of_episodes: Option<u32>,
    pub episode_run_time: Option<Vec<u32>>,
    pub homepage: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<u64>,
    #[serde(default)]
    pub genres: Vec<Genre>,
    #[serde(default)]
    pub seasons: Vec<SeasonSummary>,
    #[serde(default)]
    pub production_companies: Vec<ProductionCompany>,
    pub aggregate_credits: Option<Credits>,
    pub credits: Option<Credits>,
    pub videos: Option<Videos>,
    pub images: Option<Images>,
    pub external_ids: Option<ExternalIds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Genre {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionCompany {
    pub id: u64,
    pub name: String,
    pub logo_path: Option<String>,
    pub origin_country: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionCountry {
    pub iso_3166_1: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Credits {
    #[serde(default)]
    pub cast: Vec<CastMember>,
    #[serde(default)]
    pub crew: Vec<CastMember>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CastMember {
    pub id: u64,
    pub name: String,
    pub original_name: Option<String>,
    pub character: Option<String>,
    pub job: Option<String>,
    pub department: Option<String>,
    pub known_for_department: Option<String>,
    pub profile_path: Option<String>,
    pub order: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: u64,
    pub name: String,
    pub biography: Option<String>,
    pub birthday: Option<String>,
    pub deathday: Option<String>,
    pub place_of_birth: Option<String>,
    pub profile_path: Option<String>,
    pub known_for_department: Option<String>,
    pub also_known_as: Option<Vec<String>>,
    pub gender: Option<u8>,
    pub homepage: Option<String>,
    pub imdb_id: Option<String>,
    pub popularity: Option<f64>,
    pub combined_credits: Option<PersonCredits>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonCredits {
    #[serde(default)]
    pub cast: Vec<PersonCredit>,
    #[serde(default)]
    pub crew: Vec<PersonCredit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonCredit {
    pub id: u64,
    pub media_type: Option<MediaType>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub original_title: Option<String>,
    pub original_name: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<u64>,
    pub popularity: Option<f64>,
    pub character: Option<String>,
    pub job: Option<String>,
    pub department: Option<String>,
    pub episode_count: Option<u32>,
}

impl PersonCredit {
    pub fn display_title(&self) -> &str {
        self.title
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or("Untitled")
    }

    pub fn release_date(&self) -> Option<&str> {
        self.release_date
            .as_deref()
            .or(self.first_air_date.as_deref())
    }

    pub fn release_year(&self) -> Option<&str> {
        self.release_date().and_then(|date| date.get(..4))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Videos {
    #[serde(default)]
    pub results: Vec<VideoItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoItem {
    pub id: String,
    pub key: String,
    pub name: String,
    pub site: String,
    #[serde(rename = "type")]
    pub video_type: String,
    pub official: Option<bool>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Images {
    #[serde(default)]
    pub backdrops: Vec<ImageItem>,
    #[serde(default)]
    pub posters: Vec<ImageItem>,
    #[serde(default)]
    pub logos: Vec<ImageItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageItem {
    pub file_path: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub aspect_ratio: Option<f64>,
    pub iso_639_1: Option<String>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalIds {
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<u64>,
    pub tvmaze_id: Option<u64>,
    pub facebook_id: Option<String>,
    pub instagram_id: Option<String>,
    pub twitter_id: Option<String>,
    pub wikidata_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeasonSummary {
    pub id: u64,
    pub name: String,
    pub overview: Option<String>,
    pub season_number: u32,
    pub episode_count: Option<u32>,
    pub air_date: Option<String>,
    pub poster_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeasonDetails {
    pub id: u64,
    pub name: String,
    pub overview: Option<String>,
    pub season_number: u32,
    pub air_date: Option<String>,
    pub poster_path: Option<String>,
    #[serde(default)]
    pub episodes: Vec<EpisodeDetails>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeDetails {
    pub id: u64,
    pub name: String,
    pub overview: Option<String>,
    pub episode_number: u32,
    pub season_number: u32,
    pub air_date: Option<String>,
    pub still_path: Option<String>,
    pub runtime: Option<u32>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<u64>,
    pub credits: Option<Credits>,
    pub videos: Option<Videos>,
    pub images: Option<Images>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixed_search_results_and_resolves_display_titles() {
        let response: MediaListResponse = serde_json::from_str(
            r#"{
                "page": 1,
                "results": [
                    {"id": 27205, "media_type": "movie", "title": "Inception", "release_date": "2010-07-15"},
                    {"id": 1399, "media_type": "tv", "name": "Breaking Bad", "first_air_date": "2008-01-20"}
                ],
                "total_pages": 1,
                "total_results": 2
            }"#,
        )
        .unwrap();
        assert_eq!(response.results[0].display_title(), "Inception");
        assert_eq!(response.results[1].display_title(), "Breaking Bad");
        assert_eq!(response.results[1].release_year(), Some("2008"));
    }

    #[test]
    fn parses_appended_movie_enrichment() {
        let details: MovieDetails = serde_json::from_str(
            r#"{
                "id": 27205,
                "title": "Inception",
                "runtime": 148,
                "credits": {"cast": [{"id": 1, "name": "Actor", "character": "Cobb"}], "crew": []},
                "videos": {"results": [{"id": "v1", "key": "abc", "name": "Trailer", "site": "YouTube", "type": "Trailer"}]},
                "images": {"posters": [{"file_path": "/poster.jpg", "width": 500, "height": 750}], "backdrops": [], "logos": []},
                "external_ids": {"imdb_id": "tt1375666"}
            }"#,
        )
        .unwrap();
        assert_eq!(
            details.credits.unwrap().cast[0].character.as_deref(),
            Some("Cobb")
        );
        assert_eq!(details.videos.unwrap().results[0].video_type, "Trailer");
        assert_eq!(
            details.external_ids.unwrap().imdb_id.as_deref(),
            Some("tt1375666")
        );
    }
}
