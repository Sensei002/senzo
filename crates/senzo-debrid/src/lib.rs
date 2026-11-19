//! Optional Torbox, Premiumize and Real-Debrid integration.
//! Credentials are held only by the operating-system credential-store adapter.

use std::{collections::BTreeMap, time::Duration};

use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(18);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_POLL_ATTEMPTS: usize = 8;
const MAX_MAGNET_BYTES: usize = 8192;

pub trait DebridCredentialStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

#[derive(Debug, Error)]
pub enum DebridError {
    #[error("debrid provider is not supported")]
    UnsupportedProvider,
    #[error("debrid credential is missing or invalid")]
    Authentication,
    #[error("debrid API returned HTTP {0}")]
    HttpStatus(u16),
    #[error("debrid API response was invalid")]
    InvalidResponse,
    #[error("debrid request failed: {0}")]
    Network(String),
    #[error("debrid response exceeded its safety limit")]
    ResponseTooLarge,
    #[error("torrent magnet URI is invalid")]
    InvalidMagnet,
    #[error("torrent is not available in the provider cache")]
    NotCached,
    #[error("no matching video file was found in the torrent")]
    FileNotFound,
    #[error("debrid operation timed out")]
    Timeout,
    #[error("credential storage failed: {0}")]
    Credential(String),
    #[error("profile identifier is invalid")]
    InvalidProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebridProviderId { Torbox, Premiumize, RealDebrid }

impl DebridProviderId {
    pub const ALL: [Self; 3] = [Self::Torbox, Self::Premiumize, Self::RealDebrid];
    pub const fn id(self) -> &'static str {
        match self { Self::Torbox => "torbox", Self::Premiumize => "premiumize", Self::RealDebrid => "realdebrid" }
    }
    pub const fn name(self) -> &'static str {
        match self { Self::Torbox => "Torbox", Self::Premiumize => "Premiumize", Self::RealDebrid => "Real-Debrid" }
    }
    pub const fn capabilities(self) -> &'static [DebridCapability] {
        match self {
            Self::Torbox => &[DebridCapability::ApiKey, DebridCapability::TorrentCacheCheck, DebridCapability::MagnetResolve, DebridCapability::CachedOnlyAdd],
            Self::Premiumize => &[DebridCapability::ApiKey, DebridCapability::TorrentCacheCheck, DebridCapability::MagnetResolve, DebridCapability::DirectDownload],
            Self::RealDebrid => &[DebridCapability::ApiKey, DebridCapability::MagnetResolve, DebridCapability::FileSelection, DebridCapability::UnrestrictLinks],
        }
    }
    pub fn parse(value: &str) -> Result<Self, DebridError> {
        match value.trim().to_ascii_lowercase().replace('-', "").as_str() {
            "torbox" => Ok(Self::Torbox), "premiumize" => Ok(Self::Premiumize),
            "realdebrid" => Ok(Self::RealDebrid), _ => Err(DebridError::UnsupportedProvider),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebridCapability { ApiKey, TorrentCacheCheck, MagnetResolve, CachedOnlyAdd, DirectDownload, FileSelection, UnrestrictLinks }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheAvailability { Cached, NotCached, Unknown }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebridAccount {
    pub provider: DebridProviderId,
    pub configured: bool,
    pub masked_credential: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDebridSource {
    pub provider: DebridProviderId,
    pub title: String,
    pub url: String,
    pub size_bytes: Option<u64>,
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone)]
pub struct DebridManager {
    credentials: std::sync::Arc<dyn DebridCredentialStore>,
    client: Client,
}

impl DebridManager {
    pub fn new(credentials: std::sync::Arc<dyn DebridCredentialStore>) -> Result<Self, DebridError> {
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(7))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
            .build().map_err(|e| DebridError::Network(e.without_url().to_string()))?;
        Ok(Self { credentials, client })
    }

    pub fn accounts(&self, profile_id: &str) -> Result<Vec<DebridAccount>, DebridError> {
        validate_profile(profile_id)?;
        DebridProviderId::ALL.into_iter().map(|provider| {
            let secret = self.credentials.get(&credential_key(profile_id, provider)).map_err(DebridError::Credential)?;
            Ok(DebridAccount {
                provider,
                configured: secret.is_some(),
                masked_credential: secret.map(|value| mask_secret(&value)),
            })
        }).collect()
    }

    pub async fn connect(&self, profile_id: &str, provider: DebridProviderId, api_key: &str) -> Result<DebridAccount, DebridError> {
        validate_profile(profile_id)?;
        let token = api_key.trim();
        if token.len() < 8 || token.len() > 4096 || token.chars().any(char::is_control) {
            return Err(DebridError::Authentication);
        }
        if !self.validate_token(provider, token).await? { return Err(DebridError::Authentication); }
        self.credentials.set(&credential_key(profile_id, provider), token).map_err(DebridError::Credential)?;
        Ok(DebridAccount { provider, configured: true, masked_credential: Some(mask_secret(token)) })
    }

    pub fn disconnect(&self, profile_id: &str, provider: DebridProviderId) -> Result<(), DebridError> {
        validate_profile(profile_id)?;
        self.credentials.delete(&credential_key(profile_id, provider)).map_err(DebridError::Credential)
    }

    pub async fn validate_token(&self, provider: DebridProviderId, token: &str) -> Result<bool, DebridError> {
        let url = match provider {
            DebridProviderId::Torbox => "https://api.torbox.app/v1/api/user/me",
            DebridProviderId::Premiumize => "https://www.premiumize.me/api/account/info",
            DebridProviderId::RealDebrid => "https://api.real-debrid.com/rest/1.0/user",
        };
        let payload = self.request_json(Method::GET, url, Some(token), None).await?;
        Ok(match provider {
            DebridProviderId::Torbox => payload.get("success").and_then(Value::as_bool).unwrap_or(true),
            DebridProviderId::Premiumize => payload.get("status").and_then(Value::as_str) == Some("success"),
            DebridProviderId::RealDebrid => payload.get("id").is_some(),
        })
    }

    pub async fn check_cached(&self, profile_id: &str, provider: DebridProviderId, info_hash: &str) -> Result<CacheAvailability, DebridError> {
        let hash = normalize_hash(info_hash).ok_or(DebridError::InvalidMagnet)?;
        let token = self.load_token(profile_id, provider)?;
        match provider {
            DebridProviderId::Torbox => {
                let payload = self.request_json(Method::POST, "https://api.torbox.app/v1/api/torrents/checkcached?format=object",
                    Some(&token), Some(json!({"hashes":[hash]}))).await?;
                let cached = payload.pointer(&format!("/data/{hash}")).is_some_and(is_nonempty);
                Ok(if cached { CacheAvailability::Cached } else { CacheAvailability::NotCached })
            }
            DebridProviderId::Premiumize => {
                let response = self.client.post("https://www.premiumize.me/api/cache/check")
                    .bearer_auth(&token).form(&[("items[]", hash.as_str())])
                    .send().await.map_err(|e| DebridError::Network(e.without_url().to_string()))?;
                let payload = read_json(response).await?;
                let cached = payload.get("response").and_then(Value::as_array)
                    .and_then(|items| items.first()).and_then(Value::as_bool).unwrap_or(false);
                Ok(if cached { CacheAvailability::Cached } else { CacheAvailability::NotCached })
            }
            DebridProviderId::RealDebrid => Ok(CacheAvailability::Unknown),
        }
    }

    pub async fn resolve_magnet(
        &self, profile_id: &str, provider: DebridProviderId, magnet: &str,
        filename_hint: Option<&str>, season: Option<u32>, episode: Option<u32>,
    ) -> Result<ResolvedDebridSource, DebridError> {
        let (hash, normalized) = validate_magnet(magnet)?;
        let token = self.load_token(profile_id, provider)?;
        match provider {
            DebridProviderId::Torbox => self.resolve_torbox(&token, &hash, &normalized, filename_hint, season, episode).await,
            DebridProviderId::Premiumize => self.resolve_premiumize(&token, &normalized, filename_hint, season, episode).await,
            DebridProviderId::RealDebrid => self.resolve_real_debrid(&token, &normalized, filename_hint, season, episode).await,
        }
    }

    async fn resolve_torbox(&self, token: &str, hash: &str, magnet: &str, filename: Option<&str>, season: Option<u32>, episode: Option<u32>) -> Result<ResolvedDebridSource, DebridError> {
        if self.check_cached_with_token(DebridProviderId::Torbox, token, hash).await? != CacheAvailability::Cached {
            return Err(DebridError::NotCached);
        }
        let response = self.client.post("https://api.torbox.app/v1/api/torrents/createtorrent")
            .bearer_auth(token)
            .multipart(reqwest::multipart::Form::new().text("magnet", magnet.to_owned())
                .text("add_only_if_cached", "true").text("allow_zip", "false"))
            .send().await.map_err(|e| DebridError::Network(e.without_url().to_string()))?;
        let created = read_json(response).await?;
        if created.get("success").and_then(Value::as_bool) == Some(false) { return Err(DebridError::NotCached); }
        let id = created.pointer("/data/torrent_id").or_else(|| created.pointer("/data/id"))
            .and_then(Value::as_u64).ok_or(DebridError::InvalidResponse)?;
        for _ in 0..MAX_POLL_ATTEMPTS {
            let mut url = Url::parse("https://api.torbox.app/v1/api/torrents/mylist").map_err(|_| DebridError::InvalidResponse)?;
            url.query_pairs_mut().append_pair("id", &id.to_string()).append_pair("bypass_cache", "true");
            let payload = self.request_json(Method::GET, url.as_str(), Some(token), None).await?;
            let item = payload.get("data").and_then(Value::as_array).and_then(|v| v.first())
                .or_else(|| payload.get("data").filter(|v| v.is_object())).ok_or(DebridError::InvalidResponse)?;
            let files = item.get("files").and_then(Value::as_array).ok_or(DebridError::InvalidResponse)?;
            if let Some(file) = select_video_file(files, filename, season, episode, |f| {
                value_string(f, "name").or_else(|| value_string(f, "short_name")).or_else(|| value_string(f, "absolute_path"))
            }) {
                let file_id = file.get("id").and_then(Value::as_u64).ok_or(DebridError::InvalidResponse)?;
                let mut url = Url::parse("https://api.torbox.app/v1/api/torrents/requestdl").map_err(|_| DebridError::InvalidResponse)?;
                url.query_pairs_mut().append_pair("token", token).append_pair("torrent_id", &id.to_string())
                    .append_pair("file_id", &file_id.to_string());
                let response = self.client.get(url).bearer_auth(token).send().await
                    .map_err(|e| DebridError::Network(e.without_url().to_string()))?;
                let payload = read_json(response).await?;
                let link = payload.get("data").and_then(Value::as_str).or_else(|| value_string(&payload, "url"))
                    .or_else(|| value_string(&payload, "download")).filter(|v| valid_http_url(v))
                    .ok_or(DebridError::InvalidResponse)?;
                return Ok(ResolvedDebridSource { provider: DebridProviderId::Torbox,
                    title: value_string(file, "name").unwrap_or("Torbox stream").to_owned(),
                    url: link.to_owned(), size_bytes: file.get("size").and_then(Value::as_u64), headers: BTreeMap::new() });
            }
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        Err(DebridError::Timeout)
    }

    async fn check_cached_with_token(&self, provider: DebridProviderId, token: &str, hash: &str) -> Result<CacheAvailability, DebridError> {
        if provider != DebridProviderId::Torbox { return Ok(CacheAvailability::Unknown); }
        let payload = self.request_json(Method::POST, "https://api.torbox.app/v1/api/torrents/checkcached?format=object",
            Some(token), Some(json!({"hashes":[hash]}))).await?;
        Ok(if payload.pointer(&format!("/data/{hash}")).is_some_and(is_nonempty) { CacheAvailability::Cached } else { CacheAvailability::NotCached })
    }

    async fn resolve_premiumize(&self, token: &str, magnet: &str, filename: Option<&str>, season: Option<u32>, episode: Option<u32>) -> Result<ResolvedDebridSource, DebridError> {
        let response = self.client.post("https://www.premiumize.me/api/transfer/directdl")
            .bearer_auth(token).form(&[("src", magnet)])
            .send().await.map_err(|e| DebridError::Network(e.without_url().to_string()))?;
        let payload = read_json(response).await?;
        if payload.get("status").and_then(Value::as_str) != Some("success") { return Err(DebridError::NotCached); }
        let files = payload.get("content").and_then(Value::as_array).ok_or(DebridError::InvalidResponse)?;
        let file = select_video_file(files, filename, season, episode, |f| value_string(f, "path")).ok_or(DebridError::FileNotFound)?;
        let url = value_string(file, "link").filter(|v| valid_http_url(v)).ok_or(DebridError::InvalidResponse)?;
        Ok(ResolvedDebridSource { provider: DebridProviderId::Premiumize,
            title: value_string(file, "path").unwrap_or("Premiumize stream").to_owned(), url: url.to_owned(),
            size_bytes: file.get("size").and_then(Value::as_u64), headers: BTreeMap::new() })
    }

    async fn resolve_real_debrid(&self, token: &str, magnet: &str, filename: Option<&str>, season: Option<u32>, episode: Option<u32>) -> Result<ResolvedDebridSource, DebridError> {
        let response = self.client.post("https://api.real-debrid.com/rest/1.0/torrents/addMagnet")
            .bearer_auth(token).form(&[("magnet", magnet)])
            .send().await.map_err(|e| DebridError::Network(e.without_url().to_string()))?;
        let added = read_json(response).await?;
        let id = value_string(&added, "id").ok_or(DebridError::InvalidResponse)?;
        let selected = self.client.post(format!("https://api.real-debrid.com/rest/1.0/torrents/selectFiles/{}", path_segment(id)))
            .bearer_auth(token).form(&[("files", "all")])
            .send().await.map_err(|e| DebridError::Network(e.without_url().to_string()))?;
        if !selected.status().is_success() { return Err(DebridError::HttpStatus(selected.status().as_u16())); }
        for _ in 0..MAX_POLL_ATTEMPTS {
            let url = format!("https://api.real-debrid.com/rest/1.0/torrents/info/{}", path_segment(id));
            let info = self.request_json(Method::GET, &url, Some(token), None).await?;
            let files = info.get("files").and_then(Value::as_array).ok_or(DebridError::InvalidResponse)?;
            let file = select_video_file(files, filename, season, episode, |f| value_string(f, "path")).ok_or(DebridError::FileNotFound)?;
            if let Some(links) = info.get("links").and_then(Value::as_array) {
                let link = links.get(file.get("id").and_then(Value::as_u64).unwrap_or(0) as usize)
                    .or_else(|| links.first()).and_then(Value::as_str).ok_or(DebridError::InvalidResponse)?;
                let response = self.client.post("https://api.real-debrid.com/rest/1.0/unrestrict/link")
                    .bearer_auth(token).form(&[("link", link)]).send().await
                    .map_err(|e| DebridError::Network(e.without_url().to_string()))?;
                let resolved = read_json(response).await?;
                let url = value_string(&resolved, "download").filter(|v| valid_http_url(v)).ok_or(DebridError::InvalidResponse)?;
                return Ok(ResolvedDebridSource { provider: DebridProviderId::RealDebrid,
                    title: value_string(&resolved, "filename").unwrap_or("Real-Debrid stream").to_owned(),
                    url: url.to_owned(), size_bytes: resolved.get("filesize").and_then(Value::as_u64), headers: BTreeMap::new() });
            }
            if matches!(value_string(&info, "status"), Some("error" | "magnet_error")) { return Err(DebridError::FileNotFound); }
            tokio::time::sleep(Duration::from_millis(900)).await;
        }
        Err(DebridError::Timeout)
    }

    fn load_token(&self, profile_id: &str, provider: DebridProviderId) -> Result<String, DebridError> {
        validate_profile(profile_id)?;
        self.credentials.get(&credential_key(profile_id, provider)).map_err(DebridError::Credential)?.ok_or(DebridError::Authentication)
    }

    async fn request_json(&self, method: Method, url: &str, token: Option<&str>, body: Option<Value>) -> Result<Value, DebridError> {
        let mut request = self.client.request(method, url);
        if let Some(token) = token { request = request.bearer_auth(token); }
        if let Some(body) = body { request = request.json(&body); }
        let response = request.send().await.map_err(|e| DebridError::Network(e.without_url().to_string()))?;
        read_json(response).await
    }
}

async fn read_json(response: reqwest::Response) -> Result<Value, DebridError> {
    let status = response.status();
    if !status.is_success() {
        return Err(if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN { DebridError::Authentication } else { DebridError::HttpStatus(status.as_u16()) });
    }
    if response.content_length().is_some_and(|n| n > MAX_RESPONSE_BYTES as u64) { return Err(DebridError::ResponseTooLarge); }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| DebridError::Network(e.without_url().to_string()))? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES { return Err(DebridError::ResponseTooLarge); }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() { return Ok(Value::Null); }
    serde_json::from_slice(&bytes).map_err(|_| DebridError::InvalidResponse)
}

fn validate_magnet(raw: &str) -> Result<(String, String), DebridError> {
    if raw.len() > MAX_MAGNET_BYTES { return Err(DebridError::InvalidMagnet); }
    let url = Url::parse(raw).map_err(|_| DebridError::InvalidMagnet)?;
    if url.scheme() != "magnet" || url.host_str().is_some() { return Err(DebridError::InvalidMagnet); }
    let hash = url.query_pairs().find(|(key, value)| key == "xt" && value.to_ascii_lowercase().starts_with("urn:btih:"))
        .and_then(|(_, value)| normalize_hash(value.trim_start_matches("urn:btih:"))).ok_or(DebridError::InvalidMagnet)?;
    Ok((hash, url.to_string()))
}
fn normalize_hash(raw: &str) -> Option<String> {
    let value = raw.trim().to_ascii_lowercase();
    (value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())).then_some(value)
}
fn valid_http_url(value: &str) -> bool {
    Url::parse(value).ok().is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some() && u.username().is_empty() && u.password().is_none())
}
fn value_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> { value.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()) }
fn path_segment(value: &str) -> String { url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>().replace('+', "%20") }
fn credential_key(profile: &str, provider: DebridProviderId) -> String { format!("senzo:debrid:{profile}:{}", provider.id()) }
fn validate_profile(value: &str) -> Result<(), DebridError> {
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b)) {
        Err(DebridError::InvalidProfile)
    } else { Ok(()) }
}
fn mask_secret(secret: &str) -> String { format!("••••{}", secret.chars().rev().take(4).collect::<String>().chars().rev().collect::<String>()) }
fn is_nonempty(value: &Value) -> bool {
    match value { Value::Null => false, Value::Array(v) => !v.is_empty(), Value::Object(v) => !v.is_empty(), Value::Bool(v) => *v, _ => true }
}

fn select_video_file<'a, F>(files: &'a [Value], filename: Option<&str>, season: Option<u32>, episode: Option<u32>, name: F) -> Option<&'a Value>
where F: Fn(&'a Value) -> Option<&'a str> {
    let videos = files.iter().filter(|file| name(file).is_some_and(is_video_name)).collect::<Vec<_>>();
    if videos.is_empty() { return None; }
    if let Some(hint) = filename.map(normalize_file_name)
        && let Some(file) = videos.iter().find(|file| name(file).is_some_and(|n| normalize_file_name(n).contains(&hint)))
    {
        return Some(*file);
    }
    if let (Some(season), Some(episode)) = (season, episode) {
        let sxe = format!("s{season:02}e{episode:02}");
        let x = format!("{season}x{episode:02}");
        if let Some(file) = videos.iter().find(|file| name(file).is_some_and(|n| { let n = normalize_file_name(n); n.contains(&sxe) || n.contains(&x) })) { return Some(*file); }
    }
    videos.into_iter().next()
}
fn normalize_file_name(value: &str) -> String { value.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect() }
fn is_video_name(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [".mkv", ".mp4", ".m4v", ".avi", ".mov", ".webm", ".ts", ".m2ts"].iter().any(|suffix| value.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_only_hex_infohash_magnets() {
        assert!(validate_magnet("magnet:?xt=urn:btih:0123456789012345678901234567890123456789").is_ok());
        assert!(validate_magnet("file:///etc/passwd").is_err());
        assert!(validate_magnet("magnet:?xt=urn:btih:invalid").is_err());
    }

    #[test]
    fn exposes_provider_capabilities_and_normalizes_names() {
        assert!(DebridProviderId::Torbox.capabilities().contains(&DebridCapability::CachedOnlyAdd));
        assert!(DebridProviderId::Premiumize.capabilities().contains(&DebridCapability::DirectDownload));
        assert!(DebridProviderId::RealDebrid.capabilities().contains(&DebridCapability::UnrestrictLinks));
        assert_eq!(DebridProviderId::parse("Real-Debrid").unwrap(), DebridProviderId::RealDebrid);
    }

    #[test]
    fn chooses_episode_file_before_an_unrelated_video() {
        let files = vec![json!({"path":"movie.mkv"}), json!({"path":"Show.S02E03.1080p.mkv"})];
        let selected = select_video_file(&files, None, Some(2), Some(3), |v| value_string(v, "path"));
        assert_eq!(value_string(selected.unwrap(), "path"), Some("Show.S02E03.1080p.mkv"));
    }

    #[test]
    fn credential_namespace_is_profile_scoped() {
        assert_ne!(credential_key("default", DebridProviderId::Torbox), credential_key("work", DebridProviderId::Torbox));
        assert!(validate_profile("../root").is_err());
    }
}
