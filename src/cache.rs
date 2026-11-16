//! Bounded on-disk metadata and artwork cache.
use std::{fs, path::{Path, PathBuf}, time::{Duration, SystemTime, UNIX_EPOCH}};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use reqwest::{Client, redirect::Policy};
use url::Url;

const MAX_METADATA_ENTRIES: usize = 500;
const MAX_ARTWORK_BYTES: usize = 8 * 1024 * 1024;
const MAX_ARTWORK_CACHE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct MetadataEnvelope { saved_at: u64, payload: Value }

fn cache_root() -> Result<PathBuf> {
    let base = dirs::data_local_dir().context("local app-data directory is unavailable")?;
    let path = base.join("SENZO").join("cache");
    fs::create_dir_all(&path).with_context(|| format!("could not create {}", path.display()))?;
    Ok(path)
}
fn key_hash(value: &str) -> u64 {
    value.as_bytes().iter().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3))
}
fn metadata_path(key: &str) -> Result<PathBuf> { Ok(cache_root()?.join("metadata").join(format!("{:016x}.json", key_hash(key)))) }
fn artwork_dir() -> Result<PathBuf> {
    let path = cache_root()?.join("artwork");
    fs::create_dir_all(&path).with_context(|| format!("could not create {}", path.display()))?;
    Ok(path)
}
fn unix_now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }

pub fn load_metadata(key: &str, ttl: Duration) -> Result<Option<Value>> {
    let path = metadata_path(key)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("could not read cache {}", path.display())),
    };
    let envelope: MetadataEnvelope = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => { let _ = fs::remove_file(&path); return Ok(None); }
    };
    if unix_now().saturating_sub(envelope.saved_at) > ttl.as_secs() { let _ = fs::remove_file(&path); return Ok(None); }
    let _ = fs::File::open(&path).and_then(|file| file.set_modified(SystemTime::now()));
    Ok(Some(envelope.payload))
}
pub fn store_metadata(key: &str, payload: &Value) -> Result<()> {
    let path = metadata_path(key)?;
    let parent = path.parent().context("metadata cache path has no parent")?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(&MetadataEnvelope { saved_at: unix_now(), payload: payload.clone() })
        .context("could not encode metadata cache entry")?;
    fs::write(&path, bytes).with_context(|| format!("could not write cache {}", path.display()))?;
    prune_metadata(parent)
}
fn prune_metadata(directory: &Path) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.filter_map(std::result::Result::ok).filter_map(|entry| {
        let metadata = entry.metadata().ok()?;
        if !metadata.is_file() { return None; }
        Some((entry.path(), metadata.modified().unwrap_or(UNIX_EPOCH)))
    }).collect::<Vec<_>>();
    entries.sort_by_key(|(_, modified)| *modified);
    let remove_count = entries.len().saturating_sub(MAX_METADATA_ENTRIES);
    for (path, _) in entries.into_iter().take(remove_count) { let _ = fs::remove_file(path); }
    Ok(())
}

fn validate_artwork_url(input: &str) -> Result<Url> {
    let url = Url::parse(input).context("artwork URL is invalid")?;
    if url.scheme() != "https" || url.host_str() != Some("image.tmdb.org") || url.port().is_some()
        || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        bail!("artwork cache accepts only HTTPS URLs from image.tmdb.org");
    }
    Ok(url)
}
fn artwork_path(parsed: &Url) -> Result<PathBuf> {
    let directory = artwork_dir()?;
    let extension = Path::new(parsed.path()).extension().and_then(|v| v.to_str())
        .filter(|v| matches!(v.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp")).unwrap_or("img");
    Ok(directory.join(format!("{:016x}.{extension}", key_hash(parsed.as_str()))))
}
pub fn cached_artwork_path(url: &str) -> Result<Option<PathBuf>> {
    let parsed = validate_artwork_url(url)?;
    let path = artwork_path(&parsed)?;
    if path.is_file() {
        let _ = fs::File::open(&path).and_then(|file| file.set_modified(SystemTime::now()));
        Ok(Some(path))
    } else { Ok(None) }
}
pub fn load_artwork_bytes(url: &str) -> Result<Option<Vec<u8>>> {
    let Some(path) = cached_artwork_path(url)? else { return Ok(None); };
    let bytes = fs::read(path).context("could not read cached artwork")?;
    if bytes.is_empty() || bytes.len() > MAX_ARTWORK_BYTES { return Ok(None); }
    Ok(Some(bytes))
}
pub fn store_artwork_bytes(url: &str, bytes: &[u8]) -> Result<()> {
    validate_artwork_url(url)?;
    if bytes.is_empty() || bytes.len() > MAX_ARTWORK_BYTES { bail!("artwork is empty or exceeds the 8 MiB cache limit"); }
    let path = artwork_path(&validate_artwork_url(url)?)?;
    fs::write(&path, bytes).with_context(|| format!("could not persist artwork {}", path.display()))?;
    prune_artwork(path.parent().context("artwork cache path has no parent")?)
}
pub async fn cache_artwork(url: &str) -> Result<PathBuf> {
    let parsed = validate_artwork_url(url)?;
    if let Some(path) = cached_artwork_path(parsed.as_str())? { return Ok(path); }
    let client = Client::builder().https_only(true).redirect(Policy::none()).timeout(Duration::from_secs(15))
        .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION"))).build().context("could not initialize artwork cache client")?;
    let response = client.get(parsed.clone()).send().await.context("artwork cache request failed")?.error_for_status()
        .context("artwork cache request was rejected")?;
    if response.content_length().is_some_and(|length| length > MAX_ARTWORK_BYTES as u64) { bail!("artwork exceeds the 8 MiB cache limit"); }
    let bytes = response.bytes().await.context("could not read artwork response")?;
    store_artwork_bytes(parsed.as_str(), &bytes)?;
    artwork_path(&parsed)
}
fn prune_artwork(directory: &Path) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.filter_map(std::result::Result::ok).filter_map(|entry| {
        let metadata = entry.metadata().ok()?;
        if !metadata.is_file() { return None; }
        Some((entry.path(), metadata.len(), metadata.modified().unwrap_or(UNIX_EPOCH)))
    }).collect::<Vec<_>>();
    entries.sort_by_key(|(_, _, modified)| *modified);
    let mut total = entries.iter().map(|(_, size, _)| *size).sum::<u64>();
    for (path, size, _) in entries { if total <= MAX_ARTWORK_CACHE_BYTES { break; } if fs::remove_file(path).is_ok() { total = total.saturating_sub(size); } }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_cache_rejects_untrusted_hosts_and_protocols() {
        assert!(validate_artwork_url("https://image.tmdb.org/t/p/w500/a.jpg").is_ok());
        assert!(validate_artwork_url("http://image.tmdb.org/a.jpg").is_err());
        assert!(validate_artwork_url("https://example.org/a.jpg").is_err());
        assert!(validate_artwork_url("https://image.tmdb.org.evil.test/a.jpg").is_err());
    }
    #[test]
    fn cache_keys_are_stable_and_distinct() {
        assert_eq!(key_hash("tv:42:en-US"), key_hash("tv:42:en-US"));
        assert_ne!(key_hash("tv:42:en-US"), key_hash("tv:42:bn-BD"));
    }
}
