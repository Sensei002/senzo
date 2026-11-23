//! User-supplied authorized providers: M3U playlists and JSON feeds.
//!
//! No remote JavaScript, plugin manifests, or executable content is ever run.
//! A custom source is only a URL to a declarative document plus a small,
//! documented schema.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::direct_sources::{
    matching::{
        format_from_url, identity_matches, is_playable_format, parse_episode, parse_language,
        parse_quality, titles_match,
    },
    net::{NetError, SafeHttp},
    types::{CustomSourceConfig, CustomSourceKind, DirectRequest, DirectStream},
};

const MAX_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESULTS: usize = 24;
const MAX_STREAM_VALIDATIONS: usize = 10;

pub async fn resolve(
    http: &SafeHttp,
    request: &DirectRequest,
    config: &CustomSourceConfig,
) -> Result<Vec<DirectStream>, NetError> {
    match config.kind {
        CustomSourceKind::M3u => resolve_m3u(http, request, config).await,
        CustomSourceKind::JsonFeed => resolve_json(http, request, config).await,
    }
}

pub async fn validate(http: &SafeHttp, config: &CustomSourceConfig) -> Result<String, NetError> {
    let fetched = http.get_text(&config.url, MAX_PLAYLIST_BYTES).await?;
    match config.kind {
        CustomSourceKind::M3u => {
            let entries = parse_m3u(&fetched.body, &fetched.final_url);
            if entries.is_empty() {
                return Err(NetError::ContentType);
            }
            Ok(format!("{} playlist entry(s)", entries.len()))
        }
        CustomSourceKind::JsonFeed => {
            let feed = parse_json_feed(&fetched.body)?;
            if feed.is_empty() {
                return Err(NetError::ContentType);
            }
            Ok(format!("{} feed item(s)", feed.len()))
        }
    }
}

async fn resolve_m3u(
    http: &SafeHttp,
    request: &DirectRequest,
    config: &CustomSourceConfig,
) -> Result<Vec<DirectStream>, NetError> {
    let fetched = http.get_text(&config.url, MAX_PLAYLIST_BYTES).await?;
    let entries = parse_m3u(&fetched.body, &fetched.final_url);
    let mut streams = Vec::new();
    for entry in entries {
        if streams.len() >= MAX_RESULTS {
            break;
        }
        let Some((season, episode)) = select_entry_identity(&entry, request) else {
            continue;
        };
        let format = format_from_url(&entry.url);
        if !is_playable_format(&format) {
            continue;
        }
        if !content_type_allowed(config, request) {
            continue;
        }
        let mut stream = DirectStream::new(
            &config.id,
            &config.name,
            request,
            entry.url.clone(),
            &format,
        );
        stream.title = entry.title.clone();
        stream.season = season;
        stream.episode = episode;
        stream.quality = parse_quality(&format!("{} {}", entry.title, entry.group.as_deref().unwrap_or("")));
        stream.language = entry
            .language
            .as_deref()
            .map(parse_language)
            .unwrap_or_else(|| "unknown".to_owned());
        stream.audio = entry.audio.clone().unwrap_or_else(|| "unknown".to_owned());
        stream.headers = entry.headers.clone();
        stream
            .metadata
            .insert("playlist".to_owned(), "M3U".to_owned());
        if let Some(group) = entry.group.as_deref().filter(|group| !group.is_empty()) {
            stream.metadata.insert("group".to_owned(), group.to_owned());
        }
        if let Some(logo) = entry.logo.as_deref().filter(|logo| !logo.is_empty()) {
            stream
                .metadata
                .insert("provider-logo".to_owned(), logo.to_owned());
        }
        streams.push(stream);
    }

    validate_stream_urls(http, streams).await
}

async fn resolve_json(
    http: &SafeHttp,
    request: &DirectRequest,
    config: &CustomSourceConfig,
) -> Result<Vec<DirectStream>, NetError> {
    let fetched = http.get_text(&config.url, MAX_PLAYLIST_BYTES).await?;
    let items = parse_json_feed(&fetched.body)?;
    let mut streams = Vec::new();
    for item in items {
        if streams.len() >= MAX_RESULTS {
            break;
        }
        let Some(url) = item.url.as_deref() else {
            continue;
        };
        if !item_matches(&item, request) {
            continue;
        }
        let format = item
            .format
            .as_deref()
            .map(normalize_format)
            .unwrap_or_else(|| format_from_url(url));
        if !is_playable_format(&format) {
            continue;
        }
        if !content_type_allowed(config, request) {
            continue;
        }
        let mut stream =
            DirectStream::new(&config.id, &config.name, request, url.to_owned(), &format);
        stream.title = item
            .title
            .clone()
            .or_else(|| item.name.clone())
            .unwrap_or_else(|| request.title.clone());
        stream.quality = item
            .quality
            .as_deref()
            .map(parse_quality)
            .unwrap_or_else(|| parse_quality(&stream.title));
        stream.language = item
            .language
            .as_deref()
            .map(parse_language)
            .unwrap_or_else(|| "unknown".to_owned());
        stream.audio = item.audio.clone().unwrap_or_else(|| "unknown".to_owned());
        stream.season = item.season.or(request.season);
        stream.episode = item.episode.or(request.episode);
        stream.expires_at = item.expires_at;
        stream.headers = sanitize_headers(item.headers.unwrap_or_default());
        stream.subtitles = item
            .subtitles
            .unwrap_or_default()
            .into_iter()
            .filter(|url| SafeHttp::validate_url(url).is_ok())
            .take(8)
            .collect();
        if let Some(metadata) = item.metadata {
            for (key, value) in metadata {
                if stream.metadata.len() >= 12 {
                    break;
                }
                if let Some(value) = value_to_string(&value) {
                    stream.metadata.insert(sanitize_key(&key), value);
                }
            }
        }
        streams.push(stream);
    }
    validate_stream_urls(http, streams).await
}

/// Validate only the matched stream URLs. Provider catalogs can contain
/// thousands of entries; resolving DNS for all of them would be slow and
/// unnecessary. We stop at a small bounded set that can actually be offered.
async fn validate_stream_urls(
    http: &SafeHttp,
    streams: Vec<DirectStream>,
) -> Result<Vec<DirectStream>, NetError> {
    let mut accepted = Vec::new();
    for stream in streams.into_iter().take(MAX_STREAM_VALIDATIONS) {
        if SafeHttp::validate_public_url(&stream.url).await.is_ok() {
            accepted.push(stream);
        }
    }
    Ok(accepted)
}

fn content_type_allowed(config: &CustomSourceConfig, request: &DirectRequest) -> bool {
    if config.content_types.is_empty() {
        return true;
    }
    config
        .content_types
        .iter()
        .any(|kind| kind.eq_ignore_ascii_case(&request.media_type))
}

fn select_entry_identity(
    entry: &M3uEntry,
    request: &DirectRequest,
) -> Option<(Option<i32>, Option<i32>)> {
    let entry_episode =
        parse_episode(&entry.title).or_else(|| entry.name.as_deref().and_then(parse_episode));
    if !identity_matches(entry.id.as_deref(), &entry.display_title(), None, request) {
        return None;
    }
    if request.media_type == "series" {
        let (wanted_season, wanted_episode) = (request.season?, request.episode?);
        let (season, episode) = entry_episode?;
        if season != wanted_season || episode != wanted_episode {
            return None;
        }
        return Some((Some(season), Some(episode)));
    }
    if entry_episode.is_some() {
        return None;
    }
    Some((None, None))
}

fn item_matches(item: &JsonItem, request: &DirectRequest) -> bool {
    let item_type = item
        .media_type
        .as_deref()
        .or(item.r#type.as_deref())
        .unwrap_or_default();
    if !item_type.is_empty() && !item_type.eq_ignore_ascii_case(&request.media_type) {
        return false;
    }

    let item_id = item.id.as_ref().and_then(value_to_string);
    let item_imdb = item_id
        .as_deref()
        .filter(|id| id.trim().to_ascii_lowercase().starts_with("tt"));
    let item_title = item
        .title
        .as_deref()
        .or(item.name.as_deref())
        .unwrap_or_default();
    let item_year = item.year;

    if let Some(imdb) = request.imdb_id.as_deref()
        && let Some(candidate) = item_imdb
        && !candidate.eq_ignore_ascii_case(imdb)
    {
        return false;
    }
    if item_id.as_deref() == Some(request.meta_id.as_str()) {
        // Exact meta-id match is authoritative for series and movies.
    } else if !titles_match(item_title, request) {
        return false;
    }
    if !crate::direct_sources::matching::years_match(item_year, request.year) {
        return false;
    }
    if request.media_type == "series" {
        let (Some(wanted_season), Some(wanted_episode)) = (request.season, request.episode) else {
            return false;
        };
        if item.season != Some(wanted_season) || item.episode != Some(wanted_episode) {
            return false;
        }
    }
    true
}

fn normalize_format(format: &str) -> String {
    match format.trim().to_ascii_lowercase().as_str() {
        "hls" | "m3u8" | "application/vnd.apple.mpegurl" => "m3u8".to_owned(),
        "dash" | "mpd" => "mpd".to_owned(),
        "mp4" | "m4v" => "mp4".to_owned(),
        "webm" | "mkv" | "ogv" | "mov" => format.trim().to_ascii_lowercase(),
        _ => format_from_url(format),
    }
}

fn sanitize_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut sanitized = BTreeMap::new();
    for (name, value) in headers {
        let name = name.trim().to_owned();
        let lower = name.to_ascii_lowercase();
        // Host/Content-Length would let a feed change request framing. Header
        // values must not contain CR/LF, which prevents response splitting.
        if name.is_empty()
            || matches!(
                lower.as_str(),
                "host" | "content-length" | "transfer-encoding" | "connection"
            )
            || value.contains('\r')
            || value.contains('\n')
            || value.len() > 2048
        {
            continue;
        }
        sanitized.insert(name, value);
    }
    sanitized.sort();
    sanitized
}

fn sanitize_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(40)
        .collect()
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
struct M3uEntry {
    id: Option<String>,
    name: Option<String>,
    title: String,
    group: Option<String>,
    language: Option<String>,
    logo: Option<String>,
    audio: Option<String>,
    headers: BTreeMap<String, String>,
    url: String,
}

impl M3uEntry {
    fn display_title(&self) -> String {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.title)
            .to_owned()
    }
}

/// Parse the subset of M3U/M3U8 that describes an authorized provider catalog.
/// Media segment playlists (`#EXTINF` followed by `#EXT-X-...` segments) still
/// produce a single entry, which is valid: it is an HLS stream URL.
fn parse_m3u(body: &str, base: &reqwest::Url) -> Vec<M3uEntry> {
    let mut entries = Vec::new();
    let mut pending: Option<M3uEntry> = None;
    let mut pending_options = BTreeMap::new();

    for raw_line in body.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (attributes, title) = rest.split_once(',').unwrap_or((rest, ""));
            let attributes = parse_attributes(attributes);
            let mut entry = M3uEntry {
                id: attributes.get("tvg-id").cloned(),
                name: attributes.get("tvg-name").cloned(),
                title: title.trim().to_owned(),
                group: attributes.get("group-title").cloned(),
                language: attributes.get("tvg-language").cloned(),
                logo: attributes.get("tvg-logo").cloned(),
                audio: attributes.get("tvg-audio").cloned(),
                headers: BTreeMap::new(),
                url: String::new(),
            };
            if let Some(name) = entry.name.as_deref().filter(|name| !name.is_empty())
                && entry.title.is_empty()
            {
                entry.title = name.to_owned();
            }
            pending = Some(entry);
            pending_options.clear();
            continue;
        }
        if let Some(option) = line.strip_prefix("#EXTVLCOPT:") {
            if let Some((key, value)) = option.split_once('=') {
                let key = key.trim().to_ascii_lowercase();
                let value = value.trim().to_owned();
                match key.as_str() {
                    "http-referrer" | "http-referer" => {
                        pending_options.insert("Referer".to_owned(), value);
                    }
                    "http-user-agent" => {
                        pending_options.insert("User-Agent".to_owned(), value);
                    }
                    _ => {}
                }
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some(mut entry) = pending.take() else {
            continue;
        };
        let Ok(url) = base.join(line) else {
            continue;
        };
        if url.scheme() != "https" {
            continue;
        }
        entry.url = url.into();
        entry.headers = sanitize_headers(pending_options.clone());
        entries.push(entry);
        pending_options.clear();
    }
    entries
}

fn parse_attributes(input: &str) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let key_start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'=' {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            // Duration or a bare token; M3U puts it before the attributes.
            continue;
        }
        let key = input[key_start..index].trim().to_ascii_lowercase();
        index += 1;
        let value = if index < bytes.len() && bytes[index] == b'"' {
            index += 1;
            let value_start = index;
            while index < bytes.len() && bytes[index] != b'"' {
                index += 1;
            }
            let value = input[value_start..index].to_owned();
            index = (index + 1).min(bytes.len());
            value
        } else {
            let value_start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            input[value_start..index].to_owned()
        };
        if !key.is_empty() {
            attributes.insert(key, value);
        }
    }
    attributes
}

#[derive(Debug, Deserialize)]
struct JsonFeed {
    #[allow(dead_code)]
    name: Option<String>,
    #[serde(default, alias = "streams")]
    items: Vec<JsonItem>,
}

#[derive(Debug, Deserialize)]
struct JsonItem {
    id: Option<Value>,
    title: Option<String>,
    name: Option<String>,
    #[serde(alias = "mediaType")]
    media_type: Option<String>,
    #[serde(rename = "type")]
    r#type: Option<String>,
    year: Option<i32>,
    season: Option<i32>,
    episode: Option<i32>,
    url: Option<String>,
    quality: Option<String>,
    language: Option<String>,
    format: Option<String>,
    audio: Option<String>,
    #[serde(default)]
    subtitles: Option<Vec<String>>,
    #[serde(default)]
    headers: Option<BTreeMap<String, String>>,
    #[serde(alias = "expiresAt")]
    expires_at: Option<i64>,
    metadata: Option<BTreeMap<String, Value>>,
}

fn parse_json_feed(body: &str) -> Result<Vec<JsonItem>, NetError> {
    let value: Value = serde_json::from_str(body).map_err(|_| NetError::ContentType)?;
    // Accept the documented object shape and a bare array for convenience.
    if value.is_array() {
        return serde_json::from_value::<Vec<JsonItem>>(value).map_err(|_| NetError::ContentType);
    }
    let feed: JsonFeed = serde_json::from_value(value).map_err(|_| NetError::ContentType)?;
    Ok(feed.items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_m3u_attributes_and_titles() {
        let base = reqwest::Url::parse("https://example.com/list.m3u").unwrap();
        let body = "#EXTM3U\n#EXTINF:-1 tvg-id=\"tt1234567\" tvg-name=\"Big Buck Bunny\" group-title=\"Movies\",Big Buck Bunny (2008)\n#EXTVLCOPT:http-referrer=https://example.com/\nhttps://cdn.example.com/bunny/master.m3u8\n";
        let entries = parse_m3u(body, &base);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id.as_deref(), Some("tt1234567"));
        assert_eq!(entries[0].title, "Big Buck Bunny (2008)");
        assert_eq!(entries[0].url, "https://cdn.example.com/bunny/master.m3u8");
        assert_eq!(
            entries[0].headers.get("Referer").map(String::as_str),
            Some("https://example.com/")
        );
    }

    #[test]
    fn json_feed_accepts_streams_alias() {
        let items = parse_json_feed(
            r#"{"name":"Test","streams":[{"id":"tt1234567","title":"Big Buck Bunny","url":"https://cdn.example.com/a.mp4","format":"mp4"}]}"#,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn rejects_framing_headers() {
        let mut headers = BTreeMap::new();
        headers.insert("Host".to_owned(), "evil.example".to_owned());
        headers.insert("Referer".to_owned(), "https://example.com/".to_owned());
        let headers = sanitize_headers(headers);
        assert!(!headers.contains_key("Host"));
        assert!(headers.contains_key("Referer"));
    }
}
