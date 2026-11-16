use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SENZO_API_BASE: &str = "https://senzo-api.vercel.app/api/tmdb";
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
    #[serde(default)]
    pub media_type: Option<String>,
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
    base_url: String,
    language: String,
}

impl TmdbClient {
    /// Creates a client for the public SENZO metadata API. No TMDB credential is
    /// distributed with or requested from the desktop application.
    pub fn from_service(language: impl Into<String>) -> Result<Self> {
        let base_url = std::env::var("SENZO_API_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                option_env!("SENZO_API_BASE_URL")
                    .unwrap_or(SENZO_API_BASE)
                    .to_owned()
            });
        Self::new(base_url, language)
    }

    /// Creates a client for a SENZO-compatible metadata API endpoint.
    pub fn new(base_url: impl Into<String>, language: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into().trim().trim_end_matches('/').to_owned();
        let parsed =
            url::Url::parse(&base_url).context("SENZO metadata API base URL is invalid")?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            bail!("SENZO metadata API must use a valid HTTPS base URL");
        }

        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .context("failed to initialize the SENZO metadata HTTP client")?,
            base_url,
            language: language.into(),
        })
    }

    async fn get_json<T>(&self, path: &str, extra: &[(&str, String)]) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let mut query = vec![("language", self.language.clone())];
        query.extend(extra.iter().map(|(key, value)| (*key, value.clone())));
        self.client
            .get(url)
            .query(&query)
            .send()
            .await
            .context("SENZO metadata API request failed")?
            .error_for_status()
            .context("SENZO metadata API returned an error")?
            .json::<T>()
            .await
            .context("SENZO metadata API returned an invalid response")
    }

    /// Fetches the combined trending movie and television catalogue for the current week.
    pub async fn trending_all_week(&self, page: u32) -> Result<TmdbSearchPage> {
        self.get_json("trending/all/week", &[("page", page.max(1).to_string())])
            .await
    }

    pub async fn popular_movies(&self, page: u32) -> Result<TmdbSearchPage> {
        self.get_json("movie/popular", &[("page", page.max(1).to_string())])
            .await
    }

    pub async fn popular_tv(&self, page: u32) -> Result<TmdbSearchPage> {
        self.get_json("tv/popular", &[("page", page.max(1).to_string())])
            .await
    }

    pub async fn search_movies(&self, query: &str, page: u32) -> Result<TmdbSearchPage> {
        self.search("movie", query, page).await
    }

    pub async fn search_tv(&self, query: &str, page: u32) -> Result<TmdbSearchPage> {
        self.search("tv", query, page).await
    }

    async fn search(&self, kind: &str, query: &str, page: u32) -> Result<TmdbSearchPage> {
        self.get_json(
            "search",
            &[
                ("query", query.to_owned()),
                ("type", kind.to_owned()),
                ("page", page.max(1).to_string()),
            ],
        )
        .await
    }

    pub async fn movie_details(&self, tmdb_id: u64) -> Result<Value> {
        self.get_json(&format!("movie/{tmdb_id}"), &[]).await
    }

    pub async fn tv_details(&self, tmdb_id: u64) -> Result<Value> {
        self.get_json(&format!("tv/{tmdb_id}"), &[]).await
    }

    pub async fn tv_season(&self, tv_id: u64, season_number: u32) -> Result<TmdbSeason> {
        self.get_json(&format!("tv/{tv_id}/season/{season_number}"), &[])
            .await
            .context("failed to fetch TV season from SENZO metadata API")
    }

    pub async fn tv_episode(
        &self,
        tv_id: u64,
        season_number: u32,
        episode_number: u32,
    ) -> Result<TmdbEpisode> {
        if episode_number == 0 {
            bail!("TMDB episode numbers start at 1");
        }
        self.get_json(
            &format!("tv/{tv_id}/season/{season_number}/episode/{episode_number}"),
            &[],
        )
        .await
        .context("failed to fetch TV episode from SENZO metadata API")
    }

    pub async fn movie_external_ids(&self, tmdb_id: u64) -> Result<TmdbExternalIds> {
        self.get_json(&format!("movie/{tmdb_id}/external_ids"), &[])
            .await
            .context("failed to fetch movie external identifiers")
    }

    pub async fn tv_external_ids(&self, tmdb_id: u64) -> Result<TmdbExternalIds> {
        self.get_json(&format!("tv/{tmdb_id}/external_ids"), &[])
            .await
            .context("failed to fetch TV external identifiers")
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TmdbSeason {
    pub id: u64,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub season_number: u32,
    pub air_date: Option<String>,
    pub episodes: Vec<TmdbEpisode>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
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
    fn metadata_client_requires_a_valid_https_service_url() {
        assert!(TmdbClient::new("https://senzo-api.vercel.app/api/tmdb", "en-US").is_ok());
        assert!(TmdbClient::new("http://senzo-api.vercel.app/api/tmdb", "en-US").is_err());
        assert!(TmdbClient::new("https://user:pass@example.com/api/tmdb", "en-US").is_err());
        assert!(TmdbClient::new("https://example.com/api/tmdb?token=secret", "en-US").is_err());
    }

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
            media_type: None,
        };
        assert_eq!(result.media_id("movie"), "tmdb:movie:550");
        assert_eq!(result.display_title(), Some("Fight Club"));
    }
}
