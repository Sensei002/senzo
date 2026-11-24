//! Wikimedia Commons built-in provider.
//!
//! Uses the public MediaWiki API. Commons only hosts freely licensed media, so
//! every result links back to its file page for attribution.

use std::collections::BTreeMap;

use crate::direct_sources::{
    matching::{format_from_url, is_playable_format, parse_quality, titles_match, years_match},
    net::{NetError, SafeHttp},
    types::{DirectRequest, DirectStream, WIKIMEDIA_COMMONS},
};
use serde::Deserialize;

const PROVIDER_NAME: &str = "Wikimedia Commons";
const API_ENDPOINT: &str = "https://commons.wikimedia.org/w/api.php";
const MAX_PAGES: usize = 6;

#[derive(Debug, Deserialize)]
struct ApiResponse {
    query: Option<ApiQuery>,
}

#[derive(Debug, Deserialize)]
struct ApiQuery {
    #[serde(default)]
    pages: BTreeMap<String, ApiPage>,
}

#[derive(Debug, Deserialize)]
struct ApiPage {
    title: Option<String>,
    #[serde(default)]
    imageinfo: Vec<ImageInfo>,
}

#[derive(Debug, Deserialize)]
struct ImageInfo {
    url: Option<String>,
    #[serde(rename = "descriptionurl")]
    description_url: Option<String>,
    mime: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<f64>,
}

pub async fn resolve(
    http: &SafeHttp,
    request: &DirectRequest,
) -> Result<Vec<DirectStream>, NetError> {
    if !request.is_movie() {
        return Ok(Vec::new());
    }
    if request.title.trim().is_empty() {
        return Ok(Vec::new());
    }
    let query = format!("filetype:video \"{}\"", request.title.replace('"', " "));
    let mut url = reqwest::Url::parse(API_ENDPOINT).map_err(|_| NetError::InvalidUrl)?;
    url.query_pairs_mut()
        .append_pair("action", "query")
        .append_pair("generator", "search")
        .append_pair("gsrsearch", &query)
        .append_pair("gsrnamespace", "6")
        .append_pair("gsrlimit", &MAX_PAGES.to_string())
        .append_pair("prop", "imageinfo")
        .append_pair("iiprop", "url|mime|size")
        .append_pair("format", "json")
        .append_pair("origin", "*");

    let response = http
        .get_json::<ApiResponse>(url.as_str(), 1024 * 1024)
        .await?;
    let Some(query) = response.query else {
        return Ok(Vec::new());
    };

    let mut streams = Vec::new();
    for page in query.pages.into_values() {
        let Some(info) = page.imageinfo.first() else {
            continue;
        };
        let title = page
            .title
            .as_deref()
            .unwrap_or_default()
            .trim_start_matches("File:");
        let year = title
            .split(|character: char| !character.is_ascii_digit())
            .find(|part| part.len() == 4)
            .and_then(|part| part.parse::<i32>().ok());
        if !titles_match(title, request) || !years_match(year, request.year) {
            continue;
        }
        let Some(url) = info.url.as_deref() else {
            continue;
        };
        let mime = info
            .mime
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(
            mime.as_str(),
            "video/webm" | "video/ogg" | "application/ogg" | "video/mp4"
        ) {
            continue;
        }
        let format = format_from_url(url);
        if !is_playable_format(&format) {
            continue;
        }
        let mut stream = DirectStream::new(
            WIKIMEDIA_COMMONS,
            PROVIDER_NAME,
            request,
            url.to_owned(),
            &format,
        );
        stream.title = title.to_owned();
        stream.quality = info
            .height
            .map(|height| match height {
                value if value >= 2000 => "4K".to_owned(),
                value if value >= 1000 => "1080p".to_owned(),
                value if value >= 700 => "720p".to_owned(),
                value if value >= 400 => "480p".to_owned(),
                _ => "360p".to_owned(),
            })
            .unwrap_or_else(|| parse_quality(title));
        stream.language = "multiple".to_owned();
        if let Some(duration) = info.duration {
            stream.metadata.insert(
                "duration".to_owned(),
                format!("{}s", duration.round() as u64),
            );
        }
        if let Some(description) = info.description_url.as_deref() {
            stream
                .metadata
                .insert("attribution".to_owned(), description.to_owned());
        }
        stream.metadata.insert(
            "license".to_owned(),
            "Wikimedia Commons free license".to_owned(),
        );
        streams.push(stream);
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
    Ok(format!("{} freely licensed file(s) found", results.len()))
}
