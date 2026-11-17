use std::{
    net::IpAddr,
    sync::Arc,
    time::Duration,
};

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
        ApiHealth, EpisodeDetails, ExternalIds, MediaListResponse, MovieDetails, SearchType,
        SeasonDetails, TvDetails,
    },
};

pub const DEFAULT_API_BASE_URL: &str = "https://senzo-api.vercel.app";
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
        let mut base_url = Url::parse(&config.base_url).map_err(|_| MetadataError::InvalidBaseUrl)?;
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
            return Err(MetadataError::InvalidInput("season number must be between 0 and 100"));
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
        return Err(MetadataError::InvalidInput("page must be between 1 and 500"));
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

    async fn start_server(
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
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
        let task = tokio::spawn(async move {
            client.trending(1, "en-US", &child).await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancellation.cancel();
        assert!(matches!(task.await.unwrap(), Err(MetadataError::Cancelled)));
    }
}
