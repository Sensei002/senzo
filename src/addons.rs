use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

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
        let response = self
            .client
            .get(manifest_url.clone())
            .send()
            .await
            .context("failed to contact the add-on manifest URL")?
            .error_for_status()
            .context("the add-on server returned an unsuccessful status")?;

        let manifest_value: Value = response
            .json()
            .await
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
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("add-on URLs must use HTTP or HTTPS");
    }

    if !url.path().ends_with("manifest.json") {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid add-on URL"))?;
        segments.pop_if_empty().push("manifest.json");
    }
    Ok(url)
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
    fn rejects_non_http_addon_urls() {
        assert!(normalize_manifest_url("file:///tmp/manifest.json").is_err());
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
