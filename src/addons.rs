use std::{net::IpAddr, time::Duration};

use anyhow::{Context, Result, bail};
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

const MAX_ADDON_MANIFEST_BYTES: usize = 1024 * 1024;

use crate::storage::{LocalDatabase, StoredAddon};

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

#[derive(Clone)]
pub struct AddonClient {
    client: Client,
}

impl Default for AddonClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AddonClient {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .redirect(Policy::none())
                .build()
                .expect("the default HTTP client configuration is valid"),
        }
    }

    /// Fetches and validates a Stremio-compatible manifest, then saves it locally.
    /// SENZO ships with no preinstalled streaming add-ons; users explicitly provide URLs.
    pub async fn install_from_url(
        &self,
        database: &LocalDatabase,
        input: &str,
    ) -> Result<StoredAddon> {
        let manifest_url = normalize_manifest_url(input)?;
        let mut response = self
            .client
            .get(manifest_url.clone())
            .send()
            .await
            .context("failed to contact the add-on manifest URL")?;

        if response.status() != StatusCode::OK {
            bail!(
                "the add-on manifest endpoint returned HTTP status {}",
                response.status()
            );
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ADDON_MANIFEST_BYTES as u64)
        {
            bail!("the add-on manifest exceeds the 1 MiB size limit");
        }

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("failed while reading the add-on manifest response")?
        {
            if body.len().saturating_add(chunk.len()) > MAX_ADDON_MANIFEST_BYTES {
                bail!("the add-on manifest exceeds the 1 MiB size limit");
            }
            body.extend_from_slice(&chunk);
        }

        let manifest_value: Value = serde_json::from_slice(&body)
            .context("the add-on did not return valid manifest JSON")?;
        let manifest: AddonManifest = serde_json::from_value(manifest_value.clone())
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
            manifest_json: serde_json::to_string(&manifest_value)?,
            installed_at: unix_timestamp(),
        };
        database.save_addon(&stored)?;
        Ok(stored)
    }

    /// Requests streams only from an add-on the user has already installed and enabled.
    pub async fn streams(
        &self,
        addon: &StoredAddon,
        media_type: &str,
        media_id: &str,
    ) -> Result<Value> {
        if !addon.enabled {
            bail!("the selected add-on is disabled");
        }
        if !matches!(media_type, "movie" | "series") {
            bail!("unsupported Stremio media type");
        }

        let mut endpoint = addon_base_url(&addon.manifest_url)?;
        {
            let mut segments = endpoint
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("invalid add-on base URL"))?;
            segments.pop_if_empty();
            segments
                .push("stream")
                .push(media_type)
                .push(&format!("{media_id}.json"));
        }

        self.client
            .get(endpoint)
            .send()
            .await
            .context("add-on stream request failed")?
            .error_for_status()
            .context("add-on stream endpoint returned an unsuccessful status")?
            .json::<Value>()
            .await
            .context("add-on returned an invalid stream response")
    }
}

fn normalize_manifest_url(input: &str) -> Result<Url> {
    let mut url = Url::parse(input.trim()).context("enter a valid add-on URL")?;
    if url.scheme() != "https" || url.host_str().is_none() {
        bail!("add-on manifest URLs must use HTTPS");
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        bail!("add-on manifest URLs cannot contain credentials or fragments");
    }

    let host = url.host_str().expect("host was validated");
    let normalized_host = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized_host == "localhost"
        || normalized_host.ends_with(".localhost")
        || normalized_host.ends_with(".local")
        || normalized_host.ends_with(".internal")
    {
        bail!("add-on manifest URLs cannot target local network hostnames");
    }
    if let Ok(address) = normalized_host.parse::<IpAddr>()
        && is_disallowed_ip(address)
    {
        bail!("add-on manifest URLs cannot target local or reserved IP addresses");
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
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.octets()[0] == 0
        }
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            let unique_local = octets[0] & 0xfe == 0xfc;
            let link_local = octets[0] == 0xfe && octets[1] & 0xc0 == 0x80;
            let ipv4_mapped = octets[..10].iter().all(|byte| *byte == 0)
                && octets[10] == 0xff
                && octets[11] == 0xff;
            let mapped_ipv4 = ipv4_mapped.then(|| {
                std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15])
            });
            ip.is_unspecified()
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
    fn accepts_public_https_manifest_hosts() {
        assert!(normalize_manifest_url("https://addons.example.org/manifest.json").is_ok());
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
}
