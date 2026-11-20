//! Profile-scoped Jellyfin and Emby connections.
//! Server URLs are explicitly user-configured; private LAN destinations are permitted here only.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use reqwest::{
    Client, Method, StatusCode,
    header::{HeaderMap, HeaderValue},
};
use senzo_domain::{SettingKey, SettingsRepository};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use url::Url;

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONNECTIONS: usize = 16;
const MAX_LIBRARIES: usize = 128;
const MAX_CATALOG_ITEMS: usize = 100;

pub trait MediaServerCredentialStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("media-server address must be an absolute HTTP(S) URL without embedded credentials")]
    InvalidAddress,
    #[error("media-server provider is not supported")]
    UnsupportedProvider,
    #[error("media-server authentication failed")]
    Authentication,
    #[error("media-server returned HTTP {0}")]
    HttpStatus(u16),
    #[error("media-server response exceeded its safety limit")]
    ResponseTooLarge,
    #[error("media-server response could not be parsed")]
    InvalidResponse,
    #[error("media-server is unavailable: {0}")]
    Network(String),
    #[error("media-server connection was not found")]
    NotFound,
    #[error("media-server persistence failed: {0}")]
    Persistence(String),
    #[error("media-server credential storage failed: {0}")]
    Credential(String),
    #[error("media-server catalog query is invalid")]
    InvalidQuery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaServerProvider {
    Jellyfin,
    Emby,
}

impl MediaServerProvider {
    pub fn id(self) -> &'static str {
        match self {
            Self::Jellyfin => "jellyfin",
            Self::Emby => "emby",
        }
    }
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Jellyfin => "Jellyfin",
            Self::Emby => "Emby",
        }
    }
    pub fn parse(value: &str) -> Result<Self, IntegrationError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "jellyfin" => Ok(Self::Jellyfin),
            "emby" => Ok(Self::Emby),
            _ => Err(IntegrationError::UnsupportedProvider),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LibraryKind {
    Movies,
    Series,
    Mixed,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaServerLibrary {
    pub id: String,
    pub name: String,
    pub kind: LibraryKind,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaServerConnection {
    pub id: String,
    pub provider: MediaServerProvider,
    pub name: String,
    pub address: String,
    pub remote_server_id: String,
    pub user_id: String,
    pub user_name: String,
    pub credential_ref: String,
    pub libraries: Vec<MediaServerLibrary>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaServerCatalogItem {
    pub id: String,
    pub title: String,
    pub media_type: String,
    pub year: Option<u16>,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub overview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedServerSubtitle {
    pub url: String,
    pub language: String,
    pub name: Option<String>,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedServerSource {
    pub id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub title: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub quality: Option<String>,
    pub size_bytes: Option<u64>,
    pub subtitles: Vec<ResolvedServerSubtitle>,
}

#[derive(Clone)]
pub struct MediaServerManager {
    settings: Arc<dyn SettingsRepository>,
    credentials: Arc<dyn MediaServerCredentialStore>,
    client: Client,
}

impl MediaServerManager {
    pub fn new(
        settings: Arc<dyn SettingsRepository>,
        credentials: Arc<dyn MediaServerCredentialStore>,
    ) -> Result<Self, IntegrationError> {
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| IntegrationError::Network(e.to_string()))?;
        Ok(Self {
            settings,
            credentials,
            client,
        })
    }

    pub fn list_connections(
        &self,
        profile_id: &str,
    ) -> Result<Vec<MediaServerConnection>, IntegrationError> {
        let key = connections_key(profile_id)?;
        let raw = self
            .settings
            .get(&key)
            .map_err(|e| IntegrationError::Persistence(e.to_string()))?;
        let connections: Vec<MediaServerConnection> = match raw {
            Some(raw) => {
                serde_json::from_str(&raw).map_err(|_| IntegrationError::InvalidResponse)?
            }
            None => Vec::new(),
        };
        Ok(connections.into_iter().take(MAX_CONNECTIONS).collect())
    }

    fn save_connections(
        &self,
        profile_id: &str,
        connections: &[MediaServerConnection],
    ) -> Result<(), IntegrationError> {
        if connections.len() > MAX_CONNECTIONS {
            return Err(IntegrationError::Persistence(
                "connection limit reached".into(),
            ));
        }
        let raw = serde_json::to_string(connections)
            .map_err(|e| IntegrationError::Persistence(e.to_string()))?;
        self.settings
            .set(&connections_key(profile_id)?, &raw)
            .map_err(|e| IntegrationError::Persistence(e.to_string()))
    }

    pub async fn connect(
        &self,
        profile_id: &str,
        provider: MediaServerProvider,
        name: &str,
        address: &str,
        username: &str,
        password: &str,
    ) -> Result<MediaServerConnection, IntegrationError> {
        validate_profile(profile_id)?;
        let base = normalize_address(address)?;
        if name.trim().is_empty()
            || name.len() > 64
            || username.trim().is_empty()
            || password.is_empty()
            || username.chars().any(char::is_control)
            || password.len() > 4096
        {
            return Err(IntegrationError::InvalidQuery);
        }
        let auth_url = endpoint(&base, "Users/AuthenticateByName")?;
        let (auth_header_name, auth_header) = server_auth_header(provider, None);
        let mut headers = HeaderMap::new();
        headers.insert(
            auth_header_name,
            HeaderValue::from_str(&auth_header).map_err(|_| IntegrationError::InvalidQuery)?,
        );
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        let response = self
            .client
            .request(Method::POST, auth_url)
            .headers(headers)
            .json(&json!({"Username": username.trim(), "Pw": password}))
            .send()
            .await
            .map_err(|e| IntegrationError::Network(e.without_url().to_string()))?;
        let payload = read_json(response).await?;
        let token = value_str(&payload, "AccessToken").ok_or(IntegrationError::Authentication)?;
        let user = payload
            .get("User")
            .ok_or(IntegrationError::Authentication)?;
        let user_id = value_str(user, "Id").ok_or(IntegrationError::Authentication)?;
        let user_name = value_str(user, "Name")
            .unwrap_or(username.trim())
            .to_owned();

        let credential_ref = format!("media-server:{profile_id}:{}", uuid::Uuid::new_v4());
        self.credentials
            .set(&credential_ref, token)
            .map_err(IntegrationError::Credential)?;
        let server_info = self
            .request_json_with_token(&base, "System/Info/Public", provider, token)
            .await;
        let server_id = match server_info {
            Ok(info) => value_str(&info, "Id").unwrap_or("unknown").to_owned(),
            Err(error) => {
                let _ = self.credentials.delete(&credential_ref);
                return Err(error);
            }
        };
        let connection = MediaServerConnection {
            id: uuid::Uuid::new_v4().to_string(),
            provider,
            name: name.trim().to_owned(),
            address: base.to_string(),
            remote_server_id: server_id,
            user_id: user_id.to_owned(),
            user_name,
            credential_ref,
            libraries: Vec::new(),
            enabled: true,
        };
        let mut connections = self.list_connections(profile_id)?;
        if connections.len() >= MAX_CONNECTIONS {
            let _ = self.credentials.delete(&connection.credential_ref);
            return Err(IntegrationError::Persistence(
                "connection limit reached".into(),
            ));
        }
        connections.push(connection.clone());
        if let Err(error) = self.save_connections(profile_id, &connections) {
            let _ = self.credentials.delete(&connection.credential_ref);
            return Err(error);
        }
        if let Err(error) = self.refresh_libraries(profile_id, &connection.id).await {
            tracing::warn!(provider=%provider.id(), connection_id=%connection.id, error=%error, "Connected media server, but library refresh failed");
        }
        Ok(self
            .connection(profile_id, &connection.id)?
            .unwrap_or(connection))
    }

    pub fn connection(
        &self,
        profile_id: &str,
        id: &str,
    ) -> Result<Option<MediaServerConnection>, IntegrationError> {
        Ok(self
            .list_connections(profile_id)?
            .into_iter()
            .find(|item| item.id == id))
    }

    pub fn set_enabled(
        &self,
        profile_id: &str,
        id: &str,
        enabled: bool,
    ) -> Result<(), IntegrationError> {
        let mut connections = self.list_connections(profile_id)?;
        let connection = connections
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or(IntegrationError::NotFound)?;
        connection.enabled = enabled;
        self.save_connections(profile_id, &connections)
    }

    pub fn set_library_selected(
        &self,
        profile_id: &str,
        connection_id: &str,
        library_id: &str,
        selected: bool,
    ) -> Result<(), IntegrationError> {
        let mut connections = self.list_connections(profile_id)?;
        let connection = connections
            .iter_mut()
            .find(|item| item.id == connection_id)
            .ok_or(IntegrationError::NotFound)?;
        let library = connection
            .libraries
            .iter_mut()
            .find(|item| item.id == library_id)
            .ok_or(IntegrationError::NotFound)?;
        library.selected = selected;
        self.save_connections(profile_id, &connections)
    }

    pub fn remove(&self, profile_id: &str, id: &str) -> Result<(), IntegrationError> {
        let mut connections = self.list_connections(profile_id)?;
        let old_len = connections.len();
        let removed = connections
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .ok_or(IntegrationError::NotFound)?;
        connections.retain(|item| item.id != id);
        if connections.len() == old_len {
            return Err(IntegrationError::NotFound);
        }
        self.save_connections(profile_id, &connections)?;
        self.credentials
            .delete(&removed.credential_ref)
            .map_err(IntegrationError::Credential)
    }

    pub async fn refresh_libraries(
        &self,
        profile_id: &str,
        id: &str,
    ) -> Result<Vec<MediaServerLibrary>, IntegrationError> {
        let mut connections = self.list_connections(profile_id)?;
        let connection = connections
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or(IntegrationError::NotFound)?;
        let token = self
            .credentials
            .get(&connection.credential_ref)
            .map_err(IntegrationError::Credential)?
            .ok_or(IntegrationError::Authentication)?;
        let base = normalize_address(&connection.address)?;
        let path = format!("Users/{}/Views", path_segment(&connection.user_id));
        let data = self
            .request_json_with_token(&base, &path, connection.provider, &token)
            .await?;
        let items = data
            .get("Items")
            .and_then(Value::as_array)
            .ok_or(IntegrationError::InvalidResponse)?;
        let mut libraries = Vec::new();
        for item in items.iter().take(MAX_LIBRARIES) {
            let Some(id) = value_str(item, "Id") else {
                continue;
            };
            let Some(name) = value_str(item, "Name") else {
                continue;
            };
            let kind = match value_str(item, "CollectionType")
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "movies" | "movie" => LibraryKind::Movies,
                "tvshows" | "tv" | "series" => LibraryKind::Series,
                "mixed" => LibraryKind::Mixed,
                _ => LibraryKind::Other,
            };
            libraries.push(MediaServerLibrary {
                id: id.to_owned(),
                name: name.to_owned(),
                kind,
                selected: true,
            });
        }
        connection.libraries = libraries.clone();
        self.save_connections(profile_id, &connections)?;
        Ok(libraries)
    }

    pub async fn browse_library(
        &self,
        profile_id: &str,
        connection_id: &str,
        library_id: &str,
        start: u32,
        limit: u32,
    ) -> Result<Vec<MediaServerCatalogItem>, IntegrationError> {
        let connection = self
            .connection(profile_id, connection_id)?
            .filter(|c| c.enabled)
            .ok_or(IntegrationError::NotFound)?;
        if !connection
            .libraries
            .iter()
            .any(|lib| lib.id == library_id && lib.selected)
        {
            return Err(IntegrationError::InvalidQuery);
        }
        let token = self.load_token(&connection)?;
        let base = normalize_address(&connection.address)?;
        let mut url = endpoint(&base, "Items")?;
        url.query_pairs_mut()
            .append_pair("ParentId", library_id)
            .append_pair("Recursive", "true")
            .append_pair("IncludeItemTypes", "Movie,Series")
            .append_pair("Fields", "ProviderIds,ProductionYear,Overview")
            .append_pair("StartIndex", &start.min(100_000).to_string())
            .append_pair("Limit", &limit.clamp(1, 100).to_string());
        let data = self
            .request_url_json(&url, connection.provider, &token)
            .await?;
        parse_catalog_items(&data)
    }

    pub async fn search_catalog(
        &self,
        profile_id: &str,
        connection_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<MediaServerCatalogItem>, IntegrationError> {
        let query = query.trim();
        if query.chars().count() < 2
            || query.chars().count() > 120
            || query.chars().any(char::is_control)
        {
            return Err(IntegrationError::InvalidQuery);
        }
        let connection = self
            .connection(profile_id, connection_id)?
            .filter(|c| c.enabled)
            .ok_or(IntegrationError::NotFound)?;
        let token = self.load_token(&connection)?;
        let base = normalize_address(&connection.address)?;
        let mut url = endpoint(&base, "Items")?;
        let libraries = connection
            .libraries
            .iter()
            .filter(|v| v.selected)
            .map(|v| v.id.clone())
            .collect::<Vec<_>>();
        url.query_pairs_mut()
            .append_pair("SearchTerm", query)
            .append_pair("Recursive", "true")
            .append_pair("IncludeItemTypes", "Movie,Series")
            .append_pair("Fields", "ProviderIds,ProductionYear,Overview")
            .append_pair("Limit", &limit.clamp(1, 100).to_string());
        if !libraries.is_empty() {
            url.query_pairs_mut()
                .append_pair("ParentId", &libraries.join(","));
        }
        let data = self
            .request_url_json(&url, connection.provider, &token)
            .await?;
        parse_catalog_items(&data)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn find_and_resolve(
        &self,
        profile_id: &str,
        media_type: &str,
        title: &str,
        year: Option<u16>,
        tmdb_id: Option<&str>,
        imdb_id: Option<&str>,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> Result<Vec<ResolvedServerSource>, IntegrationError> {
        let mut results = Vec::new();
        for connection in self
            .list_connections(profile_id)?
            .into_iter()
            .filter(|c| c.enabled)
        {
            match self
                .find_on_connection(
                    &connection,
                    media_type,
                    title,
                    year,
                    tmdb_id,
                    imdb_id,
                    season,
                    episode,
                )
                .await
            {
                Ok(Some(source)) => results.push(source),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(provider=%connection.provider.id(), connection_id=%connection.id, error=%error, "Media server lookup isolated after failure")
                }
            }
        }
        Ok(results)
    }

    #[allow(clippy::too_many_arguments)]
    async fn find_on_connection(
        &self,
        connection: &MediaServerConnection,
        media_type: &str,
        title: &str,
        year: Option<u16>,
        tmdb_id: Option<&str>,
        imdb_id: Option<&str>,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> Result<Option<ResolvedServerSource>, IntegrationError> {
        let libraries = connection
            .libraries
            .iter()
            .filter(|lib| lib.selected && lib_matches(lib, media_type))
            .collect::<Vec<_>>();
        if libraries.is_empty() {
            return Ok(None);
        }
        let token = self.load_token(connection)?;
        let base = normalize_address(&connection.address)?;
        for library in libraries {
            let mut url = endpoint(&base, "Items")?;
            url.query_pairs_mut()
                .append_pair("ParentId", &library.id)
                .append_pair("Recursive", "true")
                .append_pair(
                    "IncludeItemTypes",
                    if media_type == "movie" {
                        "Movie"
                    } else {
                        "Series"
                    },
                )
                .append_pair("SearchTerm", title)
                .append_pair("Fields", "ProviderIds,ProductionYear,Overview")
                .append_pair("Limit", "60");
            let data = self
                .request_url_json(&url, connection.provider, &token)
                .await?;
            let items = parse_catalog_items(&data)?;
            let matched = items
                .iter()
                .find(|item| exact_ids(item, tmdb_id, imdb_id))
                .or_else(|| items.iter().find(|item| title_matches(item, title, year)));
            let Some(matched) = matched else {
                continue;
            };
            let item_id = if media_type == "movie" {
                matched.id.clone()
            } else {
                let (season, episode) = match (season, episode) {
                    (Some(s), Some(e)) => (s, e),
                    _ => continue,
                };
                self.find_episode_id(
                    &base,
                    connection.provider,
                    &token,
                    &matched.id,
                    season,
                    episode,
                )
                .await?
                .unwrap_or_default()
            };
            if item_id.is_empty() {
                continue;
            }
            let source = self
                .resolve_item(connection, &base, &token, &item_id, &matched.title)
                .await?;
            return Ok(Some(source));
        }
        Ok(None)
    }

    async fn find_episode_id(
        &self,
        base: &Url,
        provider: MediaServerProvider,
        token: &str,
        series_id: &str,
        season: u32,
        episode: u32,
    ) -> Result<Option<String>, IntegrationError> {
        let path = format!("Shows/{}/Episodes", path_segment(series_id));
        let mut url = endpoint(base, &path)?;
        url.query_pairs_mut()
            .append_pair("Season", &season.to_string())
            .append_pair("Fields", "ProviderIds,Overview")
            .append_pair("Limit", "500");
        let data = self.request_url_json(&url, provider, token).await?;
        let Some(items) = data.get("Items").and_then(Value::as_array) else {
            return Err(IntegrationError::InvalidResponse);
        };
        Ok(items
            .iter()
            .find(|item| {
                item.get("IndexNumber").and_then(Value::as_u64) == Some(episode as u64)
                    && item
                        .get("ParentIndexNumber")
                        .and_then(Value::as_u64)
                        .is_none_or(|value| value == season as u64)
            })
            .and_then(|item| value_str(item, "Id"))
            .map(str::to_owned))
    }

    async fn resolve_item(
        &self,
        connection: &MediaServerConnection,
        base: &Url,
        token: &str,
        item_id: &str,
        title: &str,
    ) -> Result<ResolvedServerSource, IntegrationError> {
        let path = format!("Items/{}/PlaybackInfo", path_segment(item_id));
        let mut playback_url = endpoint(base, &path)?;
        playback_url
            .query_pairs_mut()
            .append_pair("UserId", &connection.user_id);
        let response = self
            .request_url_json(&playback_url, connection.provider, token)
            .await?;
        let sources = response
            .get("MediaSources")
            .and_then(Value::as_array)
            .ok_or(IntegrationError::InvalidResponse)?;
        let media = sources
            .iter()
            .find(|item| {
                item.get("SupportsDirectPlay")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            })
            .or_else(|| sources.first())
            .ok_or(IntegrationError::InvalidResponse)?;
        let media_source_id = value_str(media, "Id").unwrap_or(item_id);
        let mut stream_url = endpoint(base, &format!("Videos/{}/stream", path_segment(item_id)))?;
        stream_url
            .query_pairs_mut()
            .append_pair("Static", "true")
            .append_pair("MediaSourceId", media_source_id)
            .append_pair("DeviceId", "com.senzo.desktop");
        if let Some(session) = value_str(&response, "PlaySessionId") {
            stream_url
                .query_pairs_mut()
                .append_pair("PlaySessionId", session);
        }
        let (auth_header_name, auth_header) = server_auth_header(connection.provider, Some(token));
        let mut headers = BTreeMap::new();
        headers.insert(auth_header_name.to_owned(), auth_header);
        let height = media
            .get("MediaStreams")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|stream| value_str(stream, "Type") == Some("Video"))
            .filter_map(|stream| stream.get("Height").and_then(Value::as_u64))
            .max();
        let quality = height.map(|h| {
            if h >= 2160 {
                "2160p"
            } else if h >= 1440 {
                "1440p"
            } else if h >= 1080 {
                "1080p"
            } else if h >= 720 {
                "720p"
            } else {
                "SD"
            }
            .to_owned()
        });
        let size_bytes = media.get("Size").and_then(Value::as_u64);
        let subtitles = media
            .get("MediaStreams")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|stream| {
                value_str(stream, "Type") == Some("Subtitle")
                    && stream.get("IsExternal").and_then(Value::as_bool) == Some(true)
            })
            .filter_map(|stream| {
                let index = stream.get("Index")?.as_u64()?;
                let codec = value_str(stream, "Codec").unwrap_or("srt");
                let url = endpoint(
                    base,
                    &format!(
                        "Videos/{}/{}/Subtitles/{}/Stream.{}",
                        path_segment(item_id),
                        path_segment(media_source_id),
                        index,
                        codec
                    ),
                )
                .ok()?;
                Some(ResolvedServerSubtitle {
                    url: url.to_string(),
                    language: value_str(stream, "Language").unwrap_or("und").to_owned(),
                    name: value_str(stream, "DisplayTitle").map(str::to_owned),
                    headers: headers.clone(),
                })
            })
            .collect();
        Ok(ResolvedServerSource {
            id: format!("{}:{item_id}", connection.id),
            provider_id: connection.provider.id().to_owned(),
            provider_name: format!(
                "{} · {}",
                connection.provider.display_name(),
                connection.name
            ),
            title: title.to_owned(),
            url: stream_url.to_string(),
            headers,
            quality,
            size_bytes,
            subtitles,
        })
    }

    fn load_token(&self, connection: &MediaServerConnection) -> Result<String, IntegrationError> {
        self.credentials
            .get(&connection.credential_ref)
            .map_err(IntegrationError::Credential)?
            .ok_or(IntegrationError::Authentication)
    }

    async fn request_json_with_token(
        &self,
        base: &Url,
        path: &str,
        provider: MediaServerProvider,
        token: &str,
    ) -> Result<Value, IntegrationError> {
        let url = endpoint(base, path)?;
        self.request_url_json(&url, provider, token).await
    }

    async fn request_url_json(
        &self,
        url: &Url,
        provider: MediaServerProvider,
        token: &str,
    ) -> Result<Value, IntegrationError> {
        let (header_name, header_value) = server_auth_header(provider, Some(token));
        let response = self
            .client
            .get(url.clone())
            .header(header_name, header_value)
            .send()
            .await
            .map_err(|e| IntegrationError::Network(e.without_url().to_string()))?;
        read_json(response).await
    }
}

async fn read_json(response: reqwest::Response) -> Result<Value, IntegrationError> {
    let status = response.status();
    if !status.is_success() {
        return Err(
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                IntegrationError::Authentication
            } else {
                IntegrationError::HttpStatus(status.as_u16())
            },
        );
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(IntegrationError::ResponseTooLarge);
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| IntegrationError::Network(e.without_url().to_string()))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(IntegrationError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| IntegrationError::InvalidResponse)
}

fn normalize_address(raw: &str) -> Result<Url, IntegrationError> {
    let mut url = Url::parse(raw.trim()).map_err(|_| IntegrationError::InvalidAddress)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(IntegrationError::InvalidAddress);
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn server_auth_header(
    provider: MediaServerProvider,
    token: Option<&str>,
) -> (&'static str, String) {
    let header_name = match provider {
        MediaServerProvider::Jellyfin => "Authorization",
        MediaServerProvider::Emby => "X-Emby-Authorization",
    };
    let mut value = format!(
        "MediaBrowser Client=\"SENZO\", Device=\"Desktop\", DeviceId=\"com.senzo.desktop\", Version=\"{}\"",
        env!("CARGO_PKG_VERSION"),
    );
    if let Some(token) = token {
        value.push_str(&format!(", Token=\"{token}\""));
    }
    (header_name, value)
}
fn endpoint(base: &Url, path: &str) -> Result<Url, IntegrationError> {
    base.join(path.trim_start_matches('/'))
        .map_err(|_| IntegrationError::InvalidAddress)
}
fn path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}
fn connections_key(profile_id: &str) -> Result<SettingKey, IntegrationError> {
    validate_profile(profile_id)?;
    SettingKey::new(format!("media_servers.{profile_id}"))
        .map_err(|e| IntegrationError::Persistence(e.to_string()))
}
fn validate_profile(value: &str) -> Result<(), IntegrationError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
    {
        return Err(IntegrationError::InvalidQuery);
    }
    Ok(())
}
fn value_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
}
fn parse_catalog_items(payload: &Value) -> Result<Vec<MediaServerCatalogItem>, IntegrationError> {
    let items = payload
        .get("Items")
        .and_then(Value::as_array)
        .ok_or(IntegrationError::InvalidResponse)?;
    Ok(items
        .iter()
        .take(MAX_CATALOG_ITEMS)
        .filter_map(|item| {
            let id = value_str(item, "Id")?.to_owned();
            let title = value_str(item, "Name")?.to_owned();
            let media_type = match value_str(item, "Type")? {
                "Movie" => "movie",
                "Series" => "tv",
                _ => return None,
            }
            .to_owned();
            let ids = item.get("ProviderIds");
            let year = item
                .get("ProductionYear")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok());
            Some(MediaServerCatalogItem {
                id,
                title,
                media_type,
                year,
                tmdb_id: ids.and_then(|v| value_str(v, "Tmdb")).map(str::to_owned),
                imdb_id: ids.and_then(|v| value_str(v, "Imdb")).map(str::to_owned),
                overview: value_str(item, "Overview").map(str::to_owned),
            })
        })
        .collect())
}
fn lib_matches(library: &MediaServerLibrary, media_type: &str) -> bool {
    matches!(library.kind, LibraryKind::Mixed)
        || (media_type == "movie" && library.kind == LibraryKind::Movies)
        || (media_type != "movie" && library.kind == LibraryKind::Series)
}
fn exact_ids(item: &MediaServerCatalogItem, tmdb_id: Option<&str>, imdb_id: Option<&str>) -> bool {
    tmdb_id.is_some_and(|id| item.tmdb_id.as_deref() == Some(id))
        || imdb_id.is_some_and(|id| {
            item.imdb_id
                .as_deref()
                .is_some_and(|v| v.eq_ignore_ascii_case(id))
        })
}
fn normalized_title(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn title_matches(item: &MediaServerCatalogItem, title: &str, year: Option<u16>) -> bool {
    normalized_title(&item.title) == normalized_title(title)
        && year.is_none_or(|wanted| item.year.is_none_or(|found| wanted.abs_diff(found) <= 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_user_configured_private_http_server_urls_but_rejects_credentials_and_fragments() {
        assert!(normalize_address("http://192.168.1.20:8096/").is_ok());
        assert!(normalize_address("https://media.example/emby/").is_ok());
        assert!(normalize_address("https://user:pass@media.example/").is_err());
        assert!(normalize_address("https://media.example/#frag").is_err());
    }
    #[test]
    fn provider_ids_are_explicit_and_case_insensitive() {
        assert_eq!(
            MediaServerProvider::parse("Jellyfin").unwrap().id(),
            "jellyfin"
        );
        assert!(MediaServerProvider::parse("plex").is_err());
    }
    #[test]
    fn title_matching_requires_exact_normalized_title_and_nearby_year() {
        let item = MediaServerCatalogItem {
            id: "1".into(),
            title: "The Matrix".into(),
            media_type: "movie".into(),
            year: Some(1999),
            tmdb_id: None,
            imdb_id: None,
            overview: None,
        };
        assert!(title_matches(&item, "the-matrix", Some(2000)));
        assert!(!title_matches(&item, "Matrix Reloaded", Some(2000)));
        assert!(!title_matches(&item, "The Matrix", Some(2010)));
    }

    #[test]
    fn jellyfin_uses_current_authorization_header_with_access_token() {
        let (name, value) = server_auth_header(MediaServerProvider::Jellyfin, Some("test-token"));
        assert_eq!(name, "Authorization");
        assert!(value.starts_with("MediaBrowser Client="));
        assert!(value.contains("Token=\"test-token\""));
    }

    #[test]
    fn emby_keeps_its_compatibility_authorization_header() {
        let (name, value) = server_auth_header(MediaServerProvider::Emby, Some("test-token"));
        assert_eq!(name, "X-Emby-Authorization");
        assert!(value.starts_with("MediaBrowser Client="));
        assert!(value.contains("Token=\"test-token\""));
    }

    #[test]
    fn connection_key_is_profile_scoped() {
        assert!(connections_key("default").is_ok());
        assert!(connections_key("../other").is_err());
    }
}
