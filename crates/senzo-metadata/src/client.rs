use std::{net::IpAddr, sync::Arc, time::Duration};

use reqwest::StatusCode;
use senzo_network::{HttpClient, NetworkError, ValidatedHttpUrl};
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use crate::{
    cache::MetadataCache,
    models::{
        ApiHealth, DiscoverSort, EpisodeDetails, ExternalIds, GenreListResponse, MediaListResponse,
        MediaType, MovieDetails, Person, SearchType, SeasonDetails, TvDetails,
    },
};

pub const DEFAULT_API_BASE_URL: &str = "https://senzo-api.vercel.app";

#[derive(Debug, Clone, Copy)]
pub struct DiscoverRequest<'a> {
    pub media_type: MediaType,
    pub genre_id: Option<u64>,
    pub year: Option<u32>,
    pub sort: DiscoverSort,
    pub page: u32,
    pub language: &'a str,
}
const MAX_RESPONSE_BYTES: usize = 12 * 1024 * 1024;
const MAX_API_ERROR_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone)]
pub struct MetadataClientConfig {
    pub base_url: String,
    pub cache_capacity: usize,
    pub max_retries: usize,
}

impl Default for MetadataClientConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_API_BASE_URL.to_owned(),
            cache_capacity: 256,
            max_retries: 2,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MetadataError {
    #[error("metadata API base URL is invalid or uses an unsupported scheme")]
    InvalidBaseUrl,
    #[error("metadata request parameters are invalid: {0}")]
    InvalidInput(&'static str),
    #[error("metadata request was cancelled")]
    Cancelled,
    #[error("metadata response exceeded the configured size limit")]
    ResponseTooLarge,
    #[error("metadata API returned HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("metadata network request failed: {0}")]
    Network(String),
    #[error("metadata response could not be decoded: {0}")]
    Decode(String),
    #[error("metadata cache lock was poisoned")]
    CachePoisoned,
}

#[derive(Clone)]
pub struct MetadataClient {
    http: HttpClient,
    base_url: Url,
    cache: Arc<MetadataCache>,
    max_retries: usize,
}

impl MetadataClient {
    pub fn new(http: HttpClient, config: MetadataClientConfig) -> Result<Self, MetadataError> {
        let mut base_url =
            Url::parse(&config.base_url).map_err(|_| MetadataError::InvalidBaseUrl)?;
        let loopback = match base_url.host() {
            Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            Some(Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
            Some(Host::Ipv6(address)) => IpAddr::V6(address).is_loopback(),
            None => false,
        };

        if (base_url.scheme() != "https" && !(base_url.scheme() == "http" && loopback))
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(MetadataError::InvalidBaseUrl);
        }

        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }

        Ok(Self {
            http,
            base_url,
            cache: Arc::new(MetadataCache::new(config.cache_capacity)),
            max_retries: config.max_retries.min(4),
        })
    }

    pub fn cache_stats(&self) -> Result<(u64, u64, usize), MetadataError> {
        let stats = self.cache.stats()?;
        Ok((stats.hits, stats.misses, stats.entries))
    }

    pub async fn health(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<ApiHealth, MetadataError> {
        self.get_json("api/health", &[], Duration::ZERO, cancellation)
            .await
    }

    pub async fn search(
        &self,
        query: &str,
        search_type: SearchType,
        page: u32,
        language: &str,
        cancellation: &CancellationToken,
    ) -> Result<MediaListResponse, MetadataError> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 120 || query.chars().any(char::is_control) {
            return Err(MetadataError::InvalidInput(
                "search query must contain 1 to 120 printable characters",
            ));
        }
        validate_page(page)?;
        validate_language(language)?;
        let page = page.to_string();
        self.get_json(
            "api/tmdb/search",
            &[
                ("query", query),
                ("type", search_type.as_api_value()),
                ("page", &page),
                ("language", language),
            ],
            Duration::from_secs(180),
            cancellation,
        )
        .await
    }

    pub async fn trending(
        &self,
        page: u32,
        language: &str,
        cancellation: &CancellationToken,
    ) -> Result<MediaListResponse, MetadataError> {
        validate_page(page)?;
        validate_language(language)?;
        let page = page.to_string();
        self.get_json(
            "api/tmdb/trending/all/week",
            &[("page", &page), ("language", language)],
            Duration::from_secs(300),
            cancellation,
        )
        .await
    }

    pub async fn popular_movies(
        &self,
        page: u32,
        language: &str,
        cancellation: &CancellationToken,
    ) -> Result<MediaListResponse, MetadataError> {
        self.popular("movie", page, language, cancellation).await
    }

    pub async fn popular_tv(
        &self,
        page: u32,
        language: &str,
        cancellation: &CancellationToken,
    ) -> Result<MediaListResponse, MetadataError> {
        self.popular("tv", page, language, cancellation).await
    }

    pub async fn genres(
        &self,
        media_type: MediaType,
        language: &str,
        cancellation: &CancellationToken,
    ) -> Result<GenreListResponse, MetadataError> {
        let segment = match media_type {
            MediaType::Movie => "movie",
            MediaType::Tv => "tv",
            _ => {
                return Err(MetadataError::InvalidInput(
                    "genres require movie or TV media type",
                ));
            }
        };
        validate_language(language)?;
        self.get_json(
            &format!("api/tmdb/genre/{segment}/list"),
            &[("language", language)],
            Duration::from_secs(86_400),
            cancellation,
        )
        .await
    }

    pub async fn discover(
        &self,
        request: DiscoverRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<MediaListResponse, MetadataError> {
        let DiscoverRequest {
            media_type,
            genre_id,
            year,
            sort,
            page,
            language,
        } = request;
        let (segment, year_key) = match media_type {
            MediaType::Movie => ("movie", "primary_release_year"),
            MediaType::Tv => ("tv", "first_air_date_year"),
            _ => {
                return Err(MetadataError::InvalidInput(
                    "discover requires movie or TV media type",
                ));
            }
        };
        if genre_id.is_some_and(|id| !(1..=10_000).contains(&id)) {
            return Err(MetadataError::InvalidInput(
                "genre ID must be between 1 and 10000",
            ));
        }
        if year.is_some_and(|value| !(1870..=2100).contains(&value)) {
            return Err(MetadataError::InvalidInput(
                "year must be between 1870 and 2100",
            ));
        }
        validate_page(page)?;
        validate_language(language)?;

        let page = page.to_string();
        let sort_by = sort.as_api_value(media_type);
        let genre = genre_id.map(|id| id.to_string());
        let year = year.map(|value| value.to_string());
        let mut query = vec![
            ("page", page.as_str()),
            ("language", language),
            ("sort_by", sort_by),
        ];
        if let Some(genre) = genre.as_deref() {
            query.push(("with_genres", genre));
        }
        if let Some(year) = year.as_deref() {
            query.push((year_key, year));
        }
        self.get_json(
            &format!("api/tmdb/discover/{segment}"),
            &query,
            Duration::from_secs(300),
            cancellation,
        )
        .await
    }

    pub async fn movie(
        &self,
        id: u64,
        cancellation: &CancellationToken,
    ) -> Result<MovieDetails, MetadataError> {
        validate_id(id)?;
        self.get_json(
            &format!("api/tmdb/movie/{id}"),
            &[],
            Duration::from_secs(900),
            cancellation,
        )
        .await
    }

    pub async fn movie_external_ids(
        &self,
        id: u64,
        cancellation: &CancellationToken,
    ) -> Result<ExternalIds, MetadataError> {
        validate_id(id)?;
        self.get_json(
            &format!("api/tmdb/movie/{id}/external_ids"),
            &[],
            Duration::from_secs(900),
            cancellation,
        )
        .await
    }

    pub async fn tv(
        &self,
        id: u64,
        cancellation: &CancellationToken,
    ) -> Result<TvDetails, MetadataError> {
        validate_id(id)?;
        self.get_json(
            &format!("api/tmdb/tv/{id}"),
            &[],
            Duration::from_secs(900),
            cancellation,
        )
        .await
    }

    pub async fn person(
        &self,
        id: u64,
        language: &str,
        cancellation: &CancellationToken,
    ) -> Result<Person, MetadataError> {
        validate_id(id)?;
        validate_language(language)?;
        self.get_json(
            &format!("api/tmdb/person/{id}"),
            &[("language", language)],
            Duration::from_secs(900),
            cancellation,
        )
        .await
    }

    pub async fn tv_external_ids(
        &self,
        id: u64,
        cancellation: &CancellationToken,
    ) -> Result<ExternalIds, MetadataError> {
        validate_id(id)?;
        self.get_json(
            &format!("api/tmdb/tv/{id}/external_ids"),
            &[],
            Duration::from_secs(900),
            cancellation,
        )
        .await
    }

    pub async fn season(
        &self,
        series_id: u64,
        season_number: u32,
        cancellation: &CancellationToken,
    ) -> Result<SeasonDetails, MetadataError> {
        validate_id(series_id)?;
        if season_number > 100 {
            return Err(MetadataError::InvalidInput(
                "season number must be between 0 and 100",
            ));
        }
        self.get_json(
            &format!("api/tmdb/tv/{series_id}/season/{season_number}"),
            &[],
            Duration::from_secs(900),
            cancellation,
        )
        .await
    }

    pub async fn episode(
        &self,
        series_id: u64,
        season_number: u32,
        episode_number: u32,
        cancellation: &CancellationToken,
    ) -> Result<EpisodeDetails, MetadataError> {
        validate_id(series_id)?;
        if season_number > 100 || !(1..=10_000).contains(&episode_number) {
            return Err(MetadataError::InvalidInput(
                "season must be 0 to 100 and episode must be 1 to 10000",
            ));
        }
        self.get_json(
            &format!("api/tmdb/tv/{series_id}/season/{season_number}/episode/{episode_number}"),
            &[],
            Duration::from_secs(900),
            cancellation,
        )
        .await
    }

    async fn popular(
        &self,
        media_type: &str,
        page: u32,
        language: &str,
        cancellation: &CancellationToken,
    ) -> Result<MediaListResponse, MetadataError> {
        validate_page(page)?;
        validate_language(language)?;
        let page = page.to_string();
        self.get_json(
            &format!("api/tmdb/{media_type}/popular"),
            &[("page", &page), ("language", language)],
            Duration::from_secs(300),
            cancellation,
        )
        .await
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        ttl: Duration,
        cancellation: &CancellationToken,
    ) -> Result<T, MetadataError> {
        let url = self.endpoint_url(path, query)?;
        let key = url.as_url().as_str().to_owned();

        if let Some(body) = self.cache.get(&key)? {
            return serde_json::from_slice(&body)
                .map_err(|error| MetadataError::Decode(error.to_string()));
        }

        let cell = self.cache.in_flight(&key)?;
        let shared = tokio::select! {
            _ = cancellation.cancelled() => Err(MetadataError::Cancelled),
            result = cell.get_or_init(|| async {
                let loaded = self
                    .load_json_body(&url, cancellation)
                    .await
                    .map(Arc::<[u8]>::from);
                if let Ok(bytes) = &loaded {
                    self.cache.insert(key.clone(), Arc::clone(bytes), ttl)?;
                }
                loaded
            }) => result.clone(),
        };
        let _ = self.cache.finish_in_flight(&key, &cell);
        let body = shared?;
        serde_json::from_slice(&body).map_err(|error| MetadataError::Decode(error.to_string()))
    }

    fn endpoint_url(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<ValidatedHttpUrl, MetadataError> {
        let mut url = self
            .base_url
            .join(path)
            .map_err(|_| MetadataError::InvalidBaseUrl)?;
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        ValidatedHttpUrl::parse(url.as_str()).map_err(|_| MetadataError::InvalidBaseUrl)
    }

    async fn load_json_body(
        &self,
        url: &ValidatedHttpUrl,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, MetadataError> {
        for attempt in 0..=self.max_retries {
            let response = tokio::select! {
                _ = cancellation.cancelled() => return Err(MetadataError::Cancelled),
                response = self.http.get(url) => response,
            };

            let mut response = match response {
                Ok(response) => response,
                Err(NetworkError::Transport(_)) if attempt < self.max_retries => {
                    self.wait_for(retry_delay(attempt), cancellation).await?;
                    tracing::debug!(attempt, "retrying a transient metadata transport error");
                    continue;
                }
                Err(error) => return Err(MetadataError::Network(error.to_string())),
            };

            let status = response.status();
            if !status.is_success() {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok());
                let retryable = matches!(
                    status,
                    StatusCode::REQUEST_TIMEOUT
                        | StatusCode::INTERNAL_SERVER_ERROR
                        | StatusCode::BAD_GATEWAY
                        | StatusCode::SERVICE_UNAVAILABLE
                        | StatusCode::GATEWAY_TIMEOUT
                ) || (status == StatusCode::TOO_MANY_REQUESTS
                    && retry_after.is_some_and(|seconds| seconds <= 3));

                if retryable && attempt < self.max_retries {
                    let delay = retry_after
                        .map(Duration::from_secs)
                        .unwrap_or_else(|| retry_delay(attempt));
                    self.wait_for(delay, cancellation).await?;
                    continue;
                }

                let body = read_limited(&mut response, MAX_API_ERROR_BYTES, cancellation).await?;
                let message = serde_json::from_slice::<ApiError>(&body)
                    .ok()
                    .and_then(|error| error.error)
                    .unwrap_or_else(|| "The metadata provider returned an error.".to_owned());
                return Err(MetadataError::Api {
                    status: status.as_u16(),
                    message,
                });
            }

            let body = read_limited(&mut response, MAX_RESPONSE_BYTES, cancellation).await?;
            if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
                return Err(MetadataError::Decode(
                    "the API returned a non-JSON success response".to_owned(),
                ));
            }
            return Ok(body);
        }

        Err(MetadataError::Network(
            "metadata request exhausted its retry budget".to_owned(),
        ))
    }

    async fn wait_for(
        &self,
        delay: Duration,
        cancellation: &CancellationToken,
    ) -> Result<(), MetadataError> {
        tokio::select! {
            _ = cancellation.cancelled() => Err(MetadataError::Cancelled),
            _ = sleep(delay) => Ok(()),
        }
    }
}

#[derive(serde::Deserialize)]
struct ApiError {
    error: Option<String>,
}

async fn read_limited(
    response: &mut reqwest::Response,
    max_bytes: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, MetadataError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(MetadataError::ResponseTooLarge);
    }

    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(MetadataError::Cancelled),
            chunk = response.chunk() => chunk,
        }
        .map_err(|error| MetadataError::Network(error.without_url().to_string()))?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(MetadataError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(250_u64.saturating_mul(1_u64 << attempt.min(3)))
}

fn validate_id(id: u64) -> Result<(), MetadataError> {
    if !(1..=999_999_999_999).contains(&id) {
        return Err(MetadataError::InvalidInput(
            "media ID must be a positive 1 to 12 digit integer",
        ));
    }
    Ok(())
}

fn validate_page(page: u32) -> Result<(), MetadataError> {
    if !(1..=500).contains(&page) {
        return Err(MetadataError::InvalidInput(
            "page must be between 1 and 500",
        ));
    }
    Ok(())
}

fn validate_language(language: &str) -> Result<(), MetadataError> {
    let mut parts = language.split('-');
    let primary = parts.next().unwrap_or_default();
    let region = parts.next();
    if parts.next().is_some()
        || !(2..=3).contains(&primary.len())
        || !primary.bytes().all(|byte| byte.is_ascii_lowercase())
        || region.is_some_and(|value| {
            value.len() != 2 || !value.bytes().all(|byte| byte.is_ascii_uppercase())
        })
    {
        return Err(MetadataError::InvalidInput(
            "language must be a tag such as en-US",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_sort_maps_to_media_specific_tmdb_fields() {
        assert_eq!(
            DiscoverSort::Popularity.as_api_value(MediaType::Movie),
            "popularity.desc"
        );
        assert_eq!(
            DiscoverSort::Newest.as_api_value(MediaType::Movie),
            "primary_release_date.desc"
        );
        assert_eq!(
            DiscoverSort::Newest.as_api_value(MediaType::Tv),
            "first_air_date.desc"
        );
    }

    #[tokio::test]
    async fn requests_filtered_movie_discovery_with_validated_query_fields() {
        let (base, server) =
            start_server(r#"{"page":2,"results":[],"total_pages":3,"total_results":0}"#).await;
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: base,
                ..Default::default()
            },
        )
        .unwrap();
        let response = client
            .discover(
                DiscoverRequest {
                    media_type: MediaType::Movie,
                    genre_id: Some(28),
                    year: Some(2024),
                    sort: DiscoverSort::Newest,
                    page: 2,
                    language: "en-US",
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let request = server.await.unwrap();
        assert_eq!(response.page, 2);
        assert!(request.starts_with("GET /api/tmdb/discover/movie?"));
        assert!(request.contains("with_genres=28"));
        assert!(request.contains("primary_release_year=2024"));
        assert!(request.contains("sort_by=primary_release_date.desc"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn requests_movie_genres_from_the_hosted_api() {
        let (base, server) = start_server(r#"{"genres":[{"id":28,"name":"Action"}]}"#).await;
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: base,
                ..Default::default()
            },
        )
        .unwrap();
        let response = client
            .genres(MediaType::Movie, "en-US", &CancellationToken::new())
            .await
            .unwrap();
        let request = server.await.unwrap();
        assert_eq!(response.genres[0].name, "Action");
        assert!(request.starts_with("GET /api/tmdb/genre/movie/list?"));
    }

    #[tokio::test]
    async fn rejects_invalid_discover_filters_before_network_access() {
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: "http://127.0.0.1:3000".to_owned(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            client
                .discover(
                    DiscoverRequest {
                        media_type: MediaType::Movie,
                        genre_id: Some(0),
                        year: None,
                        sort: DiscoverSort::Popularity,
                        page: 1,
                        language: "en-US",
                    },
                    &CancellationToken::new()
                )
                .await,
            Err(MetadataError::InvalidInput(_))
        ));
        assert!(matches!(
            client
                .discover(
                    DiscoverRequest {
                        media_type: MediaType::Tv,
                        genre_id: None,
                        year: Some(2200),
                        sort: DiscoverSort::Popularity,
                        page: 1,
                        language: "en-US",
                    },
                    &CancellationToken::new()
                )
                .await,
            Err(MetadataError::InvalidInput(_))
        ));
        assert!(matches!(
            client
                .genres(MediaType::Person, "en-US", &CancellationToken::new())
                .await,
            Err(MetadataError::InvalidInput(_))
        ));
    }

    #[test]
    fn rejects_remote_cleartext_and_url_credentials() {
        let http = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: "http://metadata.example.test".to_owned(),
                ..Default::default()
            },
        );
        assert!(matches!(http, Err(MetadataError::InvalidBaseUrl)));

        let credentials = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: "https://user:pass@metadata.example.test".to_owned(),
                ..Default::default()
            },
        );
        assert!(matches!(credentials, Err(MetadataError::InvalidBaseUrl)));
    }

    #[test]
    fn permits_loopback_http_for_local_development() {
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: "http://127.0.0.1:3000".to_owned(),
                ..Default::default()
            },
        );
        assert!(client.is_ok());
    }

    #[test]
    fn validates_request_parameters_before_network_access() {
        assert!(validate_page(1).is_ok());
        assert!(validate_page(500).is_ok());
        assert!(validate_page(0).is_err());
        assert!(validate_page(501).is_err());
        assert!(validate_language("en-US").is_ok());
        assert!(validate_language("eng").is_ok());
        assert!(validate_language("EN-us").is_err());
        assert!(validate_language("en-US-extra").is_err());
        assert!(validate_id(27205).is_ok());
        assert!(validate_id(0).is_err());
    }

    async fn start_server(body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let count = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn requests_hosted_api_route_without_authorization_headers() {
        let (base, server) = start_server(
            r#"{"page":1,"results":[{"id":27205,"media_type":"movie","title":"Inception"}]}"#,
        )
        .await;
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: base,
                ..Default::default()
            },
        )
        .unwrap();

        let result = client
            .search(
                "arrival",
                SearchType::Multi,
                1,
                "en-US",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let request = server.await.unwrap();
        assert_eq!(result.results[0].display_title(), "Inception");
        assert!(request.starts_with("GET /api/tmdb/search?"));
        assert!(request.contains("query=arrival"));
        assert!(request.contains("type=multi"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn requests_person_profile_with_combined_cast_and_crew_credits() {
        let (base, server) = start_server(
            r#"{
                "id":287,
                "name":"Brad Pitt",
                "biography":"An actor and producer.",
                "birthday":"1963-12-18",
                "known_for_department":"Acting",
                "profile_path":"/profile.jpg",
                "combined_credits":{
                    "cast":[{"id":550,"media_type":"movie","title":"Fight Club","character":"Tyler Durden","poster_path":"/fight-club.jpg","release_date":"1999-10-15","vote_average":8.4}],
                    "crew":[{"id":680,"media_type":"movie","title":"Pulp Fiction","job":"Producer","department":"Production","release_date":"1994-09-10"}]
                }
            }"#,
        )
        .await;
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: base,
                ..Default::default()
            },
        )
        .unwrap();

        let person = client
            .person(287, "en-US", &CancellationToken::new())
            .await
            .unwrap();
        let request = server.await.unwrap();

        assert_eq!(person.name, "Brad Pitt");
        let credits = person
            .combined_credits
            .expect("combined credits are present");
        assert_eq!(credits.cast[0].display_title(), "Fight Club");
        assert_eq!(credits.cast[0].character.as_deref(), Some("Tyler Durden"));
        assert_eq!(credits.cast[0].release_year(), Some("1999"));
        assert_eq!(credits.crew[0].job.as_deref(), Some("Producer"));
        assert!(request.starts_with("GET /api/tmdb/person/287?"));
        assert!(request.contains("language=en-US"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn rejects_invalid_person_ids_before_network_access() {
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: "http://127.0.0.1:3000".to_owned(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            client.person(0, "en-US", &CancellationToken::new()).await,
            Err(MetadataError::InvalidInput(_))
        ));
        assert!(matches!(
            client.person(287, "EN-us", &CancellationToken::new()).await,
            Err(MetadataError::InvalidInput(_))
        ));
    }

    #[tokio::test]
    async fn reports_offline_connection_failures_as_network_errors() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: format!("http://{address}"),
                max_retries: 0,
                ..Default::default()
            },
        )
        .unwrap();

        let error = client
            .trending(1, "en-US", &CancellationToken::new())
            .await
            .unwrap_err();

        assert!(matches!(error, MetadataError::Network(_)));
    }

    #[tokio::test]
    async fn cancels_a_request_while_waiting_for_the_server() {
        use tokio::io::AsyncReadExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        });

        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: format!("http://{address}"),
                ..Default::default()
            },
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let child = cancellation.clone();
        let task = tokio::spawn(async move { client.trending(1, "en-US", &child).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancellation.cancel();
        assert!(matches!(task.await.unwrap(), Err(MetadataError::Cancelled)));
    }

    #[tokio::test]
    async fn retries_transient_5xx_and_recovers() {
        let (base, server) = start_sequence_server(vec![
            MockResponse::new(503, None, r#"{"error":"provider unavailable"}"#),
            MockResponse::new(
                200,
                None,
                r#"{"page":1,"results":[{"id":27205,"media_type":"movie","title":"Inception"}]}"#,
            ),
        ])
        .await;
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: base,
                max_retries: 1,
                ..Default::default()
            },
        )
        .unwrap();

        let response = client
            .search(
                "inception",
                SearchType::Multi,
                1,
                "en-US",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let requests = server.await.unwrap();

        assert_eq!(response.results[0].display_title(), "Inception");
        assert_eq!(requests.len(), 2, "a transient 503 should be retried once");
    }

    #[tokio::test]
    async fn retries_rate_limits_only_when_retry_after_is_short() {
        let (base, server) = start_sequence_server(vec![
            MockResponse::new(429, Some("0"), r#"{"error":"rate limited"}"#),
            MockResponse::new(200, None, r#"{"page":1,"results":[]}"#),
        ])
        .await;
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: base,
                max_retries: 1,
                ..Default::default()
            },
        )
        .unwrap();

        client
            .search(
                "inception",
                SearchType::Multi,
                1,
                "en-US",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn does_not_retry_rate_limits_without_a_short_retry_after() {
        for retry_after in [None, Some("4")] {
            let (base, server) = start_sequence_server(vec![MockResponse::new(
                429,
                retry_after,
                r#"{"error":"rate limited"}"#,
            )])
            .await;
            let client = MetadataClient::new(
                HttpClient::new(Default::default()).unwrap(),
                MetadataClientConfig {
                    base_url: base,
                    max_retries: 3,
                    ..Default::default()
                },
            )
            .unwrap();

            let error = client
                .search(
                    "inception",
                    SearchType::Multi,
                    1,
                    "en-US",
                    &CancellationToken::new(),
                )
                .await
                .unwrap_err();
            assert!(matches!(error, MetadataError::Api { status: 429, .. }));
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn does_not_cache_failed_responses() {
        let (base, server) = start_sequence_server(vec![
            MockResponse::new(503, None, r#"{"error":"provider unavailable"}"#),
            MockResponse::new(200, None, r#"{"page":1,"results":[]}"#),
        ])
        .await;
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: base,
                max_retries: 0,
                ..Default::default()
            },
        )
        .unwrap();

        let first = client.trending(1, "en-US", &CancellationToken::new()).await;
        assert!(matches!(first, Err(MetadataError::Api { status: 503, .. })));

        let second = client
            .trending(1, "en-US", &CancellationToken::new())
            .await
            .unwrap();
        assert!(second.results.is_empty());
        assert_eq!(
            server.await.unwrap().len(),
            2,
            "the failed response must not poison the resource cache"
        );
    }

    #[tokio::test]
    async fn coalesces_identical_concurrent_requests() {
        let (base, server) = start_sequence_server(vec![MockResponse::new(
            200,
            None,
            r#"{"page":1,"results":[]}"#,
        )])
        .await;
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: base,
                ..Default::default()
            },
        )
        .unwrap();

        let first_cancellation = CancellationToken::new();
        let second_cancellation = CancellationToken::new();
        let first = client.trending(1, "en-US", &first_cancellation);
        let second = client.trending(1, "en-US", &second_cancellation);
        let (first, second) = tokio::join!(first, second);

        assert!(first.unwrap().results.is_empty());
        assert!(second.unwrap().results.is_empty());
        assert_eq!(
            server.await.unwrap().len(),
            1,
            "identical concurrent GETs should share one network request"
        );
    }

    #[tokio::test]
    async fn cancels_during_retry_after_backoff() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::sync::oneshot;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (response_sent, response_received) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            let body = r#"{"error":"provider unavailable"}"#;
            let response = format!(
                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 3\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = response_sent.send(());
        });
        let client = MetadataClient::new(
            HttpClient::new(Default::default()).unwrap(),
            MetadataClientConfig {
                base_url: format!("http://{address}"),
                max_retries: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let request =
            tokio::spawn(async move { client.trending(1, "en-US", &task_cancellation).await });

        response_received.await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancellation.cancel();

        assert!(matches!(
            request.await.unwrap(),
            Err(MetadataError::Cancelled)
        ));
        server.await.unwrap();
    }

    #[derive(Clone, Copy)]
    struct MockResponse {
        status: u16,
        retry_after: Option<&'static str>,
        body: &'static str,
    }

    impl MockResponse {
        fn new(status: u16, retry_after: Option<&'static str>, body: &'static str) -> Self {
            Self {
                status,
                retry_after,
                body,
            }
        }
    }

    async fn start_sequence_server(
        responses: Vec<MockResponse>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(responses.len());
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 4096];
                let count = socket.read(&mut request).await.unwrap();
                requests.push(String::from_utf8_lossy(&request[..count]).to_string());

                let reason = match response.status {
                    200 => "OK",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    502 => "Bad Gateway",
                    503 => "Service Unavailable",
                    504 => "Gateway Timeout",
                    _ => "Test Response",
                };
                let retry_after = response
                    .retry_after
                    .map(|value| format!("Retry-After: {value}\r\n"))
                    .unwrap_or_default();
                let wire_response = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
                    response.status,
                    reason,
                    response.body.len(),
                    retry_after,
                    response.body
                );
                socket.write_all(wire_response.as_bytes()).await.unwrap();
            }
            requests
        });

        (format!("http://{address}"), task)
    }
}
