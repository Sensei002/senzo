//! Profile-scoped HTTP download queue with resumable partial files.
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, header};
use senzo_platform::CredentialStore;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    runtime::Handle,
};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

const QUEUE_SCHEMA_VERSION: u32 = 1;
const MAX_RETRIES_WITHOUT_PROGRESS: u32 = 10;
const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(1);
const MAX_SOURCE_HEADERS: usize = 64;
const MAX_SOURCE_HEADER_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadItem {
    pub id: String,
    pub profile_id: String,
    pub media_id: String,
    pub title: String,
    pub file_name: String,
    pub status: DownloadStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub local_path: Option<String>,
    pub error: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl DownloadItem {
    pub fn progress_fraction(&self) -> f32 {
        let Some(total) = self.total_bytes.filter(|value| *value > 0) else {
            return 0.0;
        };
        (self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0) as f32
    }

    pub fn is_playable(&self) -> bool {
        self.status == DownloadStatus::Completed
            && self.local_path.as_deref().is_some_and(|path| Path::new(path).is_file())
    }

    /// Returns a local file URL only after the completed payload has been verified.
    pub fn local_file_url(&self) -> Option<String> {
        if !self.is_playable() {
            return None;
        }
        Url::from_file_path(self.local_path.as_deref()?)
            .ok()
            .map(|url| url.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub profile_id: String,
    pub media_id: String,
    pub title: String,
    pub source_url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("download queue storage failed: {0}")]
    Storage(String),
    #[error("download source must be an absolute HTTP or HTTPS URL without embedded credentials")]
    InvalidSource,
    #[error("download request headers are invalid or exceed configured limits")]
    InvalidHeaders,
    #[error("secure credential storage failed: {0}")]
    Credentials(String),
    #[error("download item was not found")]
    NotFound,
    #[error("download item is not in a state that supports this action")]
    InvalidState,
    #[error("download item failed: {0}")]
    Transfer(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredQueue {
    version: u32,
    items: Vec<DownloadItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DownloadSecret {
    source_url: String,
    headers: BTreeMap<String, String>,
}

struct Inner {
    queue_file: PathBuf,
    download_dir: PathBuf,
    runtime: Handle,
    client: Client,
    credentials: Arc<dyn CredentialStore>,
    state: Mutex<StoredQueue>,
    tasks: Mutex<HashMap<String, CancellationToken>>,
}

#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Inner>,
}

impl DownloadManager {
    /// Opens the durable queue. Any item left in Downloading after a crash is
    /// recovered as Paused; its .part payload remains available for Range resume.
    pub fn open(
        data_dir: impl AsRef<Path>,
        runtime: Handle,
        credentials: Arc<dyn CredentialStore>,
    ) -> Result<Self, DownloadError> {
        fs::create_dir_all(data_dir.as_ref()).map_err(storage_error)?;
        let download_dir = data_dir.as_ref().join("downloads");
        fs::create_dir_all(&download_dir).map_err(storage_error)?;
        let queue_file = data_dir.as_ref().join("downloads.json");
        let mut stored = if queue_file.is_file() {
            let bytes = fs::read(&queue_file).map_err(storage_error)?;
            serde_json::from_slice::<StoredQueue>(&bytes)
                .unwrap_or(StoredQueue { version: QUEUE_SCHEMA_VERSION, items: Vec::new() })
        } else {
            StoredQueue { version: QUEUE_SCHEMA_VERSION, items: Vec::new() }
        };
        stored.version = QUEUE_SCHEMA_VERSION;
        for item in &mut stored.items {
            if item.status == DownloadStatus::Downloading || item.status == DownloadStatus::Queued {
                item.status = DownloadStatus::Paused;
                item.error = Some("Download paused because SENZO was closed or restarted.".into());
            }
            if item.status == DownloadStatus::Completed {
                let exists = item.local_path.as_deref().is_some_and(|path| Path::new(path).is_file());
                if !exists {
                    item.status = DownloadStatus::Failed;
                    item.local_path = None;
                    item.error = Some("The downloaded file is missing. Retry the download.".into());
                }
            }
        }
        let manager = Self {
            inner: Arc::new(Inner {
                queue_file,
                download_dir,
                runtime,
                client: Client::builder()
                    .connect_timeout(Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::limited(10))
                    .build()
                    .map_err(|error| DownloadError::Storage(error.to_string()))?,
                credentials,
                state: Mutex::new(stored),
                tasks: Mutex::new(HashMap::new()),
            }),
        };
        persist_queue(&manager.inner)?;
        Ok(manager)
    }

    pub fn snapshot(&self, profile_id: &str) -> Vec<DownloadItem> {
        self.inner.state.lock().map(|state| {
            let mut items = state.items.iter()
                .filter(|item| item.profile_id == profile_id)
                .cloned()
                .collect::<Vec<_>>();
            items.sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
            items
        }).unwrap_or_default()
    }

    pub fn enqueue(&self, request: DownloadRequest) -> Result<String, DownloadError> {
        let source = Url::parse(request.source_url.trim()).map_err(|_| DownloadError::InvalidSource)?;
        if !matches!(source.scheme(), "http" | "https")
            || source.host_str().is_none()
            || !source.username().is_empty()
            || source.password().is_some()
        {
            return Err(DownloadError::InvalidSource);
        }
        if request.profile_id.trim().is_empty() || request.media_id.trim().is_empty() {
            return Err(DownloadError::InvalidSource);
        }
        let headers = sanitize_headers(request.headers)?;
        let id = Uuid::new_v4().simple().to_string();
        let secret = DownloadSecret { source_url: source.to_string(), headers };
        let secret_json = serde_json::to_string(&secret).map_err(|error| DownloadError::Storage(error.to_string()))?;
        self.inner.credentials.set(&secret_key(&id), &secret_json)
            .map_err(|error| DownloadError::Credentials(error.to_string()))?;

        let now = now_ms();
        let file_name = build_file_name(&request.title, &source, &id);
        let item = DownloadItem {
            id: id.clone(),
            profile_id: request.profile_id.trim().to_owned(),
            media_id: request.media_id.trim().to_owned(),
            title: request.title.trim().to_owned(),
            file_name,
            status: DownloadStatus::Queued,
            downloaded_bytes: 0,
            total_bytes: None,
            local_path: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        {
            let mut state = self.inner.state.lock().map_err(|_| DownloadError::Storage("queue lock is unavailable".into()))?;
            state.items.insert(0, item);
        }
        if let Err(error) = persist_queue(&self.inner) {
            let _ = self.inner.credentials.delete(&secret_key(&id));
            return Err(error);
        }
        self.start_worker(id.clone())?;
        Ok(id)
    }

    pub fn pause(&self, id: &str) -> Result<(), DownloadError> {
        let token = self.inner.tasks.lock()
            .map_err(|_| DownloadError::Storage("download task lock is unavailable".into()))?
            .get(id).cloned();
        let token = token.ok_or(DownloadError::InvalidState)?;
        token.cancel();
        self.update_item(id, |item| {
            item.status = DownloadStatus::Paused;
            item.error = None;
            item.updated_at_ms = now_ms();
        })?;
        Ok(())
    }

    pub fn resume(&self, id: &str) -> Result<(), DownloadError> {
        self.update_item(id, |item| {
            if !matches!(item.status, DownloadStatus::Paused | DownloadStatus::Failed | DownloadStatus::Queued) {
                return;
            }
            item.status = DownloadStatus::Queued;
            item.error = None;
            item.updated_at_ms = now_ms();
        })?;
        let state = self.inner.state.lock().map_err(|_| DownloadError::Storage("queue lock is unavailable".into()))?;
        let item = state.items.iter().find(|item| item.id == id).ok_or(DownloadError::NotFound)?;
        if item.status != DownloadStatus::Queued {
            return Err(DownloadError::InvalidState);
        }
        drop(state);
        self.start_worker(id.to_owned())
    }

    pub fn cancel(&self, id: &str) -> Result<(), DownloadError> {
        if let Some(token) = self.inner.tasks.lock()
            .map_err(|_| DownloadError::Storage("download task lock is unavailable".into()))?
            .remove(id) {
            token.cancel();
        }
        let item = {
            let mut state = self.inner.state.lock().map_err(|_| DownloadError::Storage("queue lock is unavailable".into()))?;
            let index = state.items.iter().position(|item| item.id == id).ok_or(DownloadError::NotFound)?;
            state.items.remove(index)
        };
        let part = self.inner.download_dir.join(format!("{}.part", item.file_name));
        let _ = fs::remove_file(part);
        if item.status == DownloadStatus::Completed {
            if let Some(path) = item.local_path.as_deref() {
                let _ = fs::remove_file(path);
            }
        }
        self.inner.credentials.delete(&secret_key(id))
            .map_err(|error| DownloadError::Credentials(error.to_string()))?;
        persist_queue(&self.inner)
    }

    pub fn remove_completed(&self, id: &str) -> Result<(), DownloadError> {
        let item = {
            let mut state = self.inner.state.lock().map_err(|_| DownloadError::Storage("queue lock is unavailable".into()))?;
            let index = state.items.iter().position(|item| item.id == id).ok_or(DownloadError::NotFound)?;
            if state.items[index].status != DownloadStatus::Completed {
                return Err(DownloadError::InvalidState);
            }
            state.items.remove(index)
        };
        if let Some(path) = item.local_path.as_deref() {
            let _ = fs::remove_file(path);
        }
        self.inner.credentials.delete(&secret_key(id))
            .map_err(|error| DownloadError::Credentials(error.to_string()))?;
        persist_queue(&self.inner)
    }

    pub fn local_file_path(&self, id: &str) -> Option<PathBuf> {
        let state = self.inner.state.lock().ok()?;
        let item = state.items.iter().find(|item| item.id == id)?;
        if item.status != DownloadStatus::Completed {
            return None;
        }
        let path = PathBuf::from(item.local_path.as_deref()?);
        path.is_file().then_some(path)
    }

    /// Stops workers cleanly while retaining .part files and saved source secrets.
    pub fn pause_all(&self) {
        if let Ok(mut tasks) = self.inner.tasks.lock() {
            for token in tasks.values() { token.cancel(); }
            tasks.clear();
        }
        if let Ok(mut state) = self.inner.state.lock() {
            for item in &mut state.items {
                if matches!(item.status, DownloadStatus::Downloading | DownloadStatus::Queued) {
                    item.status = DownloadStatus::Paused;
                    item.error = None;
                    item.updated_at_ms = now_ms();
                }
            }
        }
        let _ = persist_queue(&self.inner);
    }

    fn start_worker(&self, id: String) -> Result<(), DownloadError> {
        let token = CancellationToken::new();
        {
            let mut tasks = self.inner.tasks.lock().map_err(|_| DownloadError::Storage("download task lock is unavailable".into()))?;
            if tasks.contains_key(&id) {
                return Err(DownloadError::InvalidState);
            }
            tasks.insert(id.clone(), token.clone());
        }
        self.update_item(&id, |item| {
            item.status = DownloadStatus::Downloading;
            item.error = None;
            item.updated_at_ms = now_ms();
        })?;
        let inner = self.inner.clone();
        let file_name = inner.state.lock()
            .map_err(|_| DownloadError::Storage("download queue lock is unavailable".into()))?
            .items.iter()
            .find(|item| item.id == id)
            .map(|item| item.file_name.clone())
            .ok_or(DownloadError::NotFound)?;
        self.inner.runtime.spawn(async move {
            run_download(inner.clone(), id.clone(), token).await;
            let item_still_exists = inner.state.lock()
                .map(|state| state.items.iter().any(|item| item.id == id))
                .unwrap_or(true);
            if !item_still_exists {
                let partial = inner.download_dir.join(format!("{file_name}.part"));
                let _ = tokio::fs::remove_file(partial).await;
            }
            if let Ok(mut tasks) = inner.tasks.lock() {
                tasks.remove(&id);
            }
        });
        Ok(())
    }

    fn update_item(&self, id: &str, update: impl FnOnce(&mut DownloadItem)) -> Result<(), DownloadError> {
        {
            let mut state = self.inner.state.lock().map_err(|_| DownloadError::Storage("queue lock is unavailable".into()))?;
            let item = state.items.iter_mut().find(|item| item.id == id).ok_or(DownloadError::NotFound)?;
            update(item);
        }
        persist_queue(&self.inner)
    }
}

fn secret_key(id: &str) -> String {
    format!("senzo.download.{id}")
}

fn storage_error(error: std::io::Error) -> DownloadError {
    DownloadError::Storage(error.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(u64::MAX as u128) as u64
}

fn persist_queue(inner: &Inner) -> Result<(), DownloadError> {
    let snapshot = inner.state.lock()
        .map_err(|_| DownloadError::Storage("queue lock is unavailable".into()))?
        .clone();
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| DownloadError::Storage(error.to_string()))?;
    let temporary = inner.queue_file.with_extension("json.tmp");
    fs::write(&temporary, bytes).map_err(storage_error)?;
    if inner.queue_file.exists() {
        fs::remove_file(&inner.queue_file).map_err(storage_error)?;
    }
    fs::rename(&temporary, &inner.queue_file).map_err(storage_error)
}

fn sanitize_headers(headers: BTreeMap<String, String>) -> Result<BTreeMap<String, String>, DownloadError> {
    if headers.len() > MAX_SOURCE_HEADERS {
        return Err(DownloadError::InvalidHeaders);
    }
    let mut result = BTreeMap::new();
    let mut total_bytes = 0usize;
    for (key, value) in headers {
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty()
            || key.bytes().any(|byte| byte <= 0x20 || byte >= 0x7f || b"()<>@,;:\"/[]?={}\t".contains(&byte))
            || value.chars().any(char::is_control)
            || matches!(key.to_ascii_lowercase().as_str(), "range" | "content-length" | "host" | "accept-encoding")
        {
            return Err(DownloadError::InvalidHeaders);
        }
        total_bytes = total_bytes.saturating_add(key.len()).saturating_add(value.len());
        result.insert(key.to_owned(), value.to_owned());
    }
    if total_bytes > MAX_SOURCE_HEADER_BYTES {
        return Err(DownloadError::InvalidHeaders);
    }
    Ok(result)
}

fn build_file_name(title: &str, source: &Url, id: &str) -> String {
    let safe_title = title.chars().map(|character| {
        if character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            '_'
        } else {
            character
        }
    }).collect::<String>().trim().trim_matches('.').trim().to_owned();
    let safe_title = if safe_title.is_empty() { "download" } else { safe_title.as_str() };
    let extension = source.path().rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    let extension = if (2..=6).contains(&extension.len()) && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        extension
    } else {
        "mp4".to_owned()
    };
    format!("{}_{}.{}", safe_title.chars().take(90).collect::<String>(), id, extension)
}

fn parse_content_range(value: &str) -> Option<(u64, u64)> {
    let value = value.strip_prefix("bytes ")?;
    let (span, total) = value.split_once('/')?;
    let total = total.parse::<u64>().ok()?;
    let (start, _end) = span.split_once('-')?;
    Some((start.parse().ok()?, total))
}

async fn run_download(inner: Arc<Inner>, id: String, cancel: CancellationToken) {
    let secret = match inner.credentials.get(&secret_key(&id)) {
        Ok(Some(value)) => match serde_json::from_str::<DownloadSecret>(&value) {
            Ok(secret) => secret,
            Err(error) => {
                fail_download(&inner, &id, format!("Saved source information is invalid: {error}"));
                return;
            }
        },
        Ok(None) => {
            fail_download(&inner, &id, "Saved source credentials are missing; choose this source again.".into());
            return;
        }
        Err(error) => {
            fail_download(&inner, &id, format!("Could not access saved source credentials: {error}"));
            return;
        }
    };
    let item = match inner.state.lock().ok().and_then(|state| state.items.iter().find(|item| item.id == id).cloned()) {
        Some(item) => item,
        None => return,
    };
    let destination = inner.download_dir.join(&item.file_name);
    let partial = inner.download_dir.join(format!("{}.part", item.file_name));
    let mut no_progress_retries = 0u32;

    loop {
        if cancel.is_cancelled() {
            pause_download(&inner, &id);
            return;
        }
        let baseline = tokio::fs::metadata(&partial).await.map(|metadata| metadata.len()).unwrap_or(0);
        let mut request = inner.client.get(&secret.source_url).header(header::ACCEPT_ENCODING, "identity");
        for (key, value) in &secret.headers {
            request = request.header(key, value);
        }
        if baseline > 0 {
            request = request.header(header::RANGE, format!("bytes={baseline}-"));
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => {
                pause_download(&inner, &id);
                return;
            }
            result = request.send() => match result {
                Ok(response) => response,
                Err(error) => {
                    if !retry_or_fail(&inner, &id, &mut no_progress_retries, baseline, &partial, error.to_string(), &cancel).await {
                        return;
                    }
                    continue;
                }
            }
        };
        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE && baseline > 0 {
            let _ = tokio::fs::remove_file(&partial).await;
            continue;
        }
        if response.status() != StatusCode::OK && response.status() != StatusCode::PARTIAL_CONTENT {
            let status = response.status();
            if status == StatusCode::REQUEST_TIMEOUT || status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                if !retry_or_fail(&inner, &id, &mut no_progress_retries, baseline, &partial, format!("HTTP {status}"), &cancel).await {
                    return;
                }
                continue;
            }
            fail_download(&inner, &id, format!("Download rejected with HTTP {status}"));
            return;
        }

        let append = baseline > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
        if append {
            let valid_range = response.headers().get(header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| parse_content_range(value));
            if !valid_range.is_some_and(|(start, _)| start == baseline) {
                drop(response);
                let _ = tokio::fs::remove_file(&partial).await;
                continue;
            }
        }
        let starting_bytes = if append { baseline } else { 0 };
        let total_bytes = response.headers().get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_content_range)
            .map(|(_, total)| total)
            .or_else(|| response.content_length().map(|length| starting_bytes.saturating_add(length)));

        let mut file = match OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&partial).await {
            Ok(file) => file,
            Err(error) => {
                fail_download(&inner, &id, format!("Could not create download file: {error}"));
                return;
            }
        };
        let mut downloaded = starting_bytes;
        let mut last_persist = Instant::now();
        let mut transfer_error = None;
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = file.flush().await;
                    pause_download(&inner, &id);
                    return;
                }
                chunk = stream.next() => chunk,
            };
            match next {
                Some(Ok(chunk)) => {
                    if let Err(error) = file.write_all(&chunk).await {
                        transfer_error = Some(error.to_string());
                        break;
                    }
                    downloaded = downloaded.saturating_add(chunk.len() as u64);
                    update_progress(&inner, &id, downloaded, total_bytes, &mut last_persist);
                }
                Some(Err(error)) => {
                    transfer_error = Some(error.to_string());
                    break;
                }
                None => break,
            }
        }
        if let Err(error) = file.flush().await {
            transfer_error.get_or_insert_with(|| error.to_string());
        }
        if let Err(error) = file.sync_all().await {
            transfer_error.get_or_insert_with(|| error.to_string());
        }
        drop(file);

        if let Some(error) = transfer_error {
            if !retry_or_fail(&inner, &id, &mut no_progress_retries, baseline, &partial, error, &cancel).await {
                return;
            }
            continue;
        }
        if total_bytes.is_some_and(|total| downloaded < total) {
            if !retry_or_fail(&inner, &id, &mut no_progress_retries, baseline, &partial, format!("Connection ended at {downloaded} bytes before the expected total"), &cancel).await {
                return;
            }
            continue;
        }

        if destination.exists() {
            let _ = tokio::fs::remove_file(&destination).await;
        }
        if let Err(error) = tokio::fs::rename(&partial, &destination).await {
            fail_download(&inner, &id, format!("Could not finalize download: {error}"));
            return;
        }
        set_status(&inner, &id, DownloadStatus::Completed, None, Some(destination.to_string_lossy().into_owned()), downloaded, total_bytes);
        let _ = inner.credentials.delete(&secret_key(&id));
        return;
    }
}

fn update_progress(inner: &Inner, id: &str, downloaded: u64, total: Option<u64>, last_persist: &mut Instant) {
    if let Ok(mut state) = inner.state.lock() {
        if let Some(item) = state.items.iter_mut().find(|item| item.id == id) {
            item.downloaded_bytes = downloaded;
            item.total_bytes = total;
            item.updated_at_ms = now_ms();
        }
    }
    if last_persist.elapsed() >= PROGRESS_PERSIST_INTERVAL {
        let _ = persist_queue(inner);
        *last_persist = Instant::now();
    }
}

fn pause_download(inner: &Inner, id: &str) {
    set_status(inner, id, DownloadStatus::Paused, None, None, 0, None);
}

fn fail_download(inner: &Inner, id: &str, message: String) {
    set_status(inner, id, DownloadStatus::Failed, Some(message), None, 0, None);
}

fn set_status(
    inner: &Inner,
    id: &str,
    status: DownloadStatus,
    error: Option<String>,
    local_path: Option<String>,
    downloaded: u64,
    total: Option<u64>,
) {
    if let Ok(mut state) = inner.state.lock() {
        if let Some(item) = state.items.iter_mut().find(|item| item.id == id) {
            item.status = status;
            item.error = error;
            if status == DownloadStatus::Completed {
                item.local_path = local_path;
            }
            if downloaded > 0 || status == DownloadStatus::Completed {
                item.downloaded_bytes = downloaded;
            }
            if total.is_some() {
                item.total_bytes = total;
            }
            item.updated_at_ms = now_ms();
        }
    }
    let _ = persist_queue(inner);
}

async fn retry_or_fail(
    inner: &Inner,
    id: &str,
    no_progress_retries: &mut u32,
    baseline: u64,
    partial: &Path,
    message: String,
    cancel: &CancellationToken,
) -> bool {
    let current = tokio::fs::metadata(partial).await.map(|metadata| metadata.len()).unwrap_or(0);
    if current > baseline {
        *no_progress_retries = 0;
    }
    *no_progress_retries = no_progress_retries.saturating_add(1);
    if *no_progress_retries > MAX_RETRIES_WITHOUT_PROGRESS {
        fail_download(inner, id, format!("Download failed after repeated connection loss: {message}"));
        return false;
    }
    let seconds = (1u64 << (*no_progress_retries).min(5)).min(30);
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(Duration::from_secs(seconds)) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_safe_unique_download_names_and_normalizes_extensions() {
        let source = Url::parse("https://media.example/video.MKV?token=secret").unwrap();
        let name = build_file_name("../A <movie>: one", &source, "abc123");
        assert!(!name.contains('/') && !name.contains('\\') && !name.contains(':'));
        assert!(name.ends_with("_abc123.mkv"));
        let fallback = build_file_name("   ", &Url::parse("https://media.example/get").unwrap(), "id");
        assert_eq!(fallback, "download_id.mp4");
    }

    #[test]
    fn parses_content_ranges_for_resume_validation() {
        assert_eq!(parse_content_range("bytes 512-1023/4096"), Some((512, 4096)));
        assert_eq!(parse_content_range("bytes 0-10/*"), None);
        assert_eq!(parse_content_range("items 0-10/20"), None);
    }

    #[test]
    fn only_accepts_safe_source_header_names_and_values() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".into(), "Bearer abc".into());
        assert!(sanitize_headers(headers).is_ok());
        let mut unsafe_headers = BTreeMap::new();
        unsafe_headers.insert("Range".into(), "bytes=0-".into());
        assert!(sanitize_headers(unsafe_headers).is_err());
        let mut invalid = BTreeMap::new();
        invalid.insert("X-Test".into(), "bad\nvalue".into());
        assert!(sanitize_headers(invalid).is_err());
    }

    #[derive(Default)]
    struct TestCredentials(Mutex<HashMap<String, String>>);

    impl CredentialStore for TestCredentials {
        fn get(&self, account: &str) -> Result<Option<String>, senzo_platform::CredentialError> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }

        fn set(&self, account: &str, secret: &str) -> Result<(), senzo_platform::CredentialError> {
            self.0.lock().unwrap().insert(account.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), senzo_platform::CredentialError> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }

    #[tokio::test]
    async fn resumes_partial_http_download_and_keeps_source_secrets_out_of_queue() {
        use axum::{
            Router,
            body::Body,
            http::{HeaderMap, Response, StatusCode, header},
            routing::get,
        };
        use tokio::net::TcpListener;

        let payload = (0..131_072).map(|index| (index % 251) as u8).collect::<Vec<_>>();
        let server_payload = payload.clone();
        let app = Router::new().route("/video.mp4", get(move |headers: HeaderMap| {
            let payload = server_payload.clone();
            async move {
                let start = headers.get(header::RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("bytes="))
                    .and_then(|value| value.split('-').next())
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0)
                    .min(payload.len());
                let body = payload[start..].to_vec();
                let status = if start > 0 { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
                let mut response = Response::builder()
                    .status(status)
                    .header(header::ACCEPT_RANGES, "bytes")
                    .header(header::CONTENT_LENGTH, body.len());
                if start > 0 && start < payload.len() {
                    response = response.header(
                        header::CONTENT_RANGE,
                        format!("bytes {}-{}/{}", start, payload.len() - 1, payload.len()),
                    );
                }
                response.body(Body::from(body)).unwrap()
            }
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let directory = tempfile::tempdir().unwrap();
        let source_url = format!("http://{address}/video.mp4");
        let download_id = "range-resume-test";
        let file_name = build_file_name("Range resume", &Url::parse(&source_url).unwrap(), download_id);
        let download_root = directory.path().join("downloads");
        std::fs::create_dir_all(&download_root).unwrap();
        let resume_at = payload.len() / 2;
        std::fs::write(download_root.join(format!("{file_name}.part")), &payload[..resume_at]).unwrap();

        let item = DownloadItem {
            id: download_id.to_owned(),
            profile_id: "default".to_owned(),
            media_id: "603".to_owned(),
            title: "Range resume".to_owned(),
            file_name: file_name.clone(),
            status: DownloadStatus::Downloading,
            downloaded_bytes: resume_at as u64,
            total_bytes: Some(payload.len() as u64),
            local_path: None,
            error: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let stored = StoredQueue { version: QUEUE_SCHEMA_VERSION, items: vec![item] };
        std::fs::write(
            directory.path().join("downloads.json"),
            serde_json::to_vec(&stored).unwrap(),
        ).unwrap();

        let credentials = Arc::new(TestCredentials::default());
        let secret = DownloadSecret {
            source_url: source_url.clone(),
            headers: BTreeMap::from([("Authorization".to_owned(), "Bearer do-not-write-to-disk".to_owned())]),
        };
        credentials.set(&secret_key(download_id), &serde_json::to_string(&secret).unwrap()).unwrap();

        let manager = DownloadManager::open(
            directory.path(),
            tokio::runtime::Handle::current(),
            credentials,
        ).unwrap();
        let recovered = manager.snapshot("default");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].status, DownloadStatus::Paused);
        assert!(manager.snapshot("guest").is_empty());

        manager.resume(download_id).unwrap();
        let completed = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let item = manager.snapshot("default").into_iter().find(|item| item.id == download_id).unwrap();
                if item.status == DownloadStatus::Completed {
                    break item;
                }
                if item.status == DownloadStatus::Failed {
                    panic!("resumed transfer failed: {:?}", item.error);
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }).await.expect("download did not complete within the test deadline");

        let local_path = manager.local_file_path(download_id).expect("completed payload should be playable");
        assert_eq!(std::fs::read(&local_path).unwrap(), payload);
        assert_eq!(completed.downloaded_bytes, payload.len() as u64);
        assert!(completed.local_file_url().is_some_and(|url| url.starts_with("file:")));

        let queue_json = std::fs::read_to_string(directory.path().join("downloads.json")).unwrap();
        assert!(!queue_json.contains(&source_url));
        assert!(!queue_json.contains("do-not-write-to-disk"));
        server.abort();
    }
}
