use std::{
    net::{IpAddr, ToSocketAddrs},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::{Host, Url};

use crate::storage::{LocalDatabase, StoredAddon};

const MAX_ADDON_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_ADDON_RESOURCE_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub resources: Vec<Value>,
    #[serde(default, rename = "types")]
    pub content_types: Vec<String>,
    #[serde(default, rename = "idPrefixes")]
    pub id_prefixes: Vec<String>,
    #[serde(default)]
    pub catalogs: Vec<Value>,
}

impl AddonManifest {
    pub fn supports_resource(&self, resource: &str) -> bool {
        self.resources.iter().any(|entry| match entry {
            Value::String(name) => name == resource,
            Value::Object(object) => object
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == resource),
            _ => false,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StremioCatalogResponse {
    #[serde(default)]
    pub metas: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StremioMetaResponse {
    pub meta: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StremioStreamResponse {
    #[serde(default)]
    pub streams: Vec<StremioStream>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StremioStream {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "infoHash")]
    pub info_hash: Option<String>,
    #[serde(default, rename = "fileIdx")]
    pub file_index: Option<u32>,
    #[serde(default, rename = "externalUrl")]
    pub external_url: Option<String>,
    #[serde(default)]
    pub behavior_hints: Option<Value>,
}

impl StremioStream {
    /// Returns a playable HTTP(S) URL. Torrent and external-link entries are not direct URLs.
    pub fn direct_url(&self) -> Result<Option<Url>> {
        let Some(raw_url) = self.url.as_deref() else {
            return Ok(None);
        };
        let url = Url::parse(raw_url).context("add-on returned an invalid stream URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            bail!(
                "add-on stream URLs must be absolute HTTP or HTTPS URLs without credentials or fragments"
            );
        }
        match url.host().expect("host was validated") {
            Host::Domain(host) => {
                let host = host.trim_end_matches('.').to_ascii_lowercase();
                if host == "localhost"
                    || host.ends_with(".localhost")
                    || host.ends_with(".local")
                    || host.ends_with(".internal")
                {
                    bail!("add-on stream URLs cannot target local network hostnames");
                }
            }
            Host::Ipv4(ip) if is_disallowed_ip(IpAddr::V4(ip)) => {
                bail!("add-on stream URLs cannot target local or reserved IP addresses")
            }
            Host::Ipv6(ip) if is_disallowed_ip(IpAddr::V6(ip)) => {
                bail!("add-on stream URLs cannot target local or reserved IP addresses")
            }
            Host::Ipv4(_) | Host::Ipv6(_) => {}
        }
        Ok(Some(url))
    }

    pub fn is_torrent(&self) -> bool {
        self.info_hash
            .as_deref()
            .is_some_and(|hash| is_valid_info_hash(hash))
    }
}

fn is_valid_info_hash(value: &str) -> bool {
    (value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[derive(Debug, Clone)]
pub struct AddonStreamResult {
    pub addon_id: String,
    pub addon_name: String,
    pub streams: Vec<StremioStream>,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub struct AddonClient;

impl AddonClient {
    pub fn new() -> Self {
        Self
    }

    /// Fetches and validates a Stremio-compatible manifest, then saves it locally.
    /// SENZO ships with no preinstalled streaming add-ons; users explicitly provide URLs.
    pub async fn install_from_url(
        &self,
        database: &LocalDatabase,
        input: &str,
    ) -> Result<StoredAddon> {
        let manifest_url = normalize_manifest_url(input)?;
        let value = request_json(&manifest_url, MAX_ADDON_MANIFEST_BYTES).await?;
        let manifest: AddonManifest = serde_json::from_value(value.clone())
            .context("the manifest is missing required Stremio add-on fields")?;

        if manifest.id.trim().is_empty()
            || manifest.name.trim().is_empty()
            || manifest.version.trim().is_empty()
        {
            bail!("the add-on manifest must include id, name, and version");
        }

        let stored = StoredAddon {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            manifest_url: manifest_url.to_string(),
            enabled: true,
            manifest_json: serde_json::to_string(&value)?,
            installed_at: unix_timestamp(),
        };
        database.save_addon(&stored)?;
        Ok(stored)
    }

    /// Retrieves a catalog page exposed by the selected enabled add-on.
    pub async fn catalog(
        &self,
        addon: &StoredAddon,
        media_type: &str,
        catalog_id: &str,
        extra: &[(&str, &str)],
    ) -> Result<StremioCatalogResponse> {
        let endpoint = resource_endpoint(addon, "catalog", media_type, catalog_id, extra)?;
        let value = request_json(&endpoint, MAX_ADDON_RESOURCE_BYTES).await?;
        serde_json::from_value(value).context("add-on returned an invalid catalog response")
    }

    /// Retrieves metadata for a Stremio media ID from an enabled add-on.
    pub async fn metadata(
        &self,
        addon: &StoredAddon,
        media_type: &str,
        media_id: &str,
    ) -> Result<StremioMetaResponse> {
        let endpoint = resource_endpoint(addon, "meta", media_type, media_id, &[])?;
        let value = request_json(&endpoint, MAX_ADDON_RESOURCE_BYTES).await?;
        serde_json::from_value(value).context("add-on returned an invalid metadata response")
    }

    /// Retrieves streams for a movie or series episode from an enabled add-on.
    pub async fn streams(
        &self,
        addon: &StoredAddon,
        media_type: &str,
        media_id: &str,
    ) -> Result<StremioStreamResponse> {
        let endpoint = resource_endpoint(addon, "stream", media_type, media_id, &[])?;
        let value = request_json(&endpoint, MAX_ADDON_RESOURCE_BYTES).await?;
        serde_json::from_value(value).context("add-on returned an invalid stream response")
    }

    /// Queries enabled stream-capable add-ons independently so one provider failure does not
    /// discard results from other providers.
    pub async fn discover_streams(
        &self,
        addons: &[StoredAddon],
        media_type: &str,
        media_id: &str,
    ) -> Vec<AddonStreamResult> {
        let mut results = Vec::new();
        for addon in addons.iter().filter(|addon| addon.enabled) {
            match serde_json::from_str::<AddonManifest>(&addon.manifest_json) {
                Ok(manifest) if manifest.supports_resource("stream") => {}
                Ok(_) => continue,
                Err(error) => {
                    results.push(AddonStreamResult {
                        addon_id: addon.id.clone(),
                        addon_name: addon.name.clone(),
                        streams: Vec::new(),
                        error: Some(format!("invalid saved manifest: {error}")),
                    });
                    continue;
                }
            }
            match self.streams(addon, media_type, media_id).await {
                Ok(response) => results.push(AddonStreamResult {
                    addon_id: addon.id.clone(),
                    addon_name: addon.name.clone(),
                    streams: response.streams,
                    error: None,
                }),
                Err(error) => results.push(AddonStreamResult {
                    addon_id: addon.id.clone(),
                    addon_name: addon.name.clone(),
                    streams: Vec::new(),
                    error: Some(format!("{error:#}")),
                }),
            }
        }
        results
    }

    /// Resolve a stream hostname and reject every local/reserved DNS answer before handoff.
    pub async fn validate_direct_stream_url(&self, url: &Url) -> Result<()> {
        if !matches!(url.scheme(), "http" | "https") {
            bail!("only HTTP and HTTPS streams can be handed to MPV");
        }
        match url.host().context("stream URL has no host")? {
            Host::Ipv4(ip) if is_disallowed_ip(IpAddr::V4(ip)) => {
                bail!("stream resolves to a local or reserved address")
            }
            Host::Ipv6(ip) if is_disallowed_ip(IpAddr::V6(ip)) => {
                bail!("stream resolves to a local or reserved address")
            }
            Host::Ipv4(_) | Host::Ipv6(_) => return Ok(()),
            Host::Domain(host) => {
                let host = host.to_owned();
                let port = url
                    .port_or_known_default()
                    .context("stream URL has no known port")?;
                let addresses = tokio::task::spawn_blocking(move || {
                    (host.as_str(), port)
                        .to_socket_addrs()
                        .map(|items| items.collect::<Vec<_>>())
                })
                .await
                .context("stream DNS lookup task failed")?
                .context("could not resolve stream hostname")?;
                if addresses.is_empty()
                    || addresses
                        .iter()
                        .any(|address| is_disallowed_ip(address.ip()))
                {
                    bail!("stream hostname resolves to a local or reserved address");
                }
            }
        }
        Ok(())
    }

    /// Retrieves subtitle tracks from an enabled add-on.
    pub async fn subtitles(
        &self,
        addon: &StoredAddon,
        media_type: &str,
        media_id: &str,
    ) -> Result<Value> {
        let endpoint = resource_endpoint(addon, "subtitles", media_type, media_id, &[])?;
        request_json(&endpoint, MAX_ADDON_RESOURCE_BYTES).await
    }
}

fn resource_endpoint(
    addon: &StoredAddon,
    resource: &str,
    media_type: &str,
    resource_id: &str,
    extra: &[(&str, &str)],
) -> Result<Url> {
    if !addon.enabled {
        bail!("the selected add-on is disabled");
    }
    if !matches!(media_type, "movie" | "series" | "tv" | "channel") {
        bail!("unsupported Stremio media type");
    }
    if resource_id.trim().is_empty() || resource_id.len() > 512 {
        bail!("the Stremio resource ID is empty or too long");
    }

    let manifest: AddonManifest = serde_json::from_str(&addon.manifest_json)
        .context("the stored add-on manifest is invalid")?;
    if !manifest.supports_resource(resource) {
        bail!("the selected add-on does not declare the {resource} resource");
    }

    let mut endpoint = addon_base_url(&addon.manifest_url)?;
    let final_segment = if extra.is_empty() {
        format!("{resource_id}.json")
    } else {
        format!(
            "{}.json",
            extra
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&")
        )
    };
    {
        let mut segments = endpoint
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid add-on base URL"))?;
        segments
            .pop_if_empty()
            .push(resource)
            .push(media_type)
            .push(resource_id);
        if extra.is_empty() {
            // The JSON suffix belongs to the final path segment.
            segments.pop().push(&final_segment);
        } else {
            segments.push(&final_segment);
        }
    }
    Ok(endpoint)
}

async fn request_json(url: &Url, max_bytes: usize) -> Result<Value> {
    let client = client_for_public_url(url).await?;
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .context("add-on resource request failed")?;
    if response.status() != StatusCode::OK {
        bail!("add-on endpoint returned HTTP status {}", response.status());
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        bail!("add-on response exceeds the configured size limit");
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("failed while reading add-on response")?
    {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            bail!("add-on response exceeds the configured size limit");
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).context("add-on returned invalid JSON")
}

async fn client_for_public_url(url: &Url) -> Result<Client> {
    let host = url.host_str().context("add-on URL has no host")?.to_owned();
    let port = url
        .port_or_known_default()
        .context("add-on URL has no known port")?;

    if url
        .host()
        .is_some_and(|host| !matches!(host, Host::Domain(_)))
    {
        return Ok(Client::builder()
            .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .redirect(Policy::none())
            .build()?);
    }

    let lookup_host = host.clone();
    let addresses = tokio::task::spawn_blocking(move || {
        (lookup_host.as_str(), port)
            .to_socket_addrs()
            .map(|addresses| addresses.collect::<Vec<_>>())
    })
    .await
    .context("DNS lookup task failed")?
    .context("could not resolve the add-on hostname")?;

    if addresses.is_empty() {
        bail!("the add-on hostname did not resolve to any addresses");
    }
    if addresses
        .iter()
        .any(|address| is_disallowed_ip(address.ip()))
    {
        bail!("the add-on hostname resolves to a local or reserved IP address");
    }

    Client::builder()
        .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(Policy::none())
        .resolve_to_addrs(&host, &addresses)
        .build()
        .context("failed to create a DNS-pinned add-on HTTP client")
}

fn normalize_manifest_url(input: &str) -> Result<Url> {
    let mut url = Url::parse(input.trim()).context("enter a valid add-on URL")?;
    if url.scheme() != "https" || url.host().is_none() {
        bail!("add-on manifest URLs must use HTTPS");
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        bail!("add-on manifest URLs cannot contain credentials or fragments");
    }

    match url.host().expect("host was validated") {
        Host::Domain(host) => {
            let normalized_host = host.trim_end_matches('.').to_ascii_lowercase();
            if normalized_host == "localhost"
                || normalized_host.ends_with(".localhost")
                || normalized_host.ends_with(".local")
                || normalized_host.ends_with(".internal")
            {
                bail!("add-on manifest URLs cannot target local network hostnames");
            }
        }
        Host::Ipv4(address) if is_disallowed_ip(IpAddr::V4(address)) => {
            bail!("add-on manifest URLs cannot target local or reserved IP addresses");
        }
        Host::Ipv6(address) if is_disallowed_ip(IpAddr::V6(address)) => {
            bail!("add-on manifest URLs cannot target local or reserved IP addresses");
        }
        Host::Ipv4(_) | Host::Ipv6(_) => {}
    }

    if !url.path().ends_with("manifest.json") {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid add-on URL"))?;
        segments.pop_if_empty().push("manifest.json");
    }
    Ok(url)
}

fn is_disallowed_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || a == 0
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || a >= 240
        }
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            let global_unicast = octets[0] & 0xe0 == 0x20;
            let documentation =
                octets[0] == 0x20 && octets[1] == 0x01 && octets[2] == 0x0d && octets[3] == 0xb8;
            let unique_local = octets[0] & 0xfe == 0xfc;
            let link_local = octets[0] == 0xfe && octets[1] & 0xc0 == 0x80;
            let ipv4_mapped = octets[..10].iter().all(|byte| *byte == 0)
                && octets[10] == 0xff
                && octets[11] == 0xff;
            let mapped_ipv4 = ipv4_mapped
                .then(|| std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]));
            !global_unicast
                || documentation
                || ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || unique_local
                || link_local
                || mapped_ipv4.is_some_and(|mapped| is_disallowed_ip(mapped.into()))
        }
    }
}

fn addon_base_url(manifest_url: &str) -> Result<Url> {
    let mut url = normalize_manifest_url(manifest_url)?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid add-on manifest URL"))?;
        segments.pop().push("");
    }
    Ok(url)
}

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_addon(resources: &[&str]) -> StoredAddon {
        StoredAddon {
            id: "org.example.addon".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            manifest_url: "https://example.org/addon/manifest.json".into(),
            enabled: true,
            manifest_json: serde_json::json!({
                "id": "org.example.addon",
                "name": "Example",
                "version": "1.0.0",
                "resources": resources,
                "types": ["movie", "series"]
            })
            .to_string(),
            installed_at: 1,
        }
    }

    #[test]
    fn normalizes_an_addon_root_to_manifest() {
        let url = normalize_manifest_url("https://example.org/addon/").expect("valid URL");
        assert_eq!(url.as_str(), "https://example.org/addon/manifest.json");
    }

    #[test]
    fn keeps_configured_manifest_paths() {
        let url = normalize_manifest_url("https://example.org/torrentio/key/manifest.json")
            .expect("valid URL");
        assert_eq!(
            url.as_str(),
            "https://example.org/torrentio/key/manifest.json"
        );
        assert_eq!(
            addon_base_url(url.as_str()).expect("base URL").as_str(),
            "https://example.org/torrentio/key/"
        );
    }

    #[test]
    fn rejects_non_https_addon_urls() {
        assert!(normalize_manifest_url("http://example.org/manifest.json").is_err());
        assert!(normalize_manifest_url("file:///tmp/manifest.json").is_err());
    }

    #[test]
    fn rejects_credentials_fragments_and_local_destinations() {
        assert!(normalize_manifest_url("https://user:pass@example.org/manifest.json").is_err());
        assert!(normalize_manifest_url("https://example.org/manifest.json#fragment").is_err());
        assert!(normalize_manifest_url("https://localhost/manifest.json").is_err());
        assert!(normalize_manifest_url("https://127.0.0.1/manifest.json").is_err());
        assert!(normalize_manifest_url("https://192.168.1.20/manifest.json").is_err());
        assert!(normalize_manifest_url("https://[::1]/manifest.json").is_err());
    }

    #[test]
    fn rejects_special_use_ipv4_and_non_global_ipv6() {
        for address in [
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "240.0.0.1",
        ] {
            assert!(
                is_disallowed_ip(address.parse().expect("IPv4")),
                "{address}"
            );
        }
        for address in ["2001:db8::1", "fc00::1", "fe80::1", "2001:4860:4860::8888"] {
            let ip = address.parse().expect("IPv6");
            assert_eq!(is_disallowed_ip(ip), address != "2001:4860:4860::8888");
        }
    }

    #[test]
    fn builds_stremio_resource_routes_and_extra_segments() {
        let addon = stored_addon(&["catalog", "meta", "stream", "subtitles"]);
        let url = resource_endpoint(
            &addon,
            "catalog",
            "movie",
            "top",
            &[("genre", "Action"), ("skip", "20")],
        )
        .expect("catalog route");
        assert_eq!(
            url.as_str(),
            "https://example.org/addon/catalog/movie/top/genre=Action&skip=20.json"
        );
        let stream =
            resource_endpoint(&addon, "stream", "series", "tt123:1:2", &[]).expect("stream route");
        assert_eq!(
            stream.as_str(),
            "https://example.org/addon/stream/series/tt123:1:2.json"
        );
    }

    #[test]
    fn refuses_undeclared_resources_and_disabled_addons() {
        let addon = stored_addon(&["stream"]);
        assert!(resource_endpoint(&addon, "catalog", "movie", "top", &[]).is_err());
        let mut disabled = addon;
        disabled.enabled = false;
        assert!(resource_endpoint(&disabled, "stream", "movie", "tt123", &[]).is_err());
    }

    #[test]
    fn validates_direct_streams_and_torrent_hashes() {
        let direct = StremioStream {
            name: Some("1080p".into()),
            title: None,
            description: None,
            url: Some("https://cdn.example/video.m3u8".into()),
            info_hash: None,
            file_index: None,
            external_url: None,
            behavior_hints: None,
        };
        assert_eq!(
            direct
                .direct_url()
                .expect("valid URL")
                .expect("URL")
                .scheme(),
            "https"
        );
        let unsafe_stream = StremioStream {
            url: Some("file:///etc/passwd".into()),
            ..direct.clone()
        };
        assert!(unsafe_stream.direct_url().is_err());
        let torrent = StremioStream {
            url: None,
            info_hash: Some("0123456789abcdef0123456789abcdef01234567".into()),
            ..direct
        };
        assert!(torrent.is_torrent());
    }

    #[test]
    fn recognizes_string_and_object_resource_declarations() {
        let manifest: AddonManifest = serde_json::from_value(serde_json::json!({
            "id": "example",
            "name": "Example",
            "version": "1.0.0",
            "resources": ["catalog", {"name": "stream", "types": ["movie"]}]
        }))
        .expect("valid manifest");

        assert!(manifest.supports_resource("catalog"));
        assert!(manifest.supports_resource("stream"));
        assert!(!manifest.supports_resource("subtitles"));
    }
    #[test]
    fn rejects_local_or_credentialed_direct_stream_urls() {
        let stream = |url: &str| StremioStream {
            name: None,
            title: None,
            description: None,
            url: Some(url.into()),
            info_hash: None,
            file_index: None,
            external_url: None,
            behavior_hints: None,
        };
        assert!(stream("http://127.0.0.1/video.mp4").direct_url().is_err());
        assert!(stream("https://localhost/video.mp4").direct_url().is_err());
        assert!(
            stream("https://user:pass@example.org/video.mp4")
                .direct_url()
                .is_err()
        );
        assert!(
            stream("https://example.org/video.mp4#fragment")
                .direct_url()
                .is_err()
        );
        assert!(
            stream("https://cdn.example.org/video.mp4")
                .direct_url()
                .unwrap()
                .is_some()
        );
    }
}
