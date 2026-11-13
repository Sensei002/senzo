use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

const TMDB_API_BASE: &str = "https://api.themoviedb.org/3";
const TMDB_IMAGE_BASE: &str = "https://image.tmdb.org/t/p";

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbSearchPage {
    pub page: u32,
    pub results: Vec<TmdbSearchResult>,
    pub total_pages: u32,
    pub total_results: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbSearchResult {
    pub id: u64,
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
}

impl TmdbSearchResult {
    pub fn display_title(&self) -> Option<&str> {
        self.title
            .as_deref()
            .or(self.name.as_deref())
            .or(self.original_title.as_deref())
            .or(self.original_name.as_deref())
    }

    pub fn media_id(&self, media_type: &str) -> String {
        format!("tmdb:{media_type}:{}", self.id)
    }
}

#[derive(Clone)]
pub struct TmdbClient {
    client: Client,
    access_token: String,
    language: String,
}

impl TmdbClient {
    /// Creates a TMDB client using a TMDB API Read Access Token.
    /// The token is never included in errors or application logs.
    pub fn new(access_token: impl Into<String>, language: impl Into<String>) -> Result<Self> {
        let access_token = access_token.into();
        if access_token.trim().is_empty() {
            bail!("TMDB API Read Access Token is not configured");
        }

        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
                .build()
                .context("failed to initialize the TMDB HTTP client")?,
            access_token,
            language: language.into(),
        })
    }

    pub async fn search_movies(&self, query: &str, page: u32) -> Result<TmdbSearchPage> {
        self.search("movie", query, page).await
    }

    pub async fn search_tv(&self, query: &str, page: u32) -> Result<TmdbSearchPage> {
        self.search("tv", query, page).await
    }

    async fn search(&self, kind: &str, query: &str, page: u32) -> Result<TmdbSearchPage> {
        let url = format!("{TMDB_API_BASE}/search/{kind}");
        let page = page.max(1).to_string();

        self.client
            .get(url)
            .bearer_auth(&self.access_token)
            .query(&[
                ("query", query),
                ("language", self.language.as_str()),
                ("page", page.as_str()),
                ("include_adult", "false"),
            ])
            .send()
            .await
            .context("TMDB search request failed")?
            .error_for_status()
            .context("TMDB rejected the search request")?
            .json::<TmdbSearchPage>()
            .await
            .context("TMDB returned an invalid search response")
    }

    pub async fn movie_details(&self, tmdb_id: u64) -> Result<Value> {
        self.details(
            &format!("{TMDB_API_BASE}/movie/{tmdb_id}"),
            "credits,videos,images,external_ids",
        )
        .await
    }

    pub async fn tv_details(&self, tmdb_id: u64) -> Result<Value> {
        self.details(
            &format!("{TMDB_API_BASE}/tv/{tmdb_id}"),
            "aggregate_credits,videos,images,external_ids",
        )
        .await
    }

    async fn details(&self, url: &str, append: &str) -> Result<Value> {
        self.client
            .get(url)
            .bearer_auth(&self.access_token)
            .query(&[
                ("language", self.language.as_str()),
                ("append_to_response", append),
            ])
            .send()
            .await
            .context("TMDB details request failed")?
            .error_for_status()
            .context("TMDB rejected the details request")?
            .json::<Value>()
            .await
            .context("TMDB returned an invalid details response")
    }

    /// Fetches a TV season, including its episode list.
    /// Season zero is valid for TMDB specials.
    pub async fn tv_season(&self, tv_id: u64, season_number: u32) -> Result<TmdbSeason> {
        self.get_json(&format!(
            "{TMDB_API_BASE}/tv/{tv_id}/season/{season_number}"
        ))
        .await
        .context("failed to fetch TMDB TV season")
    }

    /// Fetches an individual TV episode.
    pub async fn tv_episode(
        &self,
        tv_id: u64,
        season_number: u32,
        episode_number: u32,
    ) -> Result<TmdbEpisode> {
        if episode_number == 0 {
            bail!("TMDB episode numbers start at 1");
        }

        self.get_json(&format!(
            "{TMDB_API_BASE}/tv/{tv_id}/season/{season_number}/episode/{episode_number}"
        ))
        .await
        .context("failed to fetch TMDB TV episode")
    }

    /// Resolves provider identifiers for a movie.
    pub async fn movie_external_ids(&self, tmdb_id: u64) -> Result<TmdbExternalIds> {
        self.get_json(&format!("{TMDB_API_BASE}/movie/{tmdb_id}/external_ids"))
            .await
            .context("failed to fetch TMDB movie external identifiers")
    }

    /// Resolves provider identifiers for a TV show.
    pub async fn tv_external_ids(&self, tmdb_id: u64) -> Result<TmdbExternalIds> {
        self.get_json(&format!("{TMDB_API_BASE}/tv/{tmdb_id}/external_ids"))
            .await
            .context("failed to fetch TMDB TV external identifiers")
    }

    async fn get_json<T>(&self, url: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.client
            .get(url)
            .bearer_auth(&self.access_token)
            .query(&[("language", self.language.as_str())])
            .send()
            .await
            .context("TMDB request failed")?
            .error_for_status()
            .context("TMDB rejected the request")?
            .json::<T>()
            .await
            .context("TMDB returned an invalid response")
    }

    pub fn image_url(path: Option<&str>, size: ImageSize) -> Option<String> {
        let path = path?.trim();
        if path.is_empty() || !path.starts_with('/') {
            return None;
        }
        Some(format!(
            "{TMDB_IMAGE_BASE}/{}/{}",
            size.as_str(),
            path.trim_start_matches('/')
        ))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbSeason {
    pub id: u64,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub season_number: u32,
    pub air_date: Option<String>,
    pub episodes: Vec<TmdbEpisode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbEpisode {
    pub id: u64,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub season_number: u32,
    pub episode_number: u32,
    pub air_date: Option<String>,
    pub runtime: Option<u32>,
    pub vote_average: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TmdbExternalIds {
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<u64>,
    pub wikidata_id: Option<String>,
    pub facebook_id: Option<String>,
    pub instagram_id: Option<String>,
    pub twitter_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSize {
    PosterSmall,
    Poster,
    Backdrop,
    Original,
}

impl ImageSize {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PosterSmall => "w342",
            Self::Poster => "w500",
            Self::Backdrop => "w1280",
            Self::Original => "original",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_tv_season_and_external_ids() {
        let season: TmdbSeason = serde_json::from_str(
            r#"{"id":10,"name":"Season 1","season_number":1,"episodes":[{"id":11,"season_number":1,"episode_number":1,"name":"Pilot"}]}"#,
        )
        .unwrap();
        assert_eq!(season.episodes.len(), 1);
        assert_eq!(season.episodes[0].episode_number, 1);

        let ids: TmdbExternalIds =
            serde_json::from_str(r#"{"imdb_id":"tt1234567","tvdb_id":42}"#).unwrap();
        assert_eq!(ids.imdb_id.as_deref(), Some("tt1234567"));
        assert_eq!(ids.tvdb_id, Some(42));
    }

    #[test]
    fn builds_tmdb_image_urls_from_relative_paths() {
        assert_eq!(
            TmdbClient::image_url(Some("/poster.jpg"), ImageSize::Poster).as_deref(),
            Some("https://image.tmdb.org/t/p/w500/poster.jpg")
        );
        assert_eq!(TmdbClient::image_url(None, ImageSize::Poster), None);
        assert_eq!(
            TmdbClient::image_url(Some("https://example.com/poster.jpg"), ImageSize::Poster),
            None
        );
    }

    #[test]
    fn creates_stable_movie_and_tv_identifiers() {
        let result = TmdbSearchResult {
            id: 550,
            title: Some("Fight Club".into()),
            name: None,
            original_title: None,
            original_name: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: None,
            first_air_date: None,
            vote_average: None,
            vote_count: None,
        };
        assert_eq!(result.media_id("movie"), "tmdb:movie:550");
        assert_eq!(result.display_title(), Some("Fight Club"));
    }
}
