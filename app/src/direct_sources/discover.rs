//! Domain-restricted provider discovery.
//!
//! Discovery fetches one public homepage, a handful of conventional catalog
//! paths, and links already present on that page. It obeys robots.txt, does not
//! crawl across domains, does not bypass authentication, and never executes a
//! remote manifest or plugin. Documents it cannot understand are reported as
//! unsupported instead of being imported.

use std::time::Duration;

use reqwest::Url;
use serde_json::Value;

use crate::direct_sources::{
    net::{NetError, SafeHttp},
    types::{CustomSourceKind, DiscoveryCandidate},
};

const MAX_HTML_BYTES: usize = 512 * 1024;
const MAX_CANDIDATE_BYTES: usize = 1024 * 1024;
const MAX_PROBES: usize = 6;

pub async fn discover(http: &SafeHttp, input: &str) -> Result<Vec<DiscoveryCandidate>, NetError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(NetError::InvalidUrl);
    }
    let normalized = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let base = SafeHttp::validate_public_url(&normalized).await?;
    let robots = fetch_robots(http, &base).await?;
    let crawl_delay = Duration::from_millis(robots.crawl_delay_ms.min(2_000));

    let mut candidate_urls = Vec::new();
    let homepage_allowed = robots.can_fetch(base.path());
    let mut homepage = None;
    if homepage_allowed {
        if let Ok(fetched) = http.get_text(base.as_str(), MAX_HTML_BYTES).await {
            homepage = Some(fetched);
        }
        tokio::time::sleep(crawl_delay).await;
    }
    if let Some(fetched) = homepage.as_ref() {
        candidate_urls.extend(extract_candidate_links(&fetched.body, &fetched.final_url));
    }

    for path in [
        "/manifest.json",
        "/providers.json",
        "/feed.json",
        "/playlist.m3u",
        "/playlist.m3u8",
        "/rss.xml",
    ] {
        if candidate_urls.len() >= MAX_PROBES {
            break;
        }
        let Ok(url) = base.join(path) else {
            continue;
        };
        if url.host_str() != base.host_str() || !robots.can_fetch(url.path()) {
            continue;
        }
        if !candidate_urls
            .iter()
            .any(|existing| existing == url.as_str())
        {
            candidate_urls.push(url.to_string());
        }
    }

    let mut seen = Vec::new();
    let mut candidates = Vec::new();
    for url in candidate_urls.into_iter().take(MAX_PROBES * 2) {
        if seen.iter().any(|existing| existing == &url) {
            continue;
        }
        seen.push(url.clone());
        tokio::time::sleep(crawl_delay).await;
        let Ok(fetched) = http.get_text(&url, MAX_CANDIDATE_BYTES).await else {
            continue;
        };
        if let Some(candidate) = inspect_candidate(&fetched.body, &fetched.final_url, &base)
            && !candidates
                .iter()
                .any(|existing: &DiscoveryCandidate| existing.url == candidate.url)
        {
            candidates.push(candidate);
        }
    }

    if candidates.is_empty() {
        return Err(NetError::ContentType);
    }
    Ok(candidates)
}

async fn fetch_robots(http: &SafeHttp, base: &Url) -> Result<Robots, NetError> {
    let Ok(url) = base.join("/robots.txt") else {
        return Err(NetError::InvalidUrl);
    };
    match http.get_text(url.as_str(), 128 * 1024).await {
        Ok(fetched) => Ok(Robots::parse(&fetched.body)),
        // No robots.txt means no restrictions. Any other failure fails closed:
        // discovery must not crawl a domain whose policy it cannot read.
        Err(NetError::Status(404)) | Err(NetError::Status(410)) => Ok(Robots::default()),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Default, Clone)]
struct Robots {
    rules: Vec<(bool, String)>,
    crawl_delay_ms: u64,
}

impl Robots {
    fn parse(body: &str) -> Self {
        let mut robots = Self::default();
        let mut agents: Vec<String> = Vec::new();
        let mut rules: Vec<(bool, String)> = Vec::new();
        let mut in_rules = false;

        let flush = |agents: &mut Vec<String>,
                     rules: &mut Vec<(bool, String)>,
                     robots: &mut Robots,
                     in_rules: &mut bool| {
            if agents
                .iter()
                .any(|agent| agent == "*" || agent.eq_ignore_ascii_case("senzo"))
            {
                robots.rules.append(rules);
            }
            agents.clear();
            rules.clear();
            *in_rules = false;
        };

        for line in body.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            match key.as_str() {
                "user-agent" => {
                    if in_rules {
                        flush(&mut agents, &mut rules, &mut robots, &mut in_rules);
                    }
                    agents.push(value.to_owned());
                }
                "disallow" => {
                    in_rules = true;
                    if !value.is_empty() {
                        rules.push((false, value.to_owned()));
                    }
                }
                "allow" => {
                    in_rules = true;
                    if !value.is_empty() {
                        rules.push((true, value.to_owned()));
                    }
                }
                "crawl-delay" => {
                    if let Ok(delay) = value.parse::<f64>() {
                        robots.crawl_delay_ms = (delay * 1000.0).clamp(0.0, 60_000.0) as u64;
                    }
                }
                _ => {}
            }
        }
        flush(&mut agents, &mut rules, &mut robots, &mut in_rules);
        robots
    }

    fn can_fetch(&self, path: &str) -> bool {
        let path = if path.is_empty() { "/" } else { path };
        let mut winner: Option<(bool, usize)> = None;
        for (allow, rule) in &self.rules {
            if rule == "/" || path.starts_with(rule.trim_end_matches('*')) {
                let length = rule.len();
                if winner.is_none_or(|(_, current)| length >= current) {
                    winner = Some((*allow, length));
                }
            }
        }
        winner.map(|(allow, _)| allow).unwrap_or(true)
    }
}

fn extract_candidate_links(body: &str, base: &Url) -> Vec<String> {
    let mut links = Vec::new();
    for (index, _) in body.match_indices("href=") {
        let start = index + 5;
        let Some(quote) = body[start..].chars().next() else {
            continue;
        };
        if quote != '"' && quote != '\'' {
            continue;
        }
        let value_start = start + quote.len_utf8();
        let Some(end) = body[value_start..].find(quote) else {
            continue;
        };
        let value = &body[value_start..value_start + end];
        let Ok(url) = base.join(value) else {
            continue;
        };
        if url.host_str() != base.host_str() {
            continue;
        }
        let lower_path = url.path().to_ascii_lowercase();
        if [
            ".m3u",
            ".m3u8",
            ".json",
            ".rss",
            ".atom",
            "/feed",
            "/manifest",
            "/providers",
        ]
        .iter()
        .any(|needle| lower_path.contains(needle))
            && links.len() < MAX_PROBES
            && !links.iter().any(|existing| existing == url.as_str())
        {
            links.push(url.to_string());
        }
    }
    links
}

fn inspect_candidate(body: &str, url: &Url, base: &Url) -> Option<DiscoveryCandidate> {
    let trimmed = body.trim_start();
    let host = url.host_str().unwrap_or("unknown");
    let domain = base.host_str().unwrap_or(host).to_owned();
    let id = format!("{:016x}", fnv1a(url.as_str()));

    if trimmed.starts_with("#EXTM3U") || trimmed.contains("\n#EXTINF:") {
        let entries = body
            .lines()
            .filter(|line| line.trim_start().starts_with("#EXTINF:"))
            .count();
        return Some(DiscoveryCandidate {
            id,
            name: format!("{host} playlist"),
            domain,
            method: "M3U/M3U8 playlist".to_owned(),
            content_types: "movie, series".to_owned(),
            formats: "m3u8, mp4".to_owned(),
            status: format!("{entries} playlist entry(s)"),
            required_config: "None".to_owned(),
            url: url.to_string(),
            kind: Some(CustomSourceKind::M3u),
        });
    }

    if let Ok(json) = serde_json::from_str::<Value>(trimmed) {
        let name = json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(host)
            .to_owned();
        let items = json
            .get("items")
            .or_else(|| json.get("streams"))
            .and_then(Value::as_array);
        if let Some(items) = items {
            let kinds = items
                .iter()
                .filter_map(|item| item.get("type").or_else(|| item.get("mediaType")))
                .filter_map(Value::as_str)
                .collect::<std::collections::BTreeSet<_>>();
            return Some(DiscoveryCandidate {
                id,
                name,
                domain,
                method: "JSON feed".to_owned(),
                content_types: if kinds.is_empty() {
                    "movie, series".to_owned()
                } else {
                    kinds.into_iter().collect::<Vec<_>>().join(", ")
                },
                formats: "m3u8, mpd, mp4, mkv, webm".to_owned(),
                status: format!("{} item(s)", items.len()),
                required_config: "None".to_owned(),
                url: url.to_string(),
                kind: Some(CustomSourceKind::JsonFeed),
            });
        }
        if json.get("scrapers").is_some() || json.get("providers").is_some() {
            return Some(DiscoveryCandidate {
                id,
                name,
                domain,
                method: "Remote plugin manifest".to_owned(),
                content_types: "unknown".to_owned(),
                formats: "unknown".to_owned(),
                status: "Not installed: SENZO does not execute remote plugin code".to_owned(),
                required_config: "Unsupported; review the provider's license and data mapping"
                    .to_owned(),
                url: url.to_string(),
                kind: None,
            });
        }
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<?xml") || lower.starts_with("<rss") || lower.starts_with("<feed") {
        return Some(DiscoveryCandidate {
            id,
            name: format!("{host} feed"),
            domain,
            method: "RSS/Atom feed".to_owned(),
            content_types: "unknown".to_owned(),
            formats: "unknown".to_owned(),
            status: "Reachable; manual field mapping is required".to_owned(),
            required_config: "Feed mapping is not configurable in this release".to_owned(),
            url: url.to_string(),
            kind: None,
        });
    }
    None
}

fn fnv1a(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_disallow_wins_for_a_restricted_path() {
        let robots = Robots::parse("User-agent: *\nDisallow: /private\nAllow: /private/public\n");
        assert!(!robots.can_fetch("/private/file"));
        assert!(robots.can_fetch("/"));
        assert!(robots.can_fetch("/private/public/page"));
    }

    #[test]
    fn robots_404_default_allows() {
        let robots = Robots::default();
        assert!(robots.can_fetch("/anything"));
    }

    #[test]
    fn detects_m3u_and_json_candidates() {
        let base = Url::parse("https://example.com/").unwrap();
        let m3u = inspect_candidate(
            "#EXTM3U\n#EXTINF:-1,Test\nhttps://cdn.example.com/a.m3u8",
            &base,
            &base,
        )
        .unwrap();
        assert_eq!(m3u.kind, Some(CustomSourceKind::M3u));
        let json = inspect_candidate(
            r#"{"name":"Test","items":[{"title":"A","url":"https://cdn.example.com/a.mp4"}]}"#,
            &base,
            &base,
        )
        .unwrap();
        assert_eq!(json.kind, Some(CustomSourceKind::JsonFeed));
    }

    #[test]
    fn plugin_manifest_is_reported_but_not_importable() {
        let base = Url::parse("https://example.com/").unwrap();
        let candidate = inspect_candidate(r#"{"scrapers":[]}"#, &base, &base).unwrap();
        assert!(candidate.kind.is_none());
    }
}
