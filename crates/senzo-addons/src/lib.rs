//! Secure, profile-scoped Stremio HTTP add-on support.
//!
//! The crate owns manifest validation, explicit installation, capability matching,
//! transport URL construction, bounded HTTP reads, and catalog/meta/stream/subtitle
//! response validation. Torrent identifiers are parsed only; transport is a later phase.

use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use reqwest::{Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::{net::lookup_host, time::timeout};
use url::{Host, Url};
use uuid::Uuid;

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_RESOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_URL_LENGTH: usize = 4096;
const MAX_REDIRECTS: usize = 4;
const MAX_CATALOGS: usize = 128;
const MAX_RESOURCES: usize = 32;
const MAX_TYPES: usize = 32;
const MAX_ID_PREFIXES: usize = 64;
const MAX_EXTRA_PROPERTIES: usize = 32;
const MAX_EXTRA_OPTIONS: usize = 100;
const MAX_EXTRA_VALUE_LENGTH: usize = 1024;

#[derive(Debug, Error)]
pub enum AddonError {
    #[error("add-on URL must be an absolute HTTP(S) URL without credentials or a fragment")]
    InvalidUrl,
    #[error("HTTP add-ons require explicit approval because the connection is not encrypted")]
    InsecureTransportNotApproved,
    #[error("add-on URL points to a local, private, or reserved network destination")]
    BlockedDestination,
    #[error("add-on host could not be resolved safely")]
    DnsResolution,
    #[error("add-on request failed")]
    Network,
    #[error("add-on returned HTTP {0}")]
    HttpStatus(u16),
    #[error("add-on response exceeded the {0}-byte safety limit")]
    ResponseTooLarge(usize),
    #[error("add-on response was not valid UTF-8 JSON")]
    InvalidResponse,
    #[error("add-on manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("add-on is already installed in this profile")]
    AlreadyInstalled,
    #[error("add-on was not found in this profile")]
    NotFound,
    #[error("add-on is disabled")]
    Disabled,
    #[error("the add-on does not support this resource, media type, or identifier")]
    UnsupportedResource,
    #[error("catalogue {0} is not declared by the add-on manifest")]
    UnknownCatalog(String),
    #[error("catalogue extra properties are missing or invalid: {0}")]
    InvalidCatalogExtras(String),
    #[error("the refreshed add-on manifest changed identity; the previous manifest was retained")]
    ManifestIdentityChanged,
    #[error("the operating-system credential store is unavailable: {0}")]
    Credential(String),
    #[error("the local add-on registry is unavailable: {0}")]
    Repository(String),
    #[error("profile identifier is invalid")]
    InvalidProfile,
}

#[derive(Debug, Error)]
pub enum AddonStoreError {
    #[error("add-on registry operation failed: {0}")]
    Unavailable(String),
    #[error("add-on registry record was not found")]
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonResource {
    pub name: String,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub id_prefixes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonExtraProperty {
    pub name: String,
    #[serde(default)]
    pub is_required: bool,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub options_limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddonCatalog {
    #[serde(rename = "type")]
    pub media_type: String,
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub extra: Vec<AddonExtraProperty>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonBehaviorHints {
    #[serde(default)]
    pub configurable: bool,
    #[serde(default)]
    pub configuration_required: bool,
    #[serde(default)]
    pub adult: bool,
    #[serde(default)]
    pub p2p: bool,
    #[serde(default)]
    pub epg_provider: bool,
}

/// Manifest fields are persisted locally, but the configured transport URL is not.
/// The URL (which may contain a secret configuration token) lives in the OS keyring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonManifest {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub logo: Option<String>,
    pub resources: Vec<AddonResource>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub id_prefixes: Vec<String>,
    #[serde(default)]
    pub catalogs: Vec<AddonCatalog>,
    #[serde(default)]
    pub behavior_hints: AddonBehaviorHints,
}

impl AddonManifest {
    /// Checks a resource against its declared type and optional media-ID prefixes.
    /// Prefix filters apply to media resources, not catalogue IDs.
    pub fn supports_resource(&self, resource: &str, media_type: &str, media_id: &str) -> bool {
        self.resources.iter().any(|candidate| {
            candidate.name == resource
                && candidate.types.iter().any(|kind| kind == media_type)
                && (resource == "catalog"
                    || candidate.id_prefixes.is_empty()
                    || candidate
                        .id_prefixes
                        .iter()
                        .any(|prefix| media_id.starts_with(prefix)))
        })
    }

    pub fn catalogs_for(&self, media_type: &str) -> Vec<AddonCatalog> {
        if !self.supports_resource("catalog", media_type, "") {
            return Vec::new();
        }
        self.catalogs
            .iter()
            .filter(|catalog| catalog.media_type == media_type)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct AddonPreview {
    /// The configured URL is intentionally private so UI code cannot accidentally render or log it.
    manifest_url: String,
    pub display_address: String,
    pub manifest: AddonManifest,
    pub allow_insecure_http: bool,
}

impl AddonPreview {
    pub fn requires_configuration(&self) -> bool {
        self.manifest.behavior_hints.configuration_required
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAddon {
    pub id: String,
    pub display_address: String,
    pub manifest: Option<AddonManifest>,
    pub enabled: bool,
    pub sort_order: i64,
    pub last_checked_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddonRecord {
    pub id: String,
    pub secret_key: String,
    pub manifest_json: Option<String>,
    pub allow_insecure_http: bool,
    pub enabled: bool,
    pub sort_order: i64,
    pub last_checked_at: Option<String>,
    pub last_error: Option<String>,
}

pub trait AddonStore: Send + Sync {
    fn list(&self, profile_id: &str) -> Result<Vec<AddonRecord>, AddonStoreError>;
    fn insert(&self, profile_id: &str, record: &AddonRecord) -> Result<(), AddonStoreError>;
    fn update(&self, profile_id: &str, record: &AddonRecord) -> Result<(), AddonStoreError>;
    fn remove(&self, profile_id: &str, id: &str) -> Result<(), AddonStoreError>;
}

pub trait AddonSecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AddonCatalogItem {
    pub id: String,
    #[serde(rename = "type")]
    pub media_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl AddonCatalogItem {
    pub fn display_title(&self) -> &str {
        self.title
            .as_deref()
            .or(self.name.as_deref())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Untitled")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AddonStream {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub info_hash: Option<String>,
    #[serde(default)]
    pub file_idx: Option<u32>,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub yt_id: Option<String>,
    #[serde(default)]
    pub behavior_hints: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AddonStreamResults {
    pub streams: Vec<AddonStream>,
    pub rejected_items: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AddonSubtitle {
    #[serde(default)]
    pub id: Option<String>,
    pub url: String,
    #[serde(rename = "lang")]
    pub language: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(rename = "type", default)]
    pub mime_type: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AddonSubtitleResults {
    pub subtitles: Vec<AddonSubtitle>,
    pub rejected_items: usize,
}

#[derive(Clone)]
struct AddonHttpClient;

impl AddonHttpClient {
    async fn get_text(
        &self,
        initial_url: Url,
        allow_http: bool,
        max_bytes: usize,
    ) -> Result<String, AddonError> {
        let mut url = initial_url;
        for redirect_count in 0..=MAX_REDIRECTS {
            let addresses = resolve_public_destination(&url).await?;
            let mut builder = Client::builder()
                .timeout(Duration::from_secs(18))
                .connect_timeout(Duration::from_secs(5))
                .redirect(Policy::none())
                .no_proxy()
                .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")));
            if let Some(Host::Domain(host)) = url.host() {
                builder = builder.resolve_to_addrs(host, &addresses);
            }
            let client = builder.build().map_err(|_| AddonError::Network)?;
            let response = client
                .get(url.clone())
                .header(reqwest::header::ACCEPT, "application/json")
                .send()
                .await
                .map_err(|error| {
                    let _safe_message = error.without_url().to_string();
                    AddonError::Network
                })?;

            if response.status().is_redirection() {
                if redirect_count == MAX_REDIRECTS {
                    return Err(AddonError::Network);
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(AddonError::Network)?;
                let next = url.join(location).map_err(|_| AddonError::InvalidUrl)?;
                validate_url_syntax(&next, allow_http)?;
                if url.scheme() == "https" && next.scheme() == "http" && !allow_http {
                    return Err(AddonError::InsecureTransportNotApproved);
                }
                url = next;
                continue;
            }

            if !response.status().is_success() {
                return Err(AddonError::HttpStatus(response.status().as_u16()));
            }
            if response
                .content_length()
                .is_some_and(|length| length > max_bytes as u64)
            {
                return Err(AddonError::ResponseTooLarge(max_bytes));
            }

            let mut response = response;
            let mut bytes = Vec::with_capacity(
                response.content_length().unwrap_or(0).min(max_bytes as u64) as usize,
            );
            while let Some(chunk) = response.chunk().await.map_err(|_| AddonError::Network)? {
                if bytes.len().saturating_add(chunk.len()) > max_bytes {
                    return Err(AddonError::ResponseTooLarge(max_bytes));
                }
                bytes.extend_from_slice(&chunk);
            }
            return String::from_utf8(bytes).map_err(|_| AddonError::InvalidResponse);
        }
        Err(AddonError::Network)
    }

    async fn validate_media_url(&self, value: &str) -> Result<String, AddonError> {
        if value.len() > MAX_URL_LENGTH || value.chars().any(char::is_control) {
            return Err(AddonError::InvalidUrl);
        }
        let url = Url::parse(value).map_err(|_| AddonError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AddonError::InvalidUrl);
        }
        validate_host_syntax(&url)?;
        resolve_public_destination(&url).await?;
        Ok(url.to_string())
    }

    async fn validate_stream_url(&self, value: &str) -> Result<String, AddonError> {
        if value.len() > MAX_URL_LENGTH || value.chars().any(char::is_control) {
            return Err(AddonError::InvalidUrl);
        }
        let parsed = Url::parse(value).map_err(|_| AddonError::InvalidUrl)?;
        if parsed.scheme() == "magnet" {
            return validate_magnet_url(value);
        }
        self.validate_media_url(value).await
    }
}

#[derive(Clone)]
pub struct AddonManager {
    store: Arc<dyn AddonStore>,
    secrets: Arc<dyn AddonSecretStore>,
    http: AddonHttpClient,
}

impl AddonManager {
    pub fn new(store: Arc<dyn AddonStore>, secrets: Arc<dyn AddonSecretStore>) -> Self {
        Self {
            store,
            secrets,
            http: AddonHttpClient,
        }
    }

    /// Fetches and validates a manifest without installing it or changing profile state.
    pub async fn preview_addon(
        &self,
        raw_url: &str,
        allow_insecure_http: bool,
    ) -> Result<AddonPreview, AddonError> {
        let url = normalize_manifest_url(raw_url, allow_insecure_http)?;
        let payload = self
            .http
            .get_text(url.clone(), allow_insecure_http, MAX_MANIFEST_BYTES)
            .await?;
        let manifest = parse_manifest(&url, &payload)?;
        Ok(AddonPreview {
            display_address: redacted_url(&url),
            manifest_url: url.to_string(),
            manifest,
            allow_insecure_http,
        })
    }

    /// Confirms installation in exactly one profile. Configured URLs are held in the OS keyring.
    pub fn install_preview(
        &self,
        profile_id: &str,
        preview: &AddonPreview,
    ) -> Result<ManagedAddon, AddonError> {
        validate_profile_id(profile_id)?;
        let existing = self.store.list(profile_id).map_err(map_store_error)?;
        for record in &existing {
            if self
                .secrets
                .get(&record.secret_key)
                .map_err(AddonError::Credential)?
                .as_deref()
                == Some(preview.manifest_url.as_str())
            {
                return Err(AddonError::AlreadyInstalled);
            }
        }

        let id = Uuid::new_v4().to_string();
        let secret_key = format!("senzo.addon.{profile_id}.{id}");
        let manifest_json =
            serde_json::to_string(&preview.manifest).map_err(|_| AddonError::InvalidResponse)?;
        let record = AddonRecord {
            id: id.clone(),
            secret_key: secret_key.clone(),
            manifest_json: Some(manifest_json),
            allow_insecure_http: preview.allow_insecure_http,
            enabled: true,
            sort_order: existing.len() as i64,
            last_checked_at: Some(now_timestamp()),
            last_error: None,
        };
        self.secrets
            .set(&secret_key, &preview.manifest_url)
            .map_err(AddonError::Credential)?;
        if let Err(error) = self.store.insert(profile_id, &record) {
            let _ = self.secrets.delete(&secret_key);
            return Err(map_store_error(error));
        }
        Ok(ManagedAddon {
            id,
            display_address: preview.display_address.clone(),
            manifest: Some(preview.manifest.clone()),
            enabled: true,
            sort_order: record.sort_order,
            last_checked_at: record.last_checked_at,
            last_error: None,
        })
    }

    pub fn list_addons(&self, profile_id: &str) -> Result<Vec<ManagedAddon>, AddonError> {
        validate_profile_id(profile_id)?;
        let mut addons = self
            .store
            .list(profile_id)
            .map_err(map_store_error)?
            .into_iter()
            .map(|record| {
                let address = self
                    .secrets
                    .get(&record.secret_key)
                    .map_err(AddonError::Credential)?
                    .and_then(|raw| Url::parse(&raw).ok())
                    .map(|url| redacted_url(&url))
                    .unwrap_or_else(|| "Saved address unavailable".to_owned());
                let manifest = record
                    .manifest_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str(json).ok());
                Ok(ManagedAddon {
                    id: record.id,
                    display_address: address,
                    manifest,
                    enabled: record.enabled,
                    sort_order: record.sort_order,
                    last_checked_at: record.last_checked_at,
                    last_error: record.last_error,
                })
            })
            .collect::<Result<Vec<_>, AddonError>>()?;
        // Keep the public manager contract independent of storage backend iteration order.
        addons.sort_by(|left, right| {
            left.sort_order
                .cmp(&right.sort_order)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(addons)
    }

    pub fn set_enabled(
        &self,
        profile_id: &str,
        addon_id: &str,
        enabled: bool,
    ) -> Result<(), AddonError> {
        let mut record = self.find_record(profile_id, addon_id)?;
        record.enabled = enabled;
        self.store
            .update(profile_id, &record)
            .map_err(map_store_error)
    }

    pub fn remove_addon(&self, profile_id: &str, addon_id: &str) -> Result<(), AddonError> {
        let record = self.find_record(profile_id, addon_id)?;
        let secret = self
            .secrets
            .get(&record.secret_key)
            .map_err(AddonError::Credential)?;
        if let Some(value) = secret.as_deref() {
            self.secrets
                .delete(&record.secret_key)
                .map_err(AddonError::Credential)?;
            if let Err(error) = self.store.remove(profile_id, addon_id) {
                let _ = self.secrets.set(&record.secret_key, value);
                return Err(map_store_error(error));
            }
        } else {
            self.store
                .remove(profile_id, addon_id)
                .map_err(map_store_error)?;
        }
        Ok(())
    }

    pub fn reorder_addons(
        &self,
        profile_id: &str,
        ordered_ids: &[String],
    ) -> Result<(), AddonError> {
        validate_profile_id(profile_id)?;
        let records = self.store.list(profile_id).map_err(map_store_error)?;
        let unique = ordered_ids.iter().collect::<std::collections::HashSet<_>>();
        if unique.len() != ordered_ids.len()
            || ordered_ids.len() != records.len()
            || records.iter().any(|record| !unique.contains(&record.id))
        {
            return Err(AddonError::Repository(
                "ordering must contain every installed add-on exactly once".to_owned(),
            ));
        }
        for (index, id) in ordered_ids.iter().enumerate() {
            let mut record = records
                .iter()
                .find(|record| &record.id == id)
                .cloned()
                .ok_or(AddonError::NotFound)?;
            record.sort_order = index as i64;
            self.store
                .update(profile_id, &record)
                .map_err(map_store_error)?;
        }
        Ok(())
    }

    /// Refresh failures are isolated: a previous valid manifest and enabled state are retained.
    pub async fn refresh_addon(
        &self,
        profile_id: &str,
        addon_id: &str,
    ) -> Result<ManagedAddon, AddonError> {
        let mut record = self.find_record(profile_id, addon_id)?;
        let raw_url = self
            .secrets
            .get(&record.secret_key)
            .map_err(AddonError::Credential)?
            .ok_or_else(|| AddonError::Credential("saved manifest URL is missing".to_owned()))?;
        let result = async {
            let url = normalize_manifest_url(&raw_url, record.allow_insecure_http)?;
            let payload = self
                .http
                .get_text(url.clone(), record.allow_insecure_http, MAX_MANIFEST_BYTES)
                .await?;
            let manifest = parse_manifest(&url, &payload)?;
            let previous_id = record
                .manifest_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<AddonManifest>(json).ok())
                .map(|manifest| manifest.id);
            if previous_id.is_some_and(|previous| previous != manifest.id) {
                return Err(AddonError::ManifestIdentityChanged);
            }
            record.manifest_json =
                Some(serde_json::to_string(&manifest).map_err(|_| AddonError::InvalidResponse)?);
            record.last_checked_at = Some(now_timestamp());
            record.last_error = None;
            self.store
                .update(profile_id, &record)
                .map_err(map_store_error)?;
            Ok::<AddonManifest, AddonError>(manifest)
        }
        .await;

        match result {
            Ok(manifest) => Ok(ManagedAddon {
                id: record.id,
                display_address: redacted_url(
                    &Url::parse(&raw_url).map_err(|_| AddonError::InvalidUrl)?,
                ),
                manifest: Some(manifest),
                enabled: record.enabled,
                sort_order: record.sort_order,
                last_checked_at: record.last_checked_at,
                last_error: None,
            }),
            Err(error) => {
                record.last_error = Some(error.to_string());
                let _ = self.store.update(profile_id, &record);
                Err(error)
            }
        }
    }

    pub async fn refresh_all(
        &self,
        profile_id: &str,
    ) -> Vec<(String, Result<ManagedAddon, AddonError>)> {
        let ids = match self.store.list(profile_id) {
            Ok(records) => records
                .into_iter()
                .map(|record| record.id)
                .collect::<Vec<_>>(),
            Err(error) => return vec![(String::new(), Err(map_store_error(error)))],
        };
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            results.push((id.clone(), self.refresh_addon(profile_id, &id).await));
        }
        results
    }

    pub fn catalogs(
        &self,
        profile_id: &str,
        addon_id: &str,
        media_type: &str,
    ) -> Result<Vec<AddonCatalog>, AddonError> {
        let (_, manifest, _) = self.active_addon(profile_id, addon_id)?;
        Ok(manifest.catalogs_for(media_type))
    }

    pub async fn fetch_catalog(
        &self,
        profile_id: &str,
        addon_id: &str,
        media_type: &str,
        catalog_id: &str,
        extras: &BTreeMap<String, String>,
    ) -> Result<Vec<AddonCatalogItem>, AddonError> {
        let (url, manifest, allow_http) = self.active_addon(profile_id, addon_id)?;
        let catalog = manifest
            .catalogs
            .iter()
            .find(|catalog| catalog.media_type == media_type && catalog.id == catalog_id)
            .ok_or_else(|| AddonError::UnknownCatalog(catalog_id.to_owned()))?;
        if !manifest.supports_resource("catalog", media_type, catalog_id) {
            return Err(AddonError::UnsupportedResource);
        }
        validate_catalog_extras(catalog, extras)?;
        let request_url = build_resource_url(&url, "catalog", media_type, catalog_id, extras)?;
        let text = self
            .http
            .get_text(request_url, allow_http, MAX_RESOURCE_BYTES)
            .await?;
        let value: Value = serde_json::from_str(&text).map_err(|_| AddonError::InvalidResponse)?;
        let Some(metas) = value.get("metas").and_then(Value::as_array) else {
            return Err(AddonError::InvalidResponse);
        };
        let mut result = Vec::with_capacity(metas.len().min(500));
        for item in metas.iter().take(500) {
            if let Ok(meta) = serde_json::from_value::<AddonCatalogItem>(item.clone())
                && !meta.id.is_empty()
                && meta.media_type == media_type
            {
                result.push(meta);
            }
        }
        Ok(result)
    }

    pub async fn fetch_meta(
        &self,
        profile_id: &str,
        addon_id: &str,
        media_type: &str,
        media_id: &str,
    ) -> Result<Value, AddonError> {
        let (url, manifest, allow_http) = self.active_addon(profile_id, addon_id)?;
        if !manifest.supports_resource("meta", media_type, media_id) {
            return Err(AddonError::UnsupportedResource);
        }
        let request_url = build_resource_url(&url, "meta", media_type, media_id, &BTreeMap::new())?;
        let text = self
            .http
            .get_text(request_url, allow_http, MAX_RESOURCE_BYTES)
            .await?;
        let value: Value = serde_json::from_str(&text).map_err(|_| AddonError::InvalidResponse)?;
        let meta = value
            .get("meta")
            .filter(|meta| meta.is_object())
            .cloned()
            .ok_or(AddonError::InvalidResponse)?;
        let returned_id = meta.get("id").and_then(Value::as_str);
        let returned_type = meta.get("type").and_then(Value::as_str);
        if returned_id != Some(media_id) || returned_type != Some(media_type) {
            return Err(AddonError::InvalidResponse);
        }
        Ok(meta)
    }

    pub async fn fetch_streams(
        &self,
        profile_id: &str,
        addon_id: &str,
        media_type: &str,
        video_id: &str,
    ) -> Result<AddonStreamResults, AddonError> {
        let (url, manifest, allow_http) = self.active_addon(profile_id, addon_id)?;
        if !manifest.supports_resource("stream", media_type, video_id) {
            return Err(AddonError::UnsupportedResource);
        }
        let request_url =
            build_resource_url(&url, "stream", media_type, video_id, &BTreeMap::new())?;
        let text = self
            .http
            .get_text(request_url, allow_http, MAX_RESOURCE_BYTES)
            .await?;
        let value: Value = serde_json::from_str(&text).map_err(|_| AddonError::InvalidResponse)?;
        let Some(items) = value.get("streams").and_then(Value::as_array) else {
            return Err(AddonError::InvalidResponse);
        };
        let mut streams = Vec::new();
        let mut rejected_items = 0;
        for item in items.iter().take(250) {
            let Ok(mut stream) = serde_json::from_value::<AddonStream>(item.clone()) else {
                rejected_items += 1;
                continue;
            };
            if let Some(url) = stream.url.as_deref() {
                match self.http.validate_stream_url(url).await {
                    Ok(valid) => stream.url = Some(valid),
                    Err(_) => {
                        rejected_items += 1;
                        continue;
                    }
                }
            }
            if let Some(url) = stream.external_url.as_deref() {
                match self.http.validate_media_url(url).await {
                    Ok(valid) => stream.external_url = Some(valid),
                    Err(_) => {
                        rejected_items += 1;
                        continue;
                    }
                }
            }
            if stream
                .info_hash
                .as_deref()
                .is_some_and(|hash| !valid_info_hash(hash))
            {
                rejected_items += 1;
                continue;
            }
            if stream.url.is_none()
                && stream.info_hash.is_none()
                && stream.external_url.is_none()
                && stream.yt_id.is_none()
            {
                rejected_items += 1;
                continue;
            }
            streams.push(stream);
        }
        Ok(AddonStreamResults {
            streams,
            rejected_items,
        })
    }

    pub async fn fetch_subtitles(
        &self,
        profile_id: &str,
        addon_id: &str,
        media_type: &str,
        subtitle_id: &str,
        extra_args: &BTreeMap<String, String>,
    ) -> Result<AddonSubtitleResults, AddonError> {
        let (url, manifest, allow_http) = self.active_addon(profile_id, addon_id)?;
        if !manifest.supports_resource("subtitles", media_type, subtitle_id) {
            return Err(AddonError::UnsupportedResource);
        }
        validate_generic_extras(extra_args)?;
        let request_url =
            build_resource_url(&url, "subtitles", media_type, subtitle_id, extra_args)?;
        let text = self
            .http
            .get_text(request_url, allow_http, MAX_RESOURCE_BYTES)
            .await?;
        let value: Value = serde_json::from_str(&text).map_err(|_| AddonError::InvalidResponse)?;
        let Some(items) = value.get("subtitles").and_then(Value::as_array) else {
            return Err(AddonError::InvalidResponse);
        };
        let mut subtitles = Vec::new();
        let mut rejected_items = 0;
        for item in items.iter().take(250) {
            let Ok(mut subtitle) = serde_json::from_value::<AddonSubtitle>(item.clone()) else {
                rejected_items += 1;
                continue;
            };
            match self.http.validate_media_url(&subtitle.url).await {
                Ok(valid) => subtitle.url = valid,
                Err(_) => {
                    rejected_items += 1;
                    continue;
                }
            }
            if subtitle.language.trim().is_empty() {
                rejected_items += 1;
                continue;
            }
            subtitles.push(subtitle);
        }
        Ok(AddonSubtitleResults {
            subtitles,
            rejected_items,
        })
    }

    fn find_record(&self, profile_id: &str, addon_id: &str) -> Result<AddonRecord, AddonError> {
        validate_profile_id(profile_id)?;
        self.store
            .list(profile_id)
            .map_err(map_store_error)?
            .into_iter()
            .find(|record| record.id == addon_id)
            .ok_or(AddonError::NotFound)
    }

    fn active_addon(
        &self,
        profile_id: &str,
        addon_id: &str,
    ) -> Result<(Url, AddonManifest, bool), AddonError> {
        let record = self.find_record(profile_id, addon_id)?;
        if !record.enabled {
            return Err(AddonError::Disabled);
        }
        let manifest_json = record
            .manifest_json
            .as_deref()
            .ok_or(AddonError::InvalidResponse)?;
        let manifest = serde_json::from_str::<AddonManifest>(manifest_json)
            .map_err(|_| AddonError::InvalidResponse)?;
        let raw_url = self
            .secrets
            .get(&record.secret_key)
            .map_err(AddonError::Credential)?
            .ok_or_else(|| AddonError::Credential("saved manifest URL is missing".to_owned()))?;
        let url = normalize_manifest_url(&raw_url, record.allow_insecure_http)?;
        Ok((url, manifest, record.allow_insecure_http))
    }
}

pub fn parse_manifest(_manifest_url: &Url, payload: &str) -> Result<AddonManifest, AddonError> {
    if payload.len() > MAX_MANIFEST_BYTES {
        return Err(AddonError::ResponseTooLarge(MAX_MANIFEST_BYTES));
    }
    let root: Value = serde_json::from_str(payload)
        .map_err(|_| AddonError::InvalidManifest("JSON is malformed".to_owned()))?;
    let object = root
        .as_object()
        .ok_or_else(|| AddonError::InvalidManifest("the root must be an object".to_owned()))?;

    let id = required_string(object, "id")?;
    let name = required_string(object, "name")?;
    let version = required_string(object, "version")?;
    let description = required_string(object, "description")?;
    validate_text("id", &id, 256)?;
    validate_text("name", &name, 256)?;
    validate_text("version", &version, 64)?;
    validate_text("description", &description, 4096)?;

    let types = string_array(object.get("types"), MAX_TYPES)?;
    if types.is_empty() {
        return Err(AddonError::InvalidManifest(
            "at least one supported media type is required".to_owned(),
        ));
    }
    let id_prefixes = string_array(object.get("idPrefixes"), MAX_ID_PREFIXES)?;
    let resources_value = object
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| AddonError::InvalidManifest("resources must be an array".to_owned()))?;
    if resources_value.is_empty() || resources_value.len() > MAX_RESOURCES {
        return Err(AddonError::InvalidManifest(
            "resources count is invalid".to_owned(),
        ));
    }
    let mut resources = Vec::with_capacity(resources_value.len());
    for resource in resources_value {
        let (resource_name, resource_types, prefixes) = match resource {
            Value::String(value) => (value.clone(), types.clone(), id_prefixes.clone()),
            Value::Object(resource) => {
                let name = required_string(resource, "name")?;
                // Object-form resources define their own type scope. An omitted
                // idPrefixes list means all IDs, rather than inheriting the global list.
                let local_types = string_array(resource.get("types"), MAX_TYPES)?;
                let local_prefixes = string_array(resource.get("idPrefixes"), MAX_ID_PREFIXES)?;
                (name, local_types, local_prefixes)
            }
            _ => {
                return Err(AddonError::InvalidManifest(
                    "resource entries must be strings or objects".to_owned(),
                ));
            }
        };
        validate_text("resource name", &resource_name, 64)?;
        if resource_types.is_empty() {
            return Err(AddonError::InvalidManifest(
                "every resource must support at least one type".to_owned(),
            ));
        }
        if resources.iter().any(|existing: &AddonResource| existing.name == resource_name) {
            return Err(AddonError::InvalidManifest(
                "resource names must be unique".to_owned(),
            ));
        }
        resources.push(AddonResource {
            name: resource_name,
            types: resource_types,
            id_prefixes: prefixes,
        });
    }

    let catalogs_value = object
        .get("catalogs")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| AddonError::InvalidManifest("catalogs must be an array".to_owned()))?;
    if catalogs_value.len() > MAX_CATALOGS {
        return Err(AddonError::InvalidManifest(
            "catalogue count exceeds the limit".to_owned(),
        ));
    }
    let mut catalogs = Vec::with_capacity(catalogs_value.len());
    for catalog in catalogs_value {
        let catalog_object = catalog.as_object().ok_or_else(|| {
            AddonError::InvalidManifest("catalogue entries must be objects".to_owned())
        })?;
        let media_type = required_string(catalog_object, "type")?;
        let catalog_id = required_string(catalog_object, "id")?;
        validate_text("catalogue type", &media_type, 64)?;
        validate_text("catalogue id", &catalog_id, 256)?;
        if !types.contains(&media_type) {
            return Err(AddonError::InvalidManifest(
                "catalogue type must be declared in the manifest types".to_owned(),
            ));
        }
        if catalogs.iter().any(|catalog: &AddonCatalog| catalog.id == catalog_id) {
            return Err(AddonError::InvalidManifest(
                "catalogue IDs must be unique within an add-on".to_owned(),
            ));
        }
        let catalog_name = catalog_object
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&catalog_id)
            .to_owned();
        validate_text("catalogue name", &catalog_name, 256)?;
        let extra_value = catalog_object
            .get("extra")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if extra_value.len() > MAX_EXTRA_PROPERTIES {
            return Err(AddonError::InvalidManifest(
                "catalogue extra-property count exceeds the limit".to_owned(),
            ));
        }
        let mut extra = Vec::with_capacity(extra_value.len());
        for property in extra_value {
            let property = property.as_object().ok_or_else(|| {
                AddonError::InvalidManifest("catalogue extra properties must be objects".to_owned())
            })?;
            let property_name = required_string(property, "name")?;
            validate_text("extra-property name", &property_name, 64)?;
            let options = string_array(property.get("options"), MAX_EXTRA_OPTIONS)?;
            let options_limit = property
                .get("optionsLimit")
                .and_then(Value::as_u64)
                .map(|number| number as usize)
                .or_else(|| (!options.is_empty()).then_some(1));
            if options_limit.is_some_and(|limit| {
                limit == 0 || (!options.is_empty() && limit > options.len())
            }) {
                return Err(AddonError::InvalidManifest(
                    "catalogue optionsLimit is outside the declared options".to_owned(),
                ));
            }
            extra.push(AddonExtraProperty {
                name: property_name,
                is_required: property
                    .get("isRequired")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                options,
                options_limit,
            });
        }
        catalogs.push(AddonCatalog {
            media_type,
            id: catalog_id,
            name: catalog_name,
            extra,
        });
    }

    // Never resolve relative artwork against a configured manifest URL: its path
    // can contain a private configuration token that must stay in the OS keyring.
    let logo = object
        .get("logo")
        .and_then(Value::as_str)
        .and_then(|value| Url::parse(value).ok())
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(|mut url| {
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        });
    let behavior = object.get("behaviorHints").and_then(Value::as_object);
    let behavior_hints = AddonBehaviorHints {
        configurable: behavior
            .and_then(|value| value.get("configurable"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        configuration_required: behavior
            .and_then(|value| value.get("configurationRequired"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        adult: behavior
            .and_then(|value| value.get("adult"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        p2p: behavior
            .and_then(|value| value.get("p2p"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        epg_provider: behavior
            .and_then(|value| value.get("epgProvider"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };

    Ok(AddonManifest {
        id,
        version,
        name,
        description,
        logo,
        resources,
        types,
        id_prefixes,
        catalogs,
        behavior_hints,
    })
}

pub fn normalize_manifest_url(raw: &str, allow_insecure_http: bool) -> Result<Url, AddonError> {
    if raw.trim() != raw
        || raw.is_empty()
        || raw.len() > MAX_URL_LENGTH
        || raw.chars().any(char::is_control)
    {
        return Err(AddonError::InvalidUrl);
    }
    let mut url = Url::parse(raw).map_err(|_| AddonError::InvalidUrl)?;
    validate_url_syntax(&url, allow_insecure_http)?;
    if !url.path().ends_with("/manifest.json") && url.path() != "/manifest.json" {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| AddonError::InvalidUrl)?;
        segments.pop_if_empty().push("manifest.json");
    }
    if url.as_str().len() > MAX_URL_LENGTH {
        return Err(AddonError::InvalidUrl);
    }
    Ok(url)
}

pub fn redacted_url(url: &Url) -> String {
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("unknown host");
    match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}/…"),
        None => format!("{scheme}://{host}/…"),
    }
}

fn validate_url_syntax(url: &Url, allow_insecure_http: bool) -> Result<(), AddonError> {
    match url.scheme() {
        "https" => {}
        "http" if allow_insecure_http => {}
        "http" => return Err(AddonError::InsecureTransportNotApproved),
        _ => return Err(AddonError::InvalidUrl),
    }
    validate_host_syntax(url)
}

fn validate_host_syntax(url: &Url) -> Result<(), AddonError> {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(AddonError::InvalidUrl);
    }
    match url.host().ok_or(AddonError::InvalidUrl)? {
        Host::Ipv4(address) if !is_public_ip(IpAddr::V4(address)) => {
            Err(AddonError::BlockedDestination)
        }
        Host::Ipv6(address) if !is_public_ip(IpAddr::V6(address)) => {
            Err(AddonError::BlockedDestination)
        }
        Host::Domain(host) => {
            let host = host.to_ascii_lowercase();
            if host == "localhost"
                || host.ends_with(".localhost")
                || host.ends_with(".local")
                || host.ends_with(".internal")
                || host.ends_with(".lan")
                || host.ends_with(".home")
                || host.ends_with(".test")
            {
                return Err(AddonError::BlockedDestination);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn resolve_public_destination(url: &Url) -> Result<Vec<SocketAddr>, AddonError> {
    validate_host_syntax(url)?;
    let port = url.port_or_known_default().ok_or(AddonError::InvalidUrl)?;
    match url.host().ok_or(AddonError::InvalidUrl)? {
        Host::Ipv4(address) => Ok(vec![SocketAddr::new(IpAddr::V4(address), port)]),
        Host::Ipv6(address) => Ok(vec![SocketAddr::new(IpAddr::V6(address), port)]),
        Host::Domain(host) => {
            let resolved = timeout(Duration::from_secs(4), lookup_host((host, port)))
                .await
                .map_err(|_| AddonError::DnsResolution)?
                .map_err(|_| AddonError::DnsResolution)?
                .collect::<Vec<_>>();
            if resolved.is_empty() || resolved.iter().any(|address| !is_public_ip(address.ip())) {
                return Err(AddonError::BlockedDestination);
            }
            Ok(resolved)
        }
    }
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    if segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xffff {
        let mapped = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        );
        return is_public_ipv4(mapped);
    }
    let global_unicast = segments[0] & 0xe000 == 0x2000;
    global_unicast
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && !(segments[0] == 0x2001 && segments[1] == 0x0000)
        && segments[0] != 0x2002
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<String, AddonError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AddonError::InvalidManifest(format!("missing or invalid {field}")))
}

fn string_array(value: Option<&Value>, maximum: usize) -> Result<Vec<String>, AddonError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let array = value.as_array().ok_or_else(|| {
        AddonError::InvalidManifest("a list-valued field is not an array".to_owned())
    })?;
    if array.len() > maximum {
        return Err(AddonError::InvalidManifest(
            "a list exceeds its size limit".to_owned(),
        ));
    }
    let mut values = Vec::new();
    for item in array {
        let value = item.as_str().ok_or_else(|| {
            AddonError::InvalidManifest("list entries must be strings".to_owned())
        })?;
        let value = value.trim();
        if value.is_empty() || value.chars().any(char::is_control) || value.len() > 256 {
            return Err(AddonError::InvalidManifest(
                "list entry is invalid".to_owned(),
            ));
        }
        if !values.iter().any(|existing| existing == value) {
            values.push(value.to_owned());
        }
    }
    Ok(values)
}

fn validate_text(field: &str, value: &str, max_length: usize) -> Result<(), AddonError> {
    if value.trim().is_empty() || value.len() > max_length || value.chars().any(char::is_control) {
        return Err(AddonError::InvalidManifest(format!("{field} is invalid")));
    }
    Ok(())
}

fn validate_profile_id(profile_id: &str) -> Result<(), AddonError> {
    if profile_id.is_empty()
        || profile_id.len() > 128
        || !profile_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(AddonError::InvalidProfile);
    }
    Ok(())
}

fn validate_catalog_extras(
    catalog: &AddonCatalog,
    provided: &BTreeMap<String, String>,
) -> Result<(), AddonError> {
    validate_generic_extras(provided)?;
    for extra in provided.keys() {
        if !catalog.extra.iter().any(|declared| &declared.name == extra) {
            return Err(AddonError::InvalidCatalogExtras(format!(
                "undeclared property {extra}"
            )));
        }
    }
    for declared in &catalog.extra {
        match provided.get(&declared.name) {
            Some(value) => {
                if !declared.options.is_empty()
                    && !value
                        .split(',')
                        .all(|option| declared.options.iter().any(|item| item == option))
                {
                    return Err(AddonError::InvalidCatalogExtras(format!(
                        "unsupported value for {}",
                        declared.name
                    )));
                }
                if declared
                    .options_limit
                    .is_some_and(|limit| value.split(',').count() > limit)
                {
                    return Err(AddonError::InvalidCatalogExtras(format!(
                        "too many values for {}",
                        declared.name
                    )));
                }
            }
            None if declared.is_required => {
                return Err(AddonError::InvalidCatalogExtras(format!(
                    "required property {} is missing",
                    declared.name
                )));
            }
            None => {}
        }
    }
    Ok(())
}

fn validate_generic_extras(extras: &BTreeMap<String, String>) -> Result<(), AddonError> {
    if extras.len() > MAX_EXTRA_PROPERTIES {
        return Err(AddonError::InvalidCatalogExtras(
            "too many extra properties".to_owned(),
        ));
    }
    for (key, value) in extras {
        if key.is_empty()
            || key.len() > 64
            || value.len() > MAX_EXTRA_VALUE_LENGTH
            || key.chars().any(char::is_control)
            || value.chars().any(char::is_control)
        {
            return Err(AddonError::InvalidCatalogExtras(
                "an extra property is too long or contains control characters".to_owned(),
            ));
        }
    }
    Ok(())
}

fn build_resource_url(
    manifest_url: &Url,
    resource: &str,
    media_type: &str,
    id: &str,
    extras: &BTreeMap<String, String>,
) -> Result<Url, AddonError> {
    if !matches!(resource, "catalog" | "meta" | "stream" | "subtitles")
        || media_type.is_empty()
        || id.is_empty()
        || media_type.len() > 64
        || id.len() > 1024
        || media_type.chars().any(char::is_control)
        || id.chars().any(char::is_control)
    {
        return Err(AddonError::UnsupportedResource);
    }
    validate_generic_extras(extras)?;
    let mut url = manifest_url.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| AddonError::InvalidUrl)?;
        segments.pop_if_empty().pop();
        segments.push(resource).push(media_type).push(id);
        if extras.is_empty() {
            // Replace the ID segment with the protocol's required .json suffix.
            segments.pop().push(&format!("{id}.json"));
        }
    }
    if !extras.is_empty() {
        // Stremio represents extra arguments as one final path segment, with
        // literal '&' and '=' separators and individually percent-encoded values.
        let extra_path = extras
            .iter()
            .map(|(key, value)| {
                format!(
                    "{}={}",
                    encode_addon_path_component(key),
                    encode_addon_path_component(value)
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        let path = format!("{}/{}.json", url.path().trim_end_matches('/'), extra_path);
        url.set_path(&path);
    }
    Ok(url)
}

fn encode_addon_path_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn validate_magnet_url(value: &str) -> Result<String, AddonError> {
    if value.len() > MAX_URL_LENGTH || value.chars().any(char::is_control) {
        return Err(AddonError::InvalidUrl);
    }
    let url = Url::parse(value).map_err(|_| AddonError::InvalidUrl)?;
    if url.scheme() != "magnet" || url.fragment().is_some() {
        return Err(AddonError::InvalidUrl);
    }
    let has_valid_v1_hash = url.query_pairs().any(|(key, value)| {
        key == "xt" && value.strip_prefix("urn:btih:").is_some_and(valid_info_hash)
    });
    if !has_valid_v1_hash {
        return Err(AddonError::InvalidUrl);
    }
    // This is retained as source metadata only. Torrent transport is deliberately deferred.
    Ok(url.to_string())
}

fn valid_info_hash(hash: &str) -> bool {
    (hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || (hash.len() == 32
            && hash.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    && !matches!(byte.to_ascii_uppercase(), b'0' | b'1' | b'8' | b'9')
            }))
}

fn now_timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn map_store_error(error: AddonStoreError) -> AddonError {
    match error {
        AddonStoreError::Unavailable(message) => AddonError::Repository(message),
        AddonStoreError::NotFound => AddonError::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const MANIFEST: &str = r#"{
        "id":"org.example.addon",
        "version":"1.2.3",
        "name":"Example Provider",
        "description":"A fixture manifest",
        "types":["movie","series"],
        "idPrefixes":["tt"],
        "resources":[
            "catalog",
            "meta",
            {"name":"stream","types":["movie","series"],"idPrefixes":["tt"]},
            "subtitles"
        ],
        "catalogs":[
            {"type":"movie","id":"popular","name":"Popular","extra":[
                {"name":"genre","isRequired":true,"options":["action","comedy"]},
                {"name":"skip","optionsLimit":1}
            ]}
        ],
        "behaviorHints":{"configurable":true,"configurationRequired":true,"p2p":true}
    }"#;

    #[test]
    fn parses_a_stremio_manifest_and_matches_resource_capabilities() {
        let url = Url::parse("https://addons.example/manifest.json?config=secret").unwrap();
        let manifest = parse_manifest(&url, MANIFEST).unwrap();
        assert_eq!(manifest.id, "org.example.addon");
        assert_eq!(manifest.name, "Example Provider");
        assert!(manifest.behavior_hints.configuration_required);
        assert!(manifest.supports_resource("catalog", "movie", "popular"));
        assert!(manifest.supports_resource("stream", "movie", "tt1234567"));
        assert!(!manifest.supports_resource("stream", "movie", "kitsu:123"));
        assert!(manifest.supports_resource("subtitles", "series", "tt123"));
        assert_eq!(manifest.catalogs_for("movie").len(), 1);
    }

    #[test]
    fn rejects_malformed_or_incomplete_manifests() {
        let url = Url::parse("https://addons.example/manifest.json").unwrap();
        assert!(parse_manifest(&url, "{").is_err());
        assert!(parse_manifest(&url, r#"{"name":"Missing fields"}"#).is_err());
        let missing_description = r#"{"id":"x","name":"x","version":"1","types":["movie"],"resources":["catalog"],"catalogs":[]}"#;
        assert!(parse_manifest(&url, missing_description).is_err());
        let no_resources =
            r#"{"id":"x","name":"x","version":"1","types":["movie"],"resources":[]}"#;
        assert!(parse_manifest(&url, no_resources).is_err());
    }

    #[test]
    fn resource_objects_override_global_types_and_id_prefixes() {
        let url = Url::parse("https://addons.example/manifest.json").unwrap();
        let manifest = r#"{
            "id":"org.example.resource-scope",
            "version":"1.0.0",
            "name":"Resource scope",
            "description":"Per-resource matching rules",
            "types":["movie","series"],
            "idPrefixes":["tt"],
            "resources":[
                {"name":"stream","types":["movie"]},
                "meta"
            ],
            "catalogs":[]
        }"#;
        let parsed = parse_manifest(&url, manifest).unwrap();
        assert!(parsed.supports_resource("stream", "movie", "kitsu:123"));
        assert!(!parsed.supports_resource("stream", "series", "tt123"));
        assert!(parsed.supports_resource("meta", "series", "tt123"));
        assert!(!parsed.supports_resource("meta", "series", "kitsu:123"));
    }

    #[test]
    fn normalizes_manifest_urls_without_losing_configuration_query() {
        let url = normalize_manifest_url("https://addons.example/config=abc", false).unwrap();
        assert_eq!(
            url.as_str(),
            "https://addons.example/config=abc/manifest.json"
        );
        let url = normalize_manifest_url("https://addons.example/manifest.json?token=abc", false)
            .unwrap();
        assert_eq!(url.query(), Some("token=abc"));
    }

    #[test]
    fn insecure_or_private_manifest_destinations_require_explicit_approval_or_are_rejected() {
        assert!(matches!(
            normalize_manifest_url("http://addons.example/manifest.json", false),
            Err(AddonError::InsecureTransportNotApproved)
        ));
        assert!(normalize_manifest_url("http://addons.example/manifest.json", true).is_ok());
        assert!(normalize_manifest_url("https://127.0.0.1/manifest.json", true).is_err());
        assert!(normalize_manifest_url("https://localhost/manifest.json", true).is_err());
        assert!(
            normalize_manifest_url("https://user:pass@addons.example/manifest.json", true).is_err()
        );
        assert!(normalize_manifest_url("file:///etc/passwd", true).is_err());
    }

    #[test]
    fn resource_urls_preserve_config_tokens_and_encode_catalog_extras_as_one_segment() {
        let manifest =
            Url::parse("https://addons.example/addon/manifest.json?token=secret").unwrap();
        let extras = BTreeMap::from([
            ("search".to_owned(), "game of thrones".to_owned()),
            ("skip".to_owned(), "100".to_owned()),
        ]);
        let url = build_resource_url(&manifest, "catalog", "series", "top", &extras).unwrap();
        assert!(
            url.path()
                .ends_with("/catalog/series/top/search=game%20of%20thrones&skip=100.json")
        );
        assert_eq!(url.query(), Some("token=secret"));
    }

    #[test]
    fn catalog_extras_enforce_required_properties_options_and_limits() {
        let url = Url::parse("https://addons.example/manifest.json").unwrap();
        let manifest = parse_manifest(&url, MANIFEST).unwrap();
        let catalog = &manifest.catalogs[0];
        assert!(validate_catalog_extras(catalog, &BTreeMap::new()).is_err());
        assert!(
            validate_catalog_extras(
                catalog,
                &BTreeMap::from([
                    ("genre".to_owned(), "action".to_owned()),
                    ("skip".to_owned(), "100".to_owned()),
                ])
            )
            .is_ok()
        );
        assert!(
            validate_catalog_extras(
                catalog,
                &BTreeMap::from([("genre".to_owned(), "horror".to_owned())])
            )
            .is_err()
        );
    }

    #[test]
    fn blocks_private_and_reserved_ip_ranges() {
        for address in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn magnet_stream_metadata_requires_a_valid_btih_hash_without_starting_transport() {
        let valid = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Example";
        assert_eq!(validate_magnet_url(valid).unwrap(), valid);
        assert!(validate_magnet_url("magnet:?dn=missing-hash").is_err());
        assert!(validate_magnet_url("magnet:?xt=urn:btih:not-a-hash").is_err());
        assert!(validate_magnet_url("file:///tmp/movie.mkv").is_err());
    }

    #[test]
    fn parses_and_validates_torrent_metadata_without_starting_a_transport() {
        assert!(valid_info_hash("0123456789abcdef0123456789ABCDEF01234567"));
        assert!(valid_info_hash("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"));
        assert!(!valid_info_hash("not-a-hash"));
    }

    #[derive(Default)]
    struct MemoryStore(Mutex<HashMap<String, Vec<AddonRecord>>>);

    impl AddonStore for MemoryStore {
        fn list(&self, profile: &str) -> Result<Vec<AddonRecord>, AddonStoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(profile)
                .cloned()
                .unwrap_or_default())
        }
        fn insert(&self, profile: &str, record: &AddonRecord) -> Result<(), AddonStoreError> {
            let mut lock = self.0.lock().unwrap();
            let records = lock.entry(profile.to_owned()).or_default();
            if records.iter().any(|item| item.id == record.id) {
                return Err(AddonStoreError::Unavailable("duplicate ID".to_owned()));
            }
            records.push(record.clone());
            Ok(())
        }
        fn update(&self, profile: &str, record: &AddonRecord) -> Result<(), AddonStoreError> {
            let mut lock = self.0.lock().unwrap();
            let records = lock.get_mut(profile).ok_or(AddonStoreError::NotFound)?;
            let current = records
                .iter_mut()
                .find(|item| item.id == record.id)
                .ok_or(AddonStoreError::NotFound)?;
            *current = record.clone();
            Ok(())
        }
        fn remove(&self, profile: &str, id: &str) -> Result<(), AddonStoreError> {
            let mut lock = self.0.lock().unwrap();
            let records = lock.get_mut(profile).ok_or(AddonStoreError::NotFound)?;
            let before = records.len();
            records.retain(|record| record.id != id);
            if records.len() == before {
                return Err(AddonStoreError::NotFound);
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemorySecrets(Mutex<HashMap<String, String>>);

    impl AddonSecretStore for MemorySecrets {
        fn get(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn install_disable_list_reorder_and_remove_are_profile_scoped_and_keep_urls_secret() {
        let store = Arc::new(MemoryStore::default());
        let secrets = Arc::new(MemorySecrets::default());
        let manager = AddonManager::new(store.clone(), secrets.clone());
        let manifest_url = "https://addons.example/config=private-token/manifest.json?key=private";
        let parsed_url = Url::parse(manifest_url).unwrap();
        let preview = AddonPreview {
            manifest_url: manifest_url.to_owned(),
            display_address: redacted_url(&parsed_url),
            manifest: parse_manifest(&parsed_url, MANIFEST).unwrap(),
            allow_insecure_http: false,
        };
        let first = manager.install_preview("default", &preview).unwrap();
        assert!(first.enabled);
        assert_eq!(manager.list_addons("guest").unwrap().len(), 0);
        assert!(matches!(
            manager.install_preview("default", &preview),
            Err(AddonError::AlreadyInstalled)
        ));

        let second_url = Url::parse("https://second.example/manifest.json").unwrap();
        let second_preview = AddonPreview {
            manifest_url: second_url.to_string(),
            display_address: redacted_url(&second_url),
            manifest: preview.manifest.clone(),
            allow_insecure_http: false,
        };
        let second = manager.install_preview("default", &second_preview).unwrap();

        manager.set_enabled("default", &first.id, false).unwrap();
        assert!(!manager.list_addons("default").unwrap()[0].enabled);
        manager.set_enabled("default", &first.id, true).unwrap();
        manager
            .reorder_addons("default", &[second.id.clone(), first.id.clone()])
            .unwrap();
        let ordered = manager.list_addons("default").unwrap();
        assert_eq!(ordered[0].id, second.id);
        assert_eq!(ordered[1].id, first.id);
        assert!(manager.reorder_addons("default", &[first.id.clone(), first.id.clone()]).is_err());

        let stored = store.list("default").unwrap();
        let first_record = stored.iter().find(|record| record.id == first.id).unwrap();
        assert!(
            !first_record
                .manifest_json
                .as_deref()
                .unwrap()
                .contains("private-token")
        );
        assert!(
            !first_record
                .manifest_json
                .as_deref()
                .unwrap()
                .contains("private")
        );
        let listed = manager.list_addons("default").unwrap();
        let first_listed = listed.iter().find(|addon| addon.id == first.id).unwrap();
        assert!(!first_listed.display_address.contains("private"));
        manager.remove_addon("default", &first.id).unwrap();
        assert_eq!(manager.list_addons("default").unwrap().len(), 1);
        manager.remove_addon("default", &second.id).unwrap();
        assert!(manager.list_addons("default").unwrap().is_empty());
    }

    #[test]
    fn stremio_stream_and_subtitle_objects_decode_expected_protocol_fields() {
        let stream: AddonStream = serde_json::from_value(serde_json::json!({
            "name": "Example provider",
            "title": "1080p",
            "infoHash": "0123456789abcdef0123456789abcdef01234567",
            "fileIdx": 4,
            "behaviorHints": { "notWebReady": true },
            "providerSpecific": "preserved"
        }))
        .unwrap();
        assert_eq!(
            stream.info_hash.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(stream.file_idx, Some(4));
        assert_eq!(
            stream.extra.get("providerSpecific").and_then(Value::as_str),
            Some("preserved")
        );

        let subtitle: AddonSubtitle = serde_json::from_value(serde_json::json!({
            "id": "subtitle-1",
            "url": "https://subtitles.example/en.srt",
            "lang": "eng",
            "label": "English",
            "type": "srt"
        }))
        .unwrap();
        assert_eq!(subtitle.language, "eng");
        assert_eq!(subtitle.mime_type.as_deref(), Some("srt"));
        assert_eq!(subtitle.label.as_deref(), Some("English"));
    }

    #[tokio::test]
    #[ignore = "requires access to the public Stremio Cinemeta service"]
    async fn live_cinemeta_manifest_and_catalog_compatibility() {
        let store = Arc::new(MemoryStore::default());
        let secrets = Arc::new(MemorySecrets::default());
        let manager = AddonManager::new(store, secrets);
        let preview = manager
            .preview_addon("https://v3-cinemeta.strem.io/manifest.json", false)
            .await
            .expect("public Cinemeta manifest must be reachable and valid");
        assert_eq!(preview.manifest.id, "com.linvo.cinemeta");
        assert!(preview.manifest.supports_resource("catalog", "movie", "top"));
        assert!(preview.manifest.supports_resource("meta", "movie", "tt0111161"));

        let installed = manager
            .install_preview("live-acceptance", &preview)
            .expect("the validated manifest must install into the acceptance profile");
        let catalogs = manager
            .catalogs("live-acceptance", &installed.id, "movie")
            .expect("movie catalog declarations must be available");
        assert!(catalogs.iter().any(|catalog| catalog.id == "top"));

        let items = manager
            .fetch_catalog(
                "live-acceptance",
                &installed.id,
                "movie",
                "top",
                &BTreeMap::new(),
            )
            .await
            .expect("Cinemeta top movie catalog must be queryable");
        assert!(!items.is_empty(), "Cinemeta should return movie metadata");

        let metadata = manager
            .fetch_meta("live-acceptance", &installed.id, "movie", "tt0111161")
            .await
            .expect("Cinemeta metadata must be queryable");
        assert_eq!(
            metadata.get("id").and_then(Value::as_str),
            Some("tt0111161")
        );
        assert_eq!(
            metadata.get("type").and_then(Value::as_str),
            Some("movie")
        );
    }

    #[test]
    fn catalog_items_keep_optional_title_and_unrecognized_metadata() {
        let item: AddonCatalogItem = serde_json::from_str(
            r#"{"id":"tt123","type":"movie","name":"Example","customField":"value"}"#,
        )
        .unwrap();
        assert_eq!(item.display_title(), "Example");
        assert_eq!(item.extra.get("customField").unwrap(), "value");
    }
}
