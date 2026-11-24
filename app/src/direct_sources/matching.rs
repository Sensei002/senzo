//! Matching and metadata helpers shared by providers.

use crate::direct_sources::types::DirectRequest;

pub fn normalize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn titles_match(candidate: &str, request: &DirectRequest) -> bool {
    let candidate = normalize(candidate);
    let wanted = normalize(&request.title);
    if wanted.is_empty() || candidate.is_empty() {
        return false;
    }
    if candidate == wanted {
        return true;
    }
    // Providers often decorate titles with "Official", "4K", or a year. Allow
    // a containment match only when the requested title is long enough to be
    // distinctive; short titles otherwise match unrelated clips.
    wanted.len() >= 4 && (candidate.contains(&wanted) || wanted.contains(&candidate))
}

pub fn years_match(candidate_year: Option<i32>, request_year: Option<i32>) -> bool {
    match (candidate_year, request_year) {
        (Some(candidate), Some(wanted)) => (candidate - wanted).abs() <= 1,
        _ => true,
    }
}

pub fn parse_year(value: &str) -> Option<i32> {
    let digits = value
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect::<String>();
    if digits.len() < 4 {
        return None;
    }
    digits[0..4].parse().ok()
}

pub fn parse_quality(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    for (needle, label) in [
        ("2160", "4K"),
        ("4k", "4K"),
        ("uhd", "4K"),
        ("1440", "1440p"),
        ("1080", "1080p"),
        ("720", "720p"),
        ("576", "576p"),
        ("480", "480p"),
        ("360", "360p"),
    ] {
        if lower.contains(needle) {
            return label.to_owned();
        }
    }
    "auto".to_owned()
}

pub fn parse_language(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    for (needle, label) in [
        ("english", "English"),
        ("eng", "English"),
        ("spanish", "Spanish"),
        ("spa", "Spanish"),
        ("french", "French"),
        ("fra", "French"),
        ("german", "German"),
        ("deu", "German"),
        ("italian", "Italian"),
        ("ita", "Italian"),
        ("portuguese", "Portuguese"),
        ("por", "Portuguese"),
        ("japanese", "Japanese"),
        ("jpn", "Japanese"),
        ("hindi", "Hindi"),
        ("hin", "Hindi"),
        ("multi", "Multi"),
    ] {
        if lower.contains(needle) {
            return label.to_owned();
        }
    }
    "unknown".to_owned()
}

pub fn format_from_url(url: &str) -> String {
    let lower = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    for format in [
        "m3u8", "mpd", "mp4", "m4v", "mkv", "webm", "ogv", "mov", "avi",
    ] {
        if lower.ends_with(&format!(".{format}")) {
            return format.to_owned();
        }
    }
    "unknown".to_owned()
}

pub fn is_playable_format(format: &str) -> bool {
    matches!(
        format,
        "m3u8" | "mpd" | "mp4" | "m4v" | "mkv" | "webm" | "ogv" | "mov"
    )
}

/// Parse `S01E02`, `1x02`, or `Season 1 Episode 2` out of a title.
pub fn parse_episode(value: &str) -> Option<(i32, i32)> {
    let lower = value.to_ascii_lowercase();
    let tokens = lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != 'x')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    for token in &tokens {
        if let Some(rest) = token.strip_prefix('s')
            && let Some((season, episode)) = rest.split_once('e')
            && let (Ok(season), Ok(episode)) = (season.parse(), episode.parse())
        {
            return Some((season, episode));
        }
        if let Some((season, episode)) = token.split_once('x')
            && let (Ok(season), Ok(episode)) = (season.parse(), episode.parse())
        {
            return Some((season, episode));
        }
    }
    None
}

/// Providers that identify an item by IMDb id must not fall back to a title
/// match when a different id is present: ids are authoritative.
pub fn identity_matches(
    candidate_imdb: Option<&str>,
    candidate_title: &str,
    candidate_year: Option<i32>,
    request: &DirectRequest,
) -> bool {
    if let Some(wanted) = request.imdb_id.as_deref()
        && let Some(candidate) = candidate_imdb
        && !candidate.is_empty()
    {
        return candidate.eq_ignore_ascii_case(wanted);
    }
    titles_match(candidate_title, request) && years_match(candidate_year, request.year)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> DirectRequest {
        DirectRequest {
            imdb_id: Some("tt1234567".to_owned()),
            meta_id: "tt1234567".to_owned(),
            media_type: "movie".to_owned(),
            title: "Big Buck Bunny".to_owned(),
            year: Some(2008),
            season: None,
            episode: None,
            episode_title: None,
            video_id: None,
        }
    }

    #[test]
    fn normalizes_titles() {
        assert_eq!(normalize("Big  Buck: Bunny!"), "big buck bunny");
    }

    #[test]
    fn strong_id_beats_a_conflicting_title() {
        assert!(!identity_matches(
            Some("tt7654321"),
            "Big Buck Bunny",
            Some(2008),
            &request()
        ));
        assert!(identity_matches(
            Some("tt1234567"),
            "Something else",
            None,
            &request()
        ));
    }

    #[test]
    fn parses_quality_and_episode() {
        assert_eq!(parse_quality("Movie.1080p.WEB"), "1080p");
        assert_eq!(parse_episode("Show.S02E07.1080p"), Some((2, 7)));
        assert_eq!(parse_episode("Show 3x04"), Some((3, 4)));
        assert_eq!(
            format_from_url("https://example.com/a.m3u8?token=x"),
            "m3u8"
        );
    }
}
