//! OpenSubtitles.com REST API v1 client.
//!
//! The client is deliberately UI- and playback-independent. Callers must supply
//! the user's API key explicitly; credentials are never persisted or logged here.
//! Searches are only performed when explicitly requested by the user.

use anyhow::{Context, anyhow, bail};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://api.opensubtitles.com/api/v1";
const USER_AGENT: &str = concat!("SENZO/", env!("CARGO_PKG_VERSION"));
const MAX_SUBTITLE_BYTES: usize = 20 * 1024 * 1024;
const MAX_RESULTS: u8 = 50;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubtitleSearch {
    pub query: Option<String>,
    /// OpenSubtitles expects the numeric portion of an IMDb ID, without "tt".
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<u64>,
    /// Comma-separated ISO 639-1 language codes, e.g. "en,fr".
    pub languages: String,
    pub year: Option<u16>,
    pub season_number: Option<u16>,
    pub episode_number: Option<u16>,
}

impl SubtitleSearch {
    fn validate(&self) -> anyhow::Result<()> {
        let has_query = self
            .query
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_imdb = self
            .imdb_id
            .as_deref()
            .is_some_and(|value| normalize_imdb_id(value).is_some());
        let has_tmdb = self.tmdb_id.is_some_and(|value| value > 0);
        if !has_query && !has_imdb && !has_tmdb {
            bail!("subtitle search requires a title, IMDb ID, or TMDB ID");
        }
        if self.languages.trim().is_empty() {
            bail!("at least one subtitle language is required");
        }
        if self.season_number.is_some() != self.episode_number.is_some() {
            bail!("season and episode numbers must be provided together");
        }
        if self.season_number == Some(0) || self.episode_number == Some(0) {
            bail!("season and episode numbers must be greater than zero");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenSubtitlesResult {
    pub id: String,
    #[serde(rename = "type")]
    pub result_type: String,
    pub attributes: OpenSubtitlesAttributes,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenSubtitlesAttributes {
    pub language: Option<String>,
    pub release: Option<String>,
    pub download_count: Option<u64>,
    pub ratings: Option<serde_json::Value>,
    pub points: Option<i64>,
    pub hearing_impaired: Option<bool>,
    pub foreign_parts_only: Option<bool>,
    pub ai_translated: Option<bool>,
    pub machine_translated: Option<serde_json::Value>,
    pub from_trusted: Option<bool>,
    pub uploader: Option<OpenSubtitlesUploader>,
    pub feature_details: Option<OpenSubtitlesFeature>,
    pub files: Vec<OpenSubtitlesFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenSubtitlesUploader {
    pub name: Option<String>,
    pub rank: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenSubtitlesFeature {
    pub title: Option<String>,
    pub year: Option<u16>,
    pub feature_type: Option<String>,
    pub imdb_id: Option<u64>,
    pub tmdb_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenSubtitlesFile {
    pub file_id: u64,
    pub file_name: String,
    pub cd_number: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<OpenSubtitlesResult>,
}

#[derive(Debug, Deserialize)]
struct DownloadResponse {
    link: String,
    file_name: String,
    remaining: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedSubtitle {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub remaining_downloads: Option<u32>,
}

#[derive(Clone)]
pub struct OpenSubtitlesClient {
    client: Client,
    api_key: String,
}

impl OpenSubtitlesClient {
    /// Creates a client with an explicitly supplied OpenSubtitles application key.
    pub fn new(api_key: impl Into<String>) -> anyhow::Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            bail!("OpenSubtitles API key is empty");
        }
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let host = attempt.url().host_str().unwrap_or_default();
                if attempt.previous().len() >= 5
                    || !(host == "opensubtitles.com" || host.ends_with(".opensubtitles.com"))
                {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("could not initialize the OpenSubtitles HTTP client")?;
        Ok(Self { client, api_key })
    }

    /// Reads an API key from the environment without storing it.
    pub fn from_environment() -> anyhow::Result<Self> {
        let key = std::env::var("OPENSUBTITLES_API_KEY")
            .context("set OPENSUBTITLES_API_KEY to enable online subtitle search")?;
        Self::new(key)
    }

    /// Searches only when called by the application in response to user action.
    pub async fn search(
        &self,
        search: &SubtitleSearch,
    ) -> anyhow::Result<Vec<OpenSubtitlesResult>> {
        search.validate()?;
        let mut request = self
            .client
            .get(format!("{API_BASE}/subtitles"))
            .header("Api-Key", &self.api_key)
            .query(&[("languages", search.languages.trim())])
            .query(&[("machine_translated", "exclude")]);

        if let Some(query) = search
            .query
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            request = request.query(&[("query", query.to_lowercase())]);
        }
        if let Some(imdb_id) = search.imdb_id.as_deref().and_then(normalize_imdb_id) {
            request = request.query(&[("imdb_id", imdb_id)]);
        }
        if let Some(tmdb_id) = search.tmdb_id.filter(|value| *value > 0) {
            request = request.query(&[("tmdb_id", tmdb_id)]);
        }
        if let Some(year) = search.year {
            request = request.query(&[("year", year)]);
        }
        if let Some(season) = search.season_number {
            request = request.query(&[("season_number", season)]);
        }
        if let Some(episode) = search.episode_number {
            request = request.query(&[("episode_number", episode)]);
        }

        let response = request
            .send()
            .await
            .context("OpenSubtitles search request failed")?;
        let response = checked_response(response, "search")?;
        let mut body: SearchResponse = response
            .json()
            .await
            .context("OpenSubtitles returned an invalid search response")?;
        body.data.truncate(usize::from(MAX_RESULTS));
        Ok(body.data)
    }

    /// Requests a short-lived download link and retrieves the subtitle bytes.
    pub async fn download(&self, file_id: u64) -> anyhow::Result<DownloadedSubtitle> {
        if file_id == 0 || file_id > i32::MAX as u64 {
            bail!("OpenSubtitles file ID is outside the supported range");
        }

        let response = self
            .client
            .post(format!("{API_BASE}/download"))
            .header("Api-Key", &self.api_key)
            .json(&DownloadRequest { file_id })
            .send()
            .await
            .context("OpenSubtitles download request failed")?;
        let response = checked_response(response, "download-link request")?;
        let download: DownloadResponse = response
            .json()
            .await
            .context("OpenSubtitles returned an invalid download response")?;

        let url = validate_download_url(&download.link)?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("subtitle file download failed")?;
        let response = checked_response(response, "subtitle file download")?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SUBTITLE_BYTES as u64)
        {
            bail!("subtitle download exceeds the 20 MiB safety limit");
        }
        let bytes = response
            .bytes()
            .await
            .context("could not read downloaded subtitle data")?;
        if bytes.is_empty() {
            bail!("OpenSubtitles returned an empty subtitle file");
        }
        if bytes.len() > MAX_SUBTITLE_BYTES {
            bail!("subtitle download exceeds the 20 MiB safety limit");
        }
        let file_name = safe_subtitle_file_name(&download.file_name)?;
        Ok(DownloadedSubtitle {
            file_name,
            bytes: bytes.to_vec(),
            remaining_downloads: download.remaining,
        })
    }
}

#[derive(Serialize)]
struct DownloadRequest {
    file_id: u64,
}

fn checked_response(
    response: reqwest::Response,
    operation: &str,
) -> anyhow::Result<reqwest::Response> {
    let status = response.status();
    if !status.is_success() {
        bail!("OpenSubtitles {operation} returned HTTP {status}");
    }
    Ok(response)
}

fn normalize_imdb_id(value: &str) -> Option<String> {
    let digits = value.trim().strip_prefix("tt").unwrap_or(value.trim());
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    Some(digits.to_owned())
}

fn validate_download_url(value: &str) -> anyhow::Result<Url> {
    let url = Url::parse(value).context("OpenSubtitles returned an invalid download URL")?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || !(host == "opensubtitles.com" || host.ends_with(".opensubtitles.com"))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("OpenSubtitles returned a download URL outside its trusted HTTPS domain");
    }
    Ok(url)
}

fn safe_subtitle_file_name(value: &str) -> anyhow::Result<String> {
    let name = std::path::Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("OpenSubtitles returned an invalid subtitle filename"))?;
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "srt" | "ass" | "ssa" | "vtt" | "sub" | "idx" | "sup"
    ) {
        bail!("OpenSubtitles returned an unsupported subtitle format");
    }
    if name.is_empty()
        || name.len() > 240
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
    {
        bail!("OpenSubtitles returned an unsafe subtitle filename");
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_requires_identifier_and_language() {
        assert!(SubtitleSearch::default().validate().is_err());
        assert!(
            SubtitleSearch {
                query: Some("Example".into()),
                ..SubtitleSearch::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            SubtitleSearch {
                query: Some("Example".into()),
                languages: "en".into(),
                ..SubtitleSearch::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn episode_numbers_must_be_a_valid_pair() {
        let search = SubtitleSearch {
            query: Some("Example".into()),
            languages: "en".into(),
            season_number: Some(1),
            ..SubtitleSearch::default()
        };
        assert!(search.validate().is_err());
    }

    #[test]
    fn imdb_ids_are_normalized_to_numeric_ids() {
        assert_eq!(normalize_imdb_id("tt0012345").as_deref(), Some("12345"));
        assert_eq!(normalize_imdb_id("12345").as_deref(), Some("12345"));
        assert!(normalize_imdb_id("tt-not-an-id").is_none());
        assert!(normalize_imdb_id("tt000").is_none());
    }

    #[test]
    fn download_urls_must_be_https_opensubtitles_domains() {
        assert!(validate_download_url("https://www.opensubtitles.com/download/file").is_ok());
        assert!(validate_download_url("http://www.opensubtitles.com/file").is_err());
        assert!(validate_download_url("https://example.com/file").is_err());
        assert!(validate_download_url("https://opensubtitles.com.example.org/file").is_err());
    }

    #[test]
    fn subtitle_filename_is_sanitized_and_extension_checked() {
        assert_eq!(
            safe_subtitle_file_name("../movie.srt").unwrap(),
            "movie.srt"
        );
        assert!(safe_subtitle_file_name("movie.exe").is_err());
        assert!(safe_subtitle_file_name("movie.zip").is_err());
    }

    #[test]
    fn search_results_accept_numeric_ratings_and_optional_metadata() {
        let response: SearchResponse = serde_json::from_str(
            r#"{"total_count":1,"data":[{"id":"42","type":"subtitle","attributes":{"language":"en","release":"WEB-DL","download_count":12,"ratings":9.2,"points":4,"files":[{"file_id":123,"file_name":"movie.en.srt","cd_number":1}]}}]}"#,
        )
        .expect("deserialize search response");
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].attributes.files[0].file_id, 123);
    }

    #[test]
    fn api_key_must_not_be_blank() {
        assert!(OpenSubtitlesClient::new("  ").is_err());
    }
}
