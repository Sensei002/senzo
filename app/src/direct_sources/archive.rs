//! Internet Archive built-in provider.
//!
//! The Archive exposes a public JSON search API and per-item file metadata.
//! Only items that carry an explicit open license are offered. Nothing here
//! downloads a file or executes remote code: the player receives a validated
//! HTTPS URL for the chosen file.

use serde::Deserialize;
use serde_json::Value;

use crate::direct_sources::{
    matching::{format_from_url, is_playable_format, parse_quality, titles_match, years_match},
    net::{NetError, SafeHttp},
    types::{DirectRequest, DirectStream, INTERNET_ARCHIVE},
};

const PROVIDER_NAME: &str = "Internet Archive";
const SEARCH_ENDPOINT: &str = "https://archive.org/advancedsearch.php";
const METADATA_ENDPOINT: &str = "https://archive.org/metadata/";
const MAX_DOCUMENTS: usize = 6;
const MAX_FILES_PER_ITEM: usize = 2;

#[derive(Debug, Deserialize)]
struct SearchResponse {
    response: Option<SearchBody>,
}

#[derive(Debug, Deserialize)]
struct SearchBody {
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

#[derive(Debug, Deserialize)]
struct SearchDoc {
    identifier: Option<String>,
    title: Option<String>,
    year: Option<Value>,
    language: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct MetadataResponse {
    metadata: Option<ItemMetadata>,
    #[serde(default)]
    files: Vec<ItemFile>,
}

#[derive(Debug, Deserialize)]
struct ItemMetadata {
    title: Option<String>,
    #[serde(rename = "licenseurl")]
    license_url: Option<Value>,
    language: Option<Value>,
    #[serde(rename = "access-restricted-item")]
    access_restricted: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ItemFile {
    name: Option<String>,
    format: Option<String>,
    size: Option<Value>,
}

pub async fn resolve(
    http: &SafeHttp,
    request: &DirectRequest,
) -> Result<Vec<DirectStream>, NetError> {
    if !request.is_movie() {
        // The Archive's film collection is organised as complete works, not
        // episodes. Returning nothing is safer than labelling a full film as
        // an arbitrary season/episode.
        return Ok(Vec::new());
    }

    let mut documents = search(http, request).await?;
    if documents.is_empty()
        && let Some(imdb_id) = request.imdb_id.as_deref()
    {
        documents = search_by_imdb(http, imdb_id).await?;
    }

    let mut streams = Vec::new();
    for document in documents.into_iter().take(MAX_DOCUMENTS) {
        if streams.len() >= MAX_FILES_PER_ITEM * 2 {
            break;
        }
        let Some(identifier) = document.identifier.as_deref() else {
            continue;
        };
        let title = document.title.as_deref().unwrap_or_default();
        if !titles_match(title, request) || !years_match(value_year(&document.year), request.year) {
            continue;
        }
        let Ok(mut metadata_url) = reqwest::Url::parse(METADATA_ENDPOINT) else {
            continue;
        };
        if let Ok(mut segments) = metadata_url.path_segments_mut() {
            segments.push(identifier);
        }
        let Ok(metadata) = http
            .get_json::<MetadataResponse>(metadata_url.as_str(), 1024 * 1024)
            .await
        else {
            continue;
        };
        let Some(item) = metadata.metadata.as_ref() else {
            continue;
        };
        if item.access_restricted.as_ref().is_some_and(value_is_true) {
            continue;
        }
        if !license_is_open(&item.license_url) {
            continue;
        }

        let language = value_string(&item.language)
            .or_else(|| value_string(&document.language))
            .unwrap_or_default();
        let mut files = metadata
            .files
            .iter()
            .filter_map(|file| {
                let name = file.name.as_deref()?;
                let format = file.format.as_deref().unwrap_or_default();
                let detected = format_from_url(name);
                let looks_like_video = format.to_ascii_lowercase().contains("mpeg4")
                    || format.to_ascii_lowercase().contains("h.264")
                    || format.to_ascii_lowercase().contains("webm")
                    || format.to_ascii_lowercase().contains("matroska");
                if !is_playable_format(&detected) || !looks_like_video {
                    return None;
                }
                let size = value_u64(&file.size).unwrap_or_default();
                Some((name.to_owned(), detected, size))
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| {
            rank_quality(&right.0)
                .cmp(&rank_quality(&left.0))
                .then_with(|| right.2.cmp(&left.2))
        });

        for (name, format, _size) in files.into_iter().take(MAX_FILES_PER_ITEM) {
            let Ok(mut url) = reqwest::Url::parse("https://archive.org/download/") else {
                continue;
            };
            if let Ok(mut segments) = url.path_segments_mut() {
                segments.push(identifier);
                for segment in name.split('/') {
                    segments.push(segment);
                }
            }
            let mut stream = DirectStream::new(
                INTERNET_ARCHIVE,
                PROVIDER_NAME,
                request,
                url.into(),
                &format,
            );
            stream.title = item.title.clone().unwrap_or_else(|| title.to_owned());
            stream.quality = parse_quality(&name);
            stream.language = language.clone();
            stream.audio = detect_audio(&name);
            stream
                .metadata
                .insert("collection".to_owned(), "Internet Archive".to_owned());
            stream
                .metadata
                .insert("license".to_owned(), license_label(&item.license_url));
            stream.metadata.insert(
                "details".to_owned(),
                format!("https://archive.org/details/{identifier}"),
            );
            streams.push(stream);
        }
    }
    Ok(streams)
}

pub async fn validate(http: &SafeHttp) -> Result<String, NetError> {
    let request = DirectRequest {
        imdb_id: None,
        meta_id: String::new(),
        media_type: "movie".to_owned(),
        title: "Big Buck Bunny".to_owned(),
        year: None,
        season: None,
        episode: None,
        episode_title: None,
        video_id: None,
    };
    let results = resolve(http, &request).await?;
    if results.is_empty() {
        return Err(NetError::Request);
    }
    Ok(format!("{} open film(s) found", results.len()))
}

async fn search(http: &SafeHttp, request: &DirectRequest) -> Result<Vec<SearchDoc>, NetError> {
    if request.title.trim().is_empty() {
        return Ok(Vec::new());
    }
    let escaped = request
        .title
        .replace('\\', " ")
        .replace('"', " ")
        .replace(':', " ");
    let query = format!("title:(\"{escaped}\") AND mediatype:movies");
    search_query(http, &query).await
}

async fn search_by_imdb(http: &SafeHttp, imdb_id: &str) -> Result<Vec<SearchDoc>, NetError> {
    if !is_valid_imdb_id(imdb_id) {
        return Ok(Vec::new());
    }
    let query = format!("external-identifier:\"imdb://{imdb_id}\" AND mediatype:movies");
    search_query(http, &query).await
}

async fn search_query(http: &SafeHttp, query: &str) -> Result<Vec<SearchDoc>, NetError> {
    let mut url = reqwest::Url::parse(SEARCH_ENDPOINT).map_err(|_| NetError::InvalidUrl)?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("fl[]", "identifier")
        .append_pair("fl[]", "title")
        .append_pair("fl[]", "year")
        .append_pair("fl[]", "licenseurl")
        .append_pair("fl[]", "language")
        .append_pair("rows", &MAX_DOCUMENTS.to_string())
        .append_pair("page", "1")
        .append_pair("output", "json");
    let response = http
        .get_json::<SearchResponse>(url.as_str(), 512 * 1024)
        .await?;
    Ok(response.response.map(|body| body.docs).unwrap_or_default())
}

fn is_valid_imdb_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("tt") else {
        return false;
    };
    (7..=10).contains(&rest.len()) && rest.chars().all(|character| character.is_ascii_digit())
}

fn value_string(value: &Option<Value>) -> Option<String> {
    match value.as_ref()? {
        Value::String(value) => Some(value.trim().to_owned()),
        Value::Array(values) => values.iter().find_map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        }),
        _ => None,
    }
}

fn value_year(value: &Option<Value>) -> Option<i32> {
    let value = value_string(value)?;
    let digits = value
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.get(0..4)?.parse().ok()
}

fn value_u64(value: &Option<Value>) -> Option<u64> {
    match value.as_ref()? {
        Value::String(value) => value.parse().ok(),
        Value::Number(value) => value.as_u64(),
        _ => None,
    }
}

fn value_is_true(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::String(value) => value.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn license_is_open(value: &Option<Value>) -> bool {
    let Some(license) = value_string(value) else {
        return false;
    };
    let lower = license.to_ascii_lowercase();
    lower.contains("creativecommons.org")
        || lower.contains("publicdomain")
        || lower.contains("rightsstatements.org")
}

fn license_label(value: &Option<Value>) -> String {
    value_string(value).unwrap_or_else(|| "Open license".to_owned())
}

fn rank_quality(name: &str) -> u8 {
    let lower = name.to_ascii_lowercase();
    if lower.contains("1080") {
        4
    } else if lower.contains("720") {
        3
    } else if lower.contains("480") {
        2
    } else {
        1
    }
}

fn detect_audio(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("5.1") || lower.contains("5_1") {
        "5.1".to_owned()
    } else if lower.contains("stereo") {
        "Stereo".to_owned()
    } else {
        "unknown".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_explicit_open_licenses() {
        assert!(license_is_open(&Some(Value::String(
            "https://creativecommons.org/licenses/by/4.0/".to_owned()
        ))));
        assert!(!license_is_open(&Some(Value::String(
            "All rights reserved".to_owned()
        ))));
        assert!(!license_is_open(&None));
    }

    #[test]
    fn validates_imdb_shape() {
        assert!(is_valid_imdb_id("tt1234567"));
        assert!(!is_valid_imdb_id("1234567"));
        assert!(!is_valid_imdb_id("ttabc"));
    }
}
