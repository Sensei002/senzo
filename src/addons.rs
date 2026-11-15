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
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default, rename = "behaviorHints")]
    pub behavior_hints: Option<Value>,
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

    /// Applies Stremio resource-level type/prefix rules, falling back to manifest defaults.
    pub fn supports_resource_for(
        &self,
        resource: &str,
        media_type: &str,
        resource_id: &str,
    ) -> bool {
        self.resources.iter().any(|entry| {
            let (name, types, prefixes): (&str, Vec<&str>, Vec<&str>) = match entry {
                Value::String(name) => (
                    name.as_str(),
                    self.content_types.iter().map(String::as_str).collect(),
                    self.id_prefixes.iter().map(String::as_str).collect(),
                ),
                Value::Object(object) => {
                    let Some(name) = object.get("name").and_then(Value::as_str) else {
                        return false;
                    };
                    let types = object
                        .get("types")
                        .and_then(Value::as_array)
                        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                        .filter(|values| !values.is_empty())
                        .unwrap_or_else(|| self.content_types.iter().map(String::as_str).collect());
                    let prefixes = object
                        .get("idPrefixes")
                        .and_then(Value::as_array)
                        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                        .filter(|values| !values.is_empty())
                        .unwrap_or_else(|| self.id_prefixes.iter().map(String::as_str).collect());
                    (name, types, prefixes)
                }
                _ => return false,
            };
            if name != resource {
                return false;
            }
            let normalized_type = if media_type == "tv" {
                "series"
            } else {
                media_type
            };
            let type_matches = types.is_empty()
                || types.iter().any(|kind| {
                    *kind == normalized_type || (*kind == "tv" && normalized_type == "series")
                });
            let prefix_matches = resource == "catalog"
                || prefixes.is_empty()
                || prefixes
                    .iter()
                    .any(|prefix| resource_id.starts_with(*prefix));
            type_matches && prefix_matches
        })
    }

    pub fn requires_configuration(&self) -> bool {
        self.behavior_hints
            .as_ref()
            .and_then(|hints| hints.get("configurationRequired"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn catalog_requires_user_input(catalog: &Value) -> bool {
        let legacy_required = catalog
            .get("extraRequired")
            .and_then(Value::as_array)
            .is_some_and(|extras| !extras.is_empty());
        let declared_required =
            catalog
                .get("extra")
                .and_then(Value::as_array)
                .is_some_and(|extras| {
                    extras
                        .iter()
                        .any(|extra| extra.get("isRequired").and_then(Value::as_bool) == Some(true))
                });
        legacy_required || declared_required
    }
}

/// Stable per-installation key; configured instances of one provider remain independent.
pub fn addon_instance_id(manifest_id: &str, manifest_url: &str) -> String {
    format!("{}:{}{}", manifest_id.len(), manifest_id, manifest_url)
}

#[derive(Debug, Clone)]
pub struct AddonManifestPreview {
    pub manifest_url: String,
    pub manifest: AddonManifest,
    pub manifest_json: String,
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

/// Selects the first episode advertised by a Stremio series meta response.
/// Falls back to S01E01 only when the provider exposes no episode list.
pub fn first_series_episode_id(meta: &Value, series_id: &str) -> String {
    meta.get("videos")
        .and_then(Value::as_array)
        .and_then(|videos| {
            videos.iter().find_map(|video| {
                let season = video.get("season").and_then(Value::as_u64)?;
                let episode = video.get("episode").and_then(Value::as_u64)?;
                if season == 0 || episode == 0 {
                    return None;
                }
                video
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.trim().is_empty())
                    .map(str::to_owned)
                    .or_else(|| Some(format!("{series_id}:{season}:{episode}")))
            })
        })
        .unwrap_or_else(|| format!("{series_id}:1:1"))
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
    #[serde(default)]
    pub sources: Vec<String>,
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
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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

    /// Fetches a manifest for user review without installing or enabling it.
    pub async fn preview_from_url(&self, input: &str) -> Result<AddonManifestPreview> {
        let manifest_url = normalize_manifest_url(input)?;
        let value = request_json(&manifest_url, MAX_ADDON_MANIFEST_BYTES).await?;
        let manifest: AddonManifest = serde_json::from_value(value.clone())
            .context("the manifest is missing required Stremio add-on fields")?;
        validate_manifest(&manifest)?;
        Ok(AddonManifestPreview {
            manifest_url: manifest_url.to_string(),
            manifest,
            manifest_json: serde_json::to_string(&value)?,
        })
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

        validate_manifest(&manifest)?;

        let stored = StoredAddon {
            id: addon_instance_id(&manifest.id, manifest_url.as_str()),
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
        let requests = addons
            .iter()
            .filter(|addon| addon.enabled)
            .filter_map(
                |addon| match serde_json::from_str::<AddonManifest>(&addon.manifest_json) {
                    Ok(manifest)
                        if manifest.supports_resource_for("stream", media_type, media_id) =>
                    {
                        Some((addon.clone(), None))
                    }
                    Ok(_) => None,
                    Err(error) => Some((
                        addon.clone(),
                        Some(format!("invalid saved manifest: {error}")),
                    )),
                },
            )
            .map(|(addon, manifest_error)| async move {
                if let Some(error) = manifest_error {
                    return AddonStreamResult {
                        addon_id: addon.id,
                        addon_name: addon.name,
                        streams: Vec::new(),
                        error: Some(error),
                    };
                }
                match self.streams(&addon, media_type, media_id).await {
                    Ok(response) => AddonStreamResult {
                        addon_id: addon.id,
                        addon_name: addon.name,
                        streams: response.streams,
                        error: None,
                    },
                    Err(error) => AddonStreamResult {
                        addon_id: addon.id,
                        addon_name: addon.name,
                        streams: Vec::new(),
                        error: Some(format!("{error:#}")),
                    },
                }
            });
        futures::future::join_all(requests).await
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

    /// Refreshes a manifest while preserving its configured URL, enabled state, install date,
    /// and stable per-installation identity.
    pub async fn refresh_manifest(&self, installed: &StoredAddon) -> Result<StoredAddon> {
        let preview = self.preview_from_url(&installed.manifest_url).await?;
        let installed_manifest: AddonManifest = serde_json::from_str(&installed.manifest_json)
            .context("the installed add-on manifest is invalid")?;
        if preview.manifest.id != installed_manifest.id {
            bail!("the refreshed manifest ID does not match the installed add-on identity");
        }
        Ok(StoredAddon {
            id: installed.id.clone(),
            name: preview.manifest.name,
            version: preview.manifest.version,
            manifest_url: preview.manifest_url,
            enabled: installed.enabled,
            manifest_json: preview.manifest_json,
            installed_at: installed.installed_at,
        })
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

fn validate_manifest(manifest: &AddonManifest) -> Result<()> {
    if manifest.id.trim().is_empty()
        || manifest.name.trim().is_empty()
        || manifest.version.trim().is_empty()
    {
        bail!("the add-on manifest must include id, name, and version");
    }
    if manifest.id.len() > 512 || manifest.name.len() > 256 || manifest.version.len() > 64 {
        bail!("the add-on manifest contains an overlong id, name, or version");
    }
    if manifest.resources.is_empty() || manifest.resources.len() > 64 {
        bail!("the add-on manifest must declare between 1 and 64 resources");
    }
    if manifest.catalogs.len() > 256
        || manifest.content_types.len() > 32
        || manifest.id_prefixes.len() > 128
    {
        bail!("the add-on manifest declares too many types, prefixes, or catalogs");
    }
    for resource in &manifest.resources {
        match resource {
            Value::String(name) if !name.trim().is_empty() && name.len() <= 64 => {}
            Value::Object(object)
                if object
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| !name.trim().is_empty() && name.len() <= 64) => {}
            _ => bail!("the add-on manifest contains an invalid resource declaration"),
        }
    }
    for catalog in &manifest.catalogs {
        if !catalog.is_object()
            || catalog
                .get("id")
                .and_then(Value::as_str)
                .is_none_or(|id| id.trim().is_empty() || id.len() > 256)
            || catalog
                .get("type")
                .and_then(Value::as_str)
                .is_none_or(|kind| !matches!(kind, "movie" | "series" | "tv" | "channel"))
        {
            bail!("the add-on manifest contains an invalid catalog declaration");
        }
    }
    Ok(())
}

fn validate_catalog_extra(extra: &[(&str, &str)]) -> Result<()> {
    if extra.len() > 12 {
        bail!("a catalog request cannot contain more than 12 extra properties");
    }
    let mut keys = std::collections::HashSet::new();
    for (key, value) in extra {
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            || value.len() > 256
            || value.chars().any(char::is_control)
            || !keys.insert(*key)
        {
            bail!("catalog extra properties contain an invalid or duplicate key/value");
        }
    }
    Ok(())
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
    if !manifest.supports_resource_for(resource, media_type, resource_id) {
        bail!("the selected add-on does not declare {resource} support for this media type or ID");
    }
    validate_catalog_extra(extra)?;

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
    if input.trim().len() > 2048 {
        bail!("add-on manifest URLs cannot exceed 2048 characters");
    }
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
    #[test]
    fn first_series_episode_uses_provider_episode_metadata() {
        let meta = serde_json::json!({"videos":[
            {"id":"series:2:3","season":2,"episode":3},
            {"id":"series:2:4","season":2,"episode":4}
        ]});
        assert_eq!(first_series_episode_id(&meta, "series"), "series:2:3");
    }

    #[test]
    fn first_series_episode_falls_back_when_provider_has_no_valid_episodes() {
        let meta = serde_json::json!({"videos":[{"id":"bad","season":0,"episode":0}]});
        assert_eq!(first_series_episode_id(&meta, "series"), "series:1:1");
    }

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
        let second =
            normalize_manifest_url("https://example.org/torrentio/other-config/manifest.json")
                .expect("second config");
        assert_ne!(
            addon_instance_id("org.stremio.torrentio", url.as_str()),
            addon_instance_id("org.stremio.torrentio", second.as_str())
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
            sources: Vec::new(),
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
    fn parses_torrentio_style_manifest_and_stream_payloads() {
        let manifest: AddonManifest = serde_json::from_value(serde_json::json!({
            "id": "org.stremio.torrentio",
            "name": "Torrentio",
            "version": "1.0.0",
            "resources": [
                {"name": "stream", "types": ["movie", "series"], "idPrefixes": ["tt"]}
            ],
            "types": ["movie", "series"],
            "idPrefixes": ["tt"]
        }))
        .expect("Torrentio-compatible manifest");
        assert!(manifest.supports_resource("stream"));

        let response: StremioStreamResponse = serde_json::from_value(serde_json::json!({
            "streams": [
                {
                    "name": "Torrentio",
                    "title": "1080p · HEVC · 8.2 GB",
                    "infoHash": "0123456789abcdef0123456789abcdef01234567",
                    "fileIdx": 3,
                    "sources": ["tracker:udp://tracker.example:1337/announce"]
                },
                {
                    "name": "Direct",
                    "title": "1080p",
                    "url": "https://cdn.example.test/video.mkv"
                }
            ]
        }))
        .expect("Stremio stream response");
        assert_eq!(response.streams.len(), 2);
        assert!(response.streams[0].is_torrent());
        assert_eq!(response.streams[0].file_index, Some(3));
        assert_eq!(
            response.streams[0].sources,
            vec!["tracker:udp://tracker.example:1337/announce"]
        );
        assert_eq!(
            response.streams[1]
                .direct_url()
                .expect("direct URL")
                .expect("URL")
                .scheme(),
            "https"
        );
    }

    #[test]
    fn resource_support_honors_types_and_id_prefixes() {
        let manifest: AddonManifest = serde_json::from_value(serde_json::json!({
            "id": "example",
            "name": "Example",
            "version": "1.0.0",
            "types": ["movie", "series"],
            "idPrefixes": ["tt"],
            "resources": [
                {
                    "name": "stream",
                    "types": ["movie", "series"],
                    "idPrefixes": ["tt"]
                },
                {"name": "meta", "types": ["movie"], "idPrefixes": ["tmdb:"]},
                "catalog"
            ]
        }))
        .expect("manifest");
        assert!(manifest.supports_resource_for("stream", "movie", "tt123"));
        assert!(manifest.supports_resource_for("stream", "series", "tt123:1:2"));
        assert!(!manifest.supports_resource_for("stream", "movie", "tmdb:123"));
        assert!(!manifest.supports_resource_for("stream", "channel", "tt123"));
        assert!(manifest.supports_resource_for("meta", "movie", "tmdb:123"));
        assert!(!manifest.supports_resource_for("meta", "series", "tmdb:123"));
        assert!(manifest.supports_resource_for("catalog", "movie", "popular"));
    }

    #[test]
    fn detects_required_catalog_filters_in_stremio_and_legacy_shapes() {
        assert!(AddonManifest::catalog_requires_user_input(
            &serde_json::json!({
                "extra": [{"name": "genre", "isRequired": true}]
            })
        ));
        assert!(AddonManifest::catalog_requires_user_input(
            &serde_json::json!({
                "extraRequired": ["genre"]
            })
        ));
        assert!(!AddonManifest::catalog_requires_user_input(
            &serde_json::json!({
                "extra": [{"name": "genre", "isRequired": false}]
            })
        ));
    }

    #[test]
    fn catalog_extra_properties_are_bounded_and_unique() {
        assert!(validate_catalog_extra(&[("genre", "Action"), ("skip", "20")]).is_ok());
        assert!(validate_catalog_extra(&[("genre", "Action"), ("genre", "Comedy")]).is_err());
        assert!(validate_catalog_extra(&[("bad/key", "Action")]).is_err());
        assert!(validate_catalog_extra(&[("genre", &"x".repeat(257))]).is_err());
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
            sources: Vec::new(),
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
