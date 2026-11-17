//! Safe TMDB artwork URL resolution, bounded downloading, decoding and caching.
//!
//! The crate deliberately accepts artwork only from TMDB's image CDN. It does
//! not follow redirects, sends no credentials, caps both compressed and
//! decoded image sizes, and keeps all disk cache files below its own directory.

use std::{
    collections::HashMap,
    fs::{self, File, FileTimes, OpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use reqwest::{Response, StatusCode, header};
use senzo_network::{HttpClient, NetworkError, ValidatedHttpUrl};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{sync::Semaphore, time::sleep};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

const TMDB_IMAGE_BASE: &str = "https://image.tmdb.org/t/p/";
const DEFAULT_MAX_DOWNLOAD_BYTES: usize = 12 * 1024 * 1024;
const DEFAULT_MAX_IMAGE_DIMENSION: u32 = 12_000;
const DEFAULT_MAX_IMAGE_PIXELS: u64 = 24_000_000;
const MAX_DECODE_ALLOCATION: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkSize {
    W92,
    W154,
    W185,
    W342,
    W500,
    W780,
    W1280,
    Original,
}

impl ArtworkSize {
    const fn as_str(self) -> &'static str {
        match self {
            Self::W92 => "w92",
            Self::W154 => "w154",
            Self::W185 => "w185",
            Self::W342 => "w342",
            Self::W500 => "w500",
            Self::W780 => "w780",
            Self::W1280 => "w1280",
            Self::Original => "original",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "w92" => Self::W92,
            "w154" => Self::W154,
            "w185" => Self::W185,
            "w342" => Self::W342,
            "w500" => Self::W500,
            "w780" => Self::W780,
            "w1280" => Self::W1280,
            "original" => Self::Original,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkUrl(ValidatedHttpUrl);

impl ArtworkUrl {
    /// Parses an already-resolved TMDB image URL, rejecting non-TMDB hosts,
    /// credentials, query strings, fragments and unsupported image sizes.
    pub fn parse(value: &str) -> Result<Self, ArtworkError> {
        let url = Url::parse(value).map_err(|_| ArtworkError::UnsupportedUrl)?;
        if url.scheme() != "https"
            || url.host_str() != Some("image.tmdb.org")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ArtworkError::UnsupportedUrl);
        }

        let segments: Vec<_> = url.path_segments().ok_or(ArtworkError::UnsupportedUrl)?.collect();
        if segments.len() < 4
            || segments[0] != "t"
            || segments[1] != "p"
            || ArtworkSize::parse(segments[2]).is_none()
            || segments[3..].iter().any(|segment| !safe_path_segment(segment))
        {
            return Err(ArtworkError::UnsupportedUrl);
        }

        let validated =
            ValidatedHttpUrl::parse(url.as_str()).map_err(|_| ArtworkError::UnsupportedUrl)?;
        Ok(Self(validated))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_url().as_str()
    }
}

/// Resolves TMDB file paths without accepting arbitrary URLs or path traversal.
#[derive(Debug, Default, Clone, Copy)]
pub struct ArtworkUrlResolver;

impl ArtworkUrlResolver {
    pub fn resolve_tmdb_path(
        &self,
        file_path: &str,
        size: ArtworkSize,
    ) -> Result<ArtworkUrl, ArtworkError> {
        let path = file_path
            .strip_prefix('/')
            .ok_or(ArtworkError::InvalidPath)?;
        let segments: Vec<_> = path.split('/').collect();
        if segments.is_empty() || segments.iter().any(|segment| !safe_path_segment(segment)) {
            return Err(ArtworkError::InvalidPath);
        }

        let mut url = Url::parse(TMDB_IMAGE_BASE).map_err(|_| ArtworkError::UnsupportedUrl)?;
        {
            let mut path_segments = url
                .path_segments_mut()
                .map_err(|_| ArtworkError::UnsupportedUrl)?;
            path_segments.pop_if_empty();
            path_segments.push(size.as_str());
            for segment in segments {
                path_segments.push(segment);
            }
        }
        ArtworkUrl::parse(url.as_str())
    }
}

fn safe_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkFormat {
    Jpeg,
    Png,
    WebP,
}

impl ArtworkFormat {
    fn from_image_format(format: ImageFormat) -> Option<Self> {
        match format {
            ImageFormat::Jpeg => Some(Self::Jpeg),
            ImageFormat::Png => Some(Self::Png),
            ImageFormat::WebP => Some(Self::WebP),
            _ => None,
        }
    }

    fn matches_content_type(self, content_type: &str) -> bool {
        match self {
            Self::Jpeg => matches!(content_type, "image/jpeg" | "image/jpg" | "image/pjpeg"),
            Self::Png => matches!(content_type, "image/png" | "image/x-png"),
            Self::WebP => content_type == "image/webp",
        }
    }

    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::WebP => "image/webp",
        }
    }
}

/// An image whose compressed bytes and decoded pixel dimensions have both
/// passed the artwork size and format checks.
#[derive(Debug, Clone)]
pub struct DecodedArtwork {
    width: u32,
    height: u32,
    format: ArtworkFormat,
    encoded: Arc<[u8]>,
    rgba8: Arc<[u8]>,
}

impl DecodedArtwork {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn format(&self) -> ArtworkFormat {
        self.format
    }

    /// Decoded, tightly packed RGBA8 pixels for the native UI image adapter.
    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }

    /// Original validated image bytes, useful for native decoders and disk I/O.
    pub fn encoded_bytes(&self) -> &[u8] {
        &self.encoded
    }

    fn memory_size(&self) -> usize {
        self.rgba8.len().saturating_add(self.encoded.len())
    }
}

#[derive(Debug, Clone)]
pub struct ArtworkConfig {
    pub max_download_bytes: usize,
    pub max_image_dimension: u32,
    pub max_image_pixels: u64,
    pub memory_cache_bytes: usize,
    pub disk_cache_bytes: u64,
    pub max_concurrent_downloads: usize,
    pub max_retries: usize,
}

impl Default for ArtworkConfig {
    fn default() -> Self {
        Self {
            max_download_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
            max_image_dimension: DEFAULT_MAX_IMAGE_DIMENSION,
            max_image_pixels: DEFAULT_MAX_IMAGE_PIXELS,
            memory_cache_bytes: 64 * 1024 * 1024,
            disk_cache_bytes: 512 * 1024 * 1024,
            max_concurrent_downloads: 6,
            max_retries: 2,
        }
    }
}

impl ArtworkConfig {
    fn validate(&self) -> Result<(), ArtworkError> {
        if self.max_download_bytes == 0
            || self.max_image_dimension == 0
            || self.max_image_pixels == 0
            || self.max_concurrent_downloads == 0
        {
            return Err(ArtworkError::InvalidConfig);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ArtworkError {
    #[error("TMDB artwork path is invalid")]
    InvalidPath,
    #[error("artwork URL is not an allowed HTTPS TMDB image URL")]
    UnsupportedUrl,
    #[error("artwork configuration contains a zero limit")]
    InvalidConfig,
    #[error("artwork request was cancelled")]
    Cancelled,
    #[error("artwork server returned HTTP {0}")]
    HttpStatus(u16),
    #[error("artwork response has a missing or unsupported content type")]
    InvalidContentType,
    #[error("artwork response exceeded the configured download limit")]
    ResponseTooLarge,
    #[error("artwork image is invalid or uses an unsupported format: {0}")]
    InvalidImage(String),
    #[error("artwork image dimensions exceed the configured limit")]
    ImageTooLarge,
    #[error("artwork network request failed: {0}")]
    Network(String),
    #[error("artwork cache lock was poisoned")]
    CachePoisoned,
    #[error("artwork cache directory could not be created: {0}")]
    CacheIo(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArtworkCacheStats {
    pub memory_hits: u64,
    pub disk_hits: u64,
    pub misses: u64,
}

#[derive(Default)]
struct CacheCounters {
    memory_hits: AtomicU64,
    disk_hits: AtomicU64,
    misses: AtomicU64,
}

struct MemoryEntry {
    image: Arc<DecodedArtwork>,
    touched: u64,
    weight: usize,
}

#[derive(Default)]
struct MemoryCache {
    entries: HashMap<String, MemoryEntry>,
    bytes: usize,
    clock: u64,
}

impl MemoryCache {
    fn get(&mut self, key: &str) -> Option<Arc<DecodedArtwork>> {
        self.clock = self.clock.wrapping_add(1);
        let touched = self.clock;
        self.entries.get_mut(key).map(|entry| {
            entry.touched = touched;
            Arc::clone(&entry.image)
        })
    }

    fn insert(&mut self, key: String, image: Arc<DecodedArtwork>, capacity: usize) {
        let weight = image.memory_size();
        if capacity == 0 || weight > capacity {
            return;
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.weight);
        }

        self.clock = self.clock.wrapping_add(1);
        self.bytes = self.bytes.saturating_add(weight);
        self.entries.insert(
            key,
            MemoryEntry {
                image,
                touched: self.clock,
                weight,
            },
        );

        while self.bytes > capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed.weight);
            }
        }
    }
}

struct ArtworkCache {
    directory: PathBuf,
    memory: Mutex<MemoryCache>,
    counters: CacheCounters,
    max_memory_bytes: usize,
    max_disk_bytes: u64,
    max_download_bytes: usize,
    max_image_dimension: u32,
    max_image_pixels: u64,
}

impl ArtworkCache {
    fn new(directory: PathBuf, config: &ArtworkConfig) -> Result<Self, ArtworkError> {
        fs::create_dir_all(&directory)?;
        Ok(Self {
            directory,
            memory: Mutex::new(MemoryCache::default()),
            counters: CacheCounters::default(),
            max_memory_bytes: config.memory_cache_bytes,
            max_disk_bytes: config.disk_cache_bytes,
            max_download_bytes: config.max_download_bytes,
            max_image_dimension: config.max_image_dimension,
            max_image_pixels: config.max_image_pixels,
        })
    }

    fn stats(&self) -> ArtworkCacheStats {
        ArtworkCacheStats {
            memory_hits: self.counters.memory_hits.load(Ordering::Relaxed),
            disk_hits: self.counters.disk_hits.load(Ordering::Relaxed),
            misses: self.counters.misses.load(Ordering::Relaxed),
        }
    }

    fn get(&self, key: &str) -> Result<Option<Arc<DecodedArtwork>>, ArtworkError> {
        if let Some(image) = self
            .memory
            .lock()
            .map_err(|_| ArtworkError::CachePoisoned)?
            .get(key)
        {
            self.counters.memory_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Some(image));
        }

        let path = self.file_path(key);
        let bytes = match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file()
                    && metadata.len() <= self.max_download_bytes as u64 =>
            {
                match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        self.counters.misses.fetch_add(1, Ordering::Relaxed);
                        return Ok(None);
                    }
                }
            }
            Ok(_) => {
                let _ = fs::remove_file(&path);
                self.counters.misses.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
            Err(_) => {
                self.counters.misses.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
        };

        let image = match decode_image(
            &bytes,
            None,
            self.max_image_dimension,
            self.max_image_pixels,
        ) {
            Ok(image) => Arc::new(image),
            Err(_) => {
                let _ = fs::remove_file(&path);
                self.counters.misses.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
        };

        self.memory
            .lock()
            .map_err(|_| ArtworkError::CachePoisoned)?
            .insert(key.to_owned(), Arc::clone(&image), self.max_memory_bytes);
        if let Ok(file) = File::open(&path) {
            let _ = file.set_times(FileTimes::new().set_modified(SystemTime::now()));
        }
        self.counters.disk_hits.fetch_add(1, Ordering::Relaxed);
        Ok(Some(image))
    }

    fn put(&self, key: &str, image: Arc<DecodedArtwork>) -> Result<(), ArtworkError> {
        self.memory
            .lock()
            .map_err(|_| ArtworkError::CachePoisoned)?
            .insert(key.to_owned(), Arc::clone(&image), self.max_memory_bytes);

        if self.max_disk_bytes == 0
            || image.encoded.len() as u64 > self.max_disk_bytes
        {
            return Ok(());
        }

        let path = self.file_path(key);
        if path.exists() {
            return Ok(());
        }

        let temporary = self
            .directory
            .join(format!(".{}.{}.tmp", cache_key(key), Uuid::new_v4()));
        let write_result = (|| -> Result<(), std::io::Error> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&image.encoded)?;
            file.flush()?;
            fs::rename(&temporary, &path)?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        self.evict_disk_cache();
        Ok(())
    }

    fn file_path(&self, key: &str) -> PathBuf {
        self.directory.join(format!("{}.art", cache_key(key)))
    }

    fn evict_disk_cache(&self) {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        let mut files = Vec::new();
        let mut total = 0_u64;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("art") {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.file_type().is_file() {
                continue;
            }
            total = total.saturating_add(metadata.len());
            files.push((path, metadata.len(), metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
        }
        files.sort_by_key(|(_, _, modified)| *modified);
        for (path, size, _) in files {
            if total <= self.max_disk_bytes {
                break;
            }
            if fs::remove_file(path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }
}

/// Async artwork loader with bounded concurrency, retries and two cache tiers.
#[derive(Clone)]
pub struct ArtworkService {
    http: HttpClient,
    cache: Arc<ArtworkCache>,
    permits: Arc<Semaphore>,
    config: ArtworkConfig,
}

impl ArtworkService {
    pub fn new(
        http: HttpClient,
        cache_directory: impl AsRef<Path>,
        config: ArtworkConfig,
    ) -> Result<Self, ArtworkError> {
        config.validate()?;
        let cache = ArtworkCache::new(cache_directory.as_ref().to_path_buf(), &config)?;
        Ok(Self {
            http,
            cache: Arc::new(cache),
            permits: Arc::new(Semaphore::new(config.max_concurrent_downloads)),
            config,
        })
    }

    pub fn cache_stats(&self) -> ArtworkCacheStats {
        self.cache.stats()
    }

    pub async fn load(
        &self,
        artwork_url: &ArtworkUrl,
        cancellation: &CancellationToken,
    ) -> Result<Arc<DecodedArtwork>, ArtworkError> {
        if cancellation.is_cancelled() {
            return Err(ArtworkError::Cancelled);
        }

        let key = artwork_url.as_str().to_owned();
        if let Some(image) = self.cache.get(&key)? {
            return Ok(image);
        }

        let _permit = tokio::select! {
            _ = cancellation.cancelled() => return Err(ArtworkError::Cancelled),
            permit = self.permits.acquire() => permit.map_err(|_| ArtworkError::Cancelled)?,
        };

        if let Some(image) = self.cache.get(&key)? {
            return Ok(image);
        }

        let image = self.download(artwork_url, cancellation).await?;
        self.cache.put(&key, Arc::clone(&image))?;
        Ok(image)
    }

    async fn download(
        &self,
        artwork_url: &ArtworkUrl,
        cancellation: &CancellationToken,
    ) -> Result<Arc<DecodedArtwork>, ArtworkError> {
        for attempt in 0..=self.config.max_retries.min(3) {
            let response = tokio::select! {
                _ = cancellation.cancelled() => return Err(ArtworkError::Cancelled),
                response = self.http.get_without_redirects(&artwork_url.0) => response,
            };

            let mut response = match response {
                Ok(response) => response,
                Err(NetworkError::Transport(_)) if attempt < self.config.max_retries.min(3) => {
                    self.wait_for(retry_delay(attempt), cancellation).await?;
                    continue;
                }
                Err(error) => return Err(ArtworkError::Network(error.to_string())),
            };

            if response.url().host_str() != Some("image.tmdb.org")
                && !response.url().host_str().is_some_and(is_loopback_host)
            {
                return Err(ArtworkError::UnsupportedUrl);
            }

            let status = response.status();
            if !status.is_success() {
                if is_retryable(status) && attempt < self.config.max_retries.min(3) {
                    let retry_after = response
                        .headers()
                        .get(header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|seconds| *seconds <= 3);
                    self.wait_for(
                        retry_after.map(Duration::from_secs).unwrap_or_else(|| retry_delay(attempt)),
                        cancellation,
                    )
                    .await?;
                    continue;
                }
                return Err(ArtworkError::HttpStatus(status.as_u16()));
            }

            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase())
                .ok_or(ArtworkError::InvalidContentType)?;

            if response
                .content_length()
                .is_some_and(|length| length > self.config.max_download_bytes as u64)
            {
                return Err(ArtworkError::ResponseTooLarge);
            }

            let bytes = read_limited(
                &mut response,
                self.config.max_download_bytes,
                cancellation,
            )
            .await?;
            let image = decode_image(
                &bytes,
                Some(&content_type),
                self.config.max_image_dimension,
                self.config.max_image_pixels,
            )?;
            return Ok(Arc::new(image));
        }

        Err(ArtworkError::Network(
            "artwork request exhausted its retry budget".to_owned(),
        ))
    }

    async fn wait_for(
        &self,
        delay: Duration,
        cancellation: &CancellationToken,
    ) -> Result<(), ArtworkError> {
        tokio::select! {
            _ = cancellation.cancelled() => Err(ArtworkError::Cancelled),
            _ = sleep(delay) => Ok(()),
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    host == "127.0.0.1" || host == "[::1]" || host.eq_ignore_ascii_case("localhost")
}

fn is_retryable(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(250_u64.saturating_mul(1_u64 << attempt.min(3)))
}

async fn read_limited(
    response: &mut Response,
    max_bytes: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ArtworkError> {
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(ArtworkError::Cancelled),
            chunk = response.chunk() => chunk,
        }
        .map_err(|error| ArtworkError::Network(error.without_url().to_string()))?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ArtworkError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn decode_image(
    bytes: &[u8],
    content_type: Option<&str>,
    max_dimension: u32,
    max_pixels: u64,
) -> Result<DecodedArtwork, ArtworkError> {
    let guessed = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| ArtworkError::InvalidImage(error.to_string()))?;
    let image_format = guessed
        .format()
        .ok_or_else(|| ArtworkError::InvalidImage("unknown image signature".to_owned()))?;
    let format = ArtworkFormat::from_image_format(image_format).ok_or_else(|| {
        ArtworkError::InvalidImage("only JPEG, PNG and WebP images are supported".to_owned())
    })?;

    if content_type.is_some_and(|value| !format.matches_content_type(value)) {
        return Err(ArtworkError::InvalidContentType);
    }

    let (width, height) = guessed
        .into_dimensions()
        .map_err(|error| ArtworkError::InvalidImage(error.to_string()))?;
    if width == 0
        || height == 0
        || width > max_dimension
        || height > max_dimension
        || u64::from(width).saturating_mul(u64::from(height)) > max_pixels
    {
        return Err(ArtworkError::ImageTooLarge);
    }

    let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(max_dimension);
    limits.max_image_height = Some(max_dimension);
    limits.max_alloc = Some(MAX_DECODE_ALLOCATION);
    reader.limits(limits);
    let decoded: DynamicImage = reader
        .decode()
        .map_err(|error| ArtworkError::InvalidImage(error.to_string()))?;
    let rgba8 = decoded.to_rgba8().into_raw();

    Ok(DecodedArtwork {
        width,
        height,
        format,
        encoded: Arc::from(bytes),
        rgba8: Arc::from(rgba8),
    })
}

fn cache_key(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn png_image() -> Vec<u8> {
        let pixels = [
            255_u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&pixels, 2, 2, ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    fn temp_directory() -> PathBuf {
        std::env::temp_dir().join(format!("senzo-artwork-test-{}", Uuid::new_v4()))
    }

    async fn serve_once(status: &str, content_type: &str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = socket.read(&mut request).await;
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        });
        format!("http://{address}/artwork")
    }

    fn test_url(value: &str) -> ArtworkUrl {
        ArtworkUrl(
            ValidatedHttpUrl::parse(value)
                .expect("loopback URL should be valid in the artwork transport test"),
        )
    }

    fn service(directory: &Path, config: ArtworkConfig) -> ArtworkService {
        ArtworkService::new(
            HttpClient::new(Default::default()).unwrap(),
            directory,
            config,
        )
        .unwrap()
    }

    #[test]
    fn resolves_tmdb_paths_and_size_variants() {
        let url = ArtworkUrlResolver
            .resolve_tmdb_path("/abc123/poster.jpg", ArtworkSize::W500)
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://image.tmdb.org/t/p/w500/abc123/poster.jpg"
        );
        let wide = ArtworkUrlResolver
            .resolve_tmdb_path("/backdrop.jpg", ArtworkSize::W1280)
            .unwrap();
        assert!(wide.as_str().contains("/w1280/"));
    }

    #[test]
    fn rejects_path_traversal_arbitrary_hosts_and_unsupported_sizes() {
        assert!(ArtworkUrlResolver
            .resolve_tmdb_path("/../secret.jpg", ArtworkSize::W500)
            .is_err());
        assert!(ArtworkUrlResolver
            .resolve_tmdb_path("//evil.test/file.jpg", ArtworkSize::W500)
            .is_err());
        assert!(ArtworkUrl::parse("https://evil.test/t/p/w500/image.jpg").is_err());
        assert!(ArtworkUrl::parse("https://image.tmdb.org/t/p/bad/image.jpg").is_err());
        assert!(ArtworkUrl::parse("https://image.tmdb.org/t/p/w500/image.jpg?x=1").is_err());
    }

    #[tokio::test]
    async fn downloads_validates_decodes_and_memory_caches_image() {
        let body = png_image();
        let url = serve_once("200 OK", "image/png", body).await;
        let directory = temp_directory();
        let artwork_service = service(&directory, ArtworkConfig::default());
        let cancellation = CancellationToken::new();

        let image = artwork_service
            .load(&test_url(&url), &cancellation)
            .await
            .unwrap();
        assert_eq!((image.width(), image.height()), (2, 2));
        assert_eq!(image.format(), ArtworkFormat::Png);
        assert_eq!(image.rgba8().len(), 16);

        let cached = artwork_service
            .load(&test_url(&url), &cancellation)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&image, &cached));
        assert_eq!(artwork_service.cache_stats().memory_hits, 1);
        let _ = fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn disk_cache_is_reused_by_a_new_service_instance() {
        let url = serve_once("200 OK", "image/png", png_image()).await;
        let directory = temp_directory();
        let cancellation = CancellationToken::new();
        let first = service(&directory, ArtworkConfig::default());
        first.load(&test_url(&url), &cancellation).await.unwrap();

        let second = service(&directory, ArtworkConfig::default());
        let cached = second.load(&test_url(&url), &cancellation).await.unwrap();
        assert_eq!((cached.width(), cached.height()), (2, 2));
        assert_eq!(second.cache_stats().disk_hits, 1);
        let _ = fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn rejects_mime_signature_mismatch_and_redirect_responses() {
        let mismatch = serve_once("200 OK", "image/jpeg", png_image()).await;
        let directory = temp_directory();
        let artwork_service = service(&directory, ArtworkConfig::default());
        let cancellation = CancellationToken::new();
        assert!(matches!(
            artwork_service.load(&test_url(&mismatch), &cancellation).await,
            Err(ArtworkError::InvalidContentType)
        ));

        let redirect = serve_once("302 Found", "text/plain", Vec::new()).await;
        assert!(matches!(
            artwork_service.load(&test_url(&redirect), &cancellation).await,
            Err(ArtworkError::HttpStatus(302))
        ));
        let _ = fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn enforces_download_and_dimension_limits_and_honors_cancellation() {
        let url = serve_once("200 OK", "image/png", png_image()).await;
        let directory = temp_directory();
        let mut config = ArtworkConfig::default();
        config.max_download_bytes = 8;
        let artwork_service = service(&directory, config);
        let cancellation = CancellationToken::new();
        assert!(matches!(
            artwork_service.load(&test_url(&url), &cancellation).await,
            Err(ArtworkError::ResponseTooLarge)
        ));

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(
            artwork_service.load(&test_url(&url), &cancelled).await,
            Err(ArtworkError::Cancelled)
        ));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn bounded_memory_cache_evicts_least_recently_used_entries() {
        let mut cache = MemoryCache::default();
        let image = Arc::new(decode_image(&png_image(), None, 100, 100).unwrap());
        let capacity = image.memory_size() * 2;
        cache.insert("first".to_owned(), Arc::clone(&image), capacity);
        cache.insert("second".to_owned(), Arc::clone(&image), capacity);
        assert!(cache.get("first").is_some());
        cache.insert("third".to_owned(), image, capacity);
        assert!(!cache.entries.contains_key("second"));
        assert!(cache.entries.contains_key("first"));
        assert!(cache.entries.contains_key("third"));
        assert!(cache.bytes <= capacity);
    }
}
