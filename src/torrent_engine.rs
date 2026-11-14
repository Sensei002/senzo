//! Embedded torrent engine and loopback-only HTTP range streaming.
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use librqbit::{
    AddTorrent, AddTorrentOptions, Session, SessionOptions,
    api::{Api, ApiAddTorrentResponse, TorrentIdOrHash},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
    net::TcpListener,
    runtime::{Handle, Runtime},
    task::JoinHandle,
};
use tokio_util::{io::ReaderStream, sync::CancellationToken};
use uuid::Uuid;

use crate::torrent::TorrentSessionDirectory;

struct TorrentRuntime {
    runtime: Option<Runtime>,
}

impl TorrentRuntime {
    fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .context("could not create the torrent session runtime")?;
        Ok(Self {
            runtime: Some(runtime),
        })
    }

    fn handle(&self) -> &Handle {
        self.runtime
            .as_ref()
            .expect("torrent runtime remains available until shutdown")
            .handle()
    }

    fn shutdown(&mut self) {
        self.runtime.take();
    }
}

impl Drop for TorrentRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentVideoFile {
    pub index: usize,
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub uploaded_bytes: u64,
    pub finished: bool,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Clone)]
struct StreamServerState {
    api: Api,
    torrent_id: usize,
    token: String,
}

pub struct TorrentPlaybackSession {
    session: Option<Arc<Session>>,
    api: Option<Api>,
    temp: Option<TorrentSessionDirectory>,
    magnet_uri: String,
    files: Vec<TorrentVideoFile>,
    torrent_id: Option<usize>,
    stream_cancel: Option<CancellationToken>,
    stream_task: Option<JoinHandle<Result<()>>>,
    stream_url: Option<String>,
    runtime: Option<Runtime>,
    progress_cache: Arc<Mutex<Option<TorrentProgress>>>,
    progress_cancel: Option<CancellationToken>,
    progress_task: Option<JoinHandle<()>>,
}

impl TorrentPlaybackSession {
    /// Resolve metadata and enumerate video files without downloading payload data.
    pub async fn inspect(info_hash: &str) -> Result<(Self, Vec<TorrentVideoFile>)> {
        Self::inspect_with_trackers(info_hash, &[]).await
    }

    /// Resolve metadata using tracker sources supplied by the selected add-on.
    pub async fn inspect_with_trackers(
        info_hash: &str,
        trackers: &[String],
    ) -> Result<(Self, Vec<TorrentVideoFile>)> {
        let hash = normalize_v1_info_hash(info_hash)?;
        let magnet_uri = build_magnet_uri(&hash, trackers)?;
        let temp = TorrentSessionDirectory::create_default()?;
        let output_folder = temp.path().to_string_lossy().into_owned();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .context("could not create the torrent session runtime")?;

        let mut options = SessionOptions::default();
        options.disable_upload = true;
        options.listen = None;
        options.persistence = None;
        options.fastresume = false;
        options.disable_local_service_discovery = true;
        if let Some(dht) = options.dht.as_mut() {
            dht.persistence = None;
        }

        options.peer_limit = Some(50);
        options.runtime_worker_threads = Some(2);

        let session_runtime = runtime.handle().clone();
        let listing_magnet_uri = magnet_uri.clone();
        let mut session_task = session_runtime.spawn(async move {
            let session = Session::new_with_opts(PathBuf::from(&output_folder), options)
                .await
                .context("could not start the temporary torrent engine")?;
            let api = Api::new(session.clone(), None);
            let list_options = AddTorrentOptions {
                list_only: true,
                output_folder: Some(output_folder),
                ..Default::default()
            };
            let listing = match api
                .api_add_torrent(
                    AddTorrent::from_url(listing_magnet_uri.as_str()),
                    Some(list_options),
                )
                .await
            {
                Ok(listing) => listing,
                Err(error) => {
                    session.stop().await;
                    return Err(error)
                        .context("could not resolve torrent metadata or list its files");
                }
            };
            let files = video_files_from_response(&listing);
            if files.is_empty() {
                session.stop().await;
                bail!("this torrent does not contain a supported video file");
            }
            Ok::<_, anyhow::Error>((session, api, files))
        });
        let task_result = tokio::time::timeout(Duration::from_secs(45), &mut session_task).await;
        let (session, api, files) = match task_result {
            Ok(Ok(Ok(result))) => result,
            Ok(Ok(Err(error))) => {
                runtime.shutdown();
                return Err(error);
            }
            Ok(Err(error)) => {
                runtime.shutdown();
                return Err(error).context("torrent metadata task panicked");
            }
            Err(_) => {
                session_task.abort();
                let _ = session_task.await;
                runtime.shutdown();
                bail!("timed out while resolving torrent metadata; no peers supplied metadata");
            }
        };

        let playback = Self {
            session: Some(session),
            api: Some(api),
            temp: Some(temp),
            magnet_uri,
            files: files.clone(),
            torrent_id: None,
            stream_cancel: None,
            stream_task: None,
            stream_url: None,
            runtime: Some(runtime),
            progress_cache: Arc::new(Mutex::new(None)),
            progress_cancel: None,
            progress_task: None,
        };
        Ok((playback, files))
    }

    /// Download only the chosen video file and return an MPV-compatible local URL.
    pub async fn start_file(&mut self, file_index: usize) -> Result<String> {
        if self.stream_url.is_some() {
            bail!("a torrent file is already active in this session");
        }
        if !self.files.iter().any(|file| file.index == file_index) {
            bail!("selected file is not a video in the inspected torrent");
        }
        let output_folder = self
            .temp
            .as_ref()
            .context("torrent session is closed")?
            .path()
            .to_string_lossy()
            .into_owned();
        let api = self
            .api
            .as_ref()
            .context("torrent engine is closed")?
            .clone();
        let magnet_uri = self.magnet_uri.clone();
        let runtime_handle = self
            .runtime
            .as_ref()
            .context("torrent runtime is closed")?
            .handle()
            .clone();
        let add_options = AddTorrentOptions {
            only_files: Some(vec![file_index]),
            output_folder: Some(output_folder),
            ..Default::default()
        };
        let add_api = api.clone();
        let add_task = runtime_handle.spawn(async move {
            add_api
                .api_add_torrent(AddTorrent::from_url(magnet_uri.as_str()), Some(add_options))
                .await
                .context("could not start downloading the selected video")
        });
        let added: ApiAddTorrentResponse = add_task
            .await
            .context("torrent download initialization task panicked")??;
        let torrent_id = added
            .id
            .context("torrent engine returned no active torrent ID")?;

        let token = Uuid::new_v4().simple().to_string();
        let cancel = CancellationToken::new();
        let state = StreamServerState {
            api: api.clone(),
            torrent_id,
            token: token.clone(),
        };
        let router = Router::new()
            .route(
                "/{token}/{torrent_id}/{file_id}",
                get(stream_torrent_file).head(stream_torrent_file),
            )
            .with_state(state);
        let shutdown = cancel.clone();
        let (address_tx, address_rx) = tokio::sync::oneshot::channel();
        let task = runtime_handle.spawn(async move {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .context("could not bind the loopback torrent stream endpoint")?;
            let address = listener.local_addr()?;
            let _ = address_tx.send(address);
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
                .context("local torrent stream server failed")
        });
        let address: SocketAddr = address_rx
            .await
            .context("local torrent stream server failed before binding")?;
        let url = format!("http://{address}/{token}/{torrent_id}/{file_index}");
        let progress_cache = self.progress_cache.clone();
        let progress_cancel = CancellationToken::new();
        let monitor_cancel = progress_cancel.clone();
        let progress_api = api.clone();
        let progress_runtime = self.runtime.as_ref().context("torrent runtime is closed")?;
        let progress_task = progress_runtime.handle().spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = monitor_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        let snapshot = progress_api
                            .api_stats_v1(TorrentIdOrHash::Id(torrent_id))
                            .ok()
                            .map(|stats| TorrentProgress {
                                downloaded_bytes: stats.progress_bytes,
                                total_bytes: stats.total_bytes,
                                uploaded_bytes: stats.uploaded_bytes,
                                finished: stats.finished,
                                state: format!("{:?}", stats.state),
                                error: stats.error.clone(),
                            });
                        if let Ok(mut cache) = progress_cache.lock() {
                            *cache = snapshot;
                        }
                    }
                }
            }
        });
        self.torrent_id = Some(torrent_id);
        self.progress_cancel = Some(progress_cancel);
        self.progress_task = Some(progress_task);
        self.stream_cancel = Some(cancel);
        self.stream_task = Some(task);
        self.stream_url = Some(url.clone());
        Ok(url)
    }

    pub fn files(&self) -> &[TorrentVideoFile] {
        &self.files
    }

    pub fn stream_url(&self) -> Option<&str> {
        self.stream_url.as_deref()
    }

    pub fn progress_snapshot(&self) -> Option<TorrentProgress> {
        self.progress_cache
            .lock()
            .ok()
            .and_then(|cache| cache.clone())
    }

    pub fn progress(&self) -> Result<Option<TorrentProgress>> {
        let (Some(api), Some(id)) = (self.api.as_ref(), self.torrent_id) else {
            return Ok(None);
        };
        let stats = api.api_stats_v1(TorrentIdOrHash::Id(id))?;
        Ok(Some(TorrentProgress {
            downloaded_bytes: stats.progress_bytes,
            total_bytes: stats.total_bytes,
            uploaded_bytes: stats.uploaded_bytes,
            finished: stats.finished,
            state: format!("{:?}", stats.state),
            error: stats.error.clone(),
        }))
    }

    /// Stop the HTTP endpoint and torrent before deleting temporary payloads.
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(cancel) = self.progress_cancel.take() {
            cancel.cancel();
        }
        if let Some(mut task) = self.progress_task.take() {
            if tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
        if let Some(cancel) = self.stream_cancel.take() {
            cancel.cancel();
        }
        if let Some(mut task) = self.stream_task.take() {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
        let mut cleanup_errors = Vec::new();
        let api = self.api.take();
        let session = self.session.take();
        let torrent_id = self.torrent_id.take();
        if let Some(runtime) = self.runtime.as_ref() {
            let shutdown_task = runtime.handle().spawn(async move {
                let mut errors = Vec::new();
                if let (Some(api), Some(id)) = (api, torrent_id) {
                    if let Err(error) = api.api_torrent_action_delete(TorrentIdOrHash::Id(id)).await
                    {
                        errors.push(format!("torrent stop failed: {error:#}"));
                    }
                }
                if let Some(session) = session {
                    session.stop().await;
                }
                errors
            });
            match shutdown_task.await {
                Ok(errors) => cleanup_errors.extend(errors),
                Err(error) => {
                    cleanup_errors.push(format!("torrent engine shutdown task failed: {error}"))
                }
            }
        } else if let Some(session) = session {
            session.cancellation_token().cancel();
        }
        self.runtime.take();
        if let Some(temp) = self.temp.take() {
            if let Err(error) = temp.close() {
                cleanup_errors.push(format!("temporary session cleanup failed: {error:#}"));
            }
        }
        if cleanup_errors.is_empty() {
            Ok(())
        } else {
            bail!("{}", cleanup_errors.join("; "))
        }
    }
}

impl Drop for TorrentPlaybackSession {
    fn drop(&mut self) {
        if let Some(cancel) = self.progress_cancel.take() {
            cancel.cancel();
        }
        self.progress_task.take();
        if let Some(cancel) = self.stream_cancel.take() {
            cancel.cancel();
        }
        if let Some(session) = self.session.take() {
            session.cancellation_token().cancel();
        }
        // An un-awaited drop is treated like abnormal termination. Keep payloads
        // for startup recovery rather than deleting files while engine tasks may exit.
        self.api.take();
        self.stream_task.take();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
        if let Some(temp) = self.temp.take() {
            temp.preserve_for_recovery();
        }
    }
}

async fn stream_torrent_file(
    State(state): State<StreamServerState>,
    Path((token, torrent_id, file_id)): Path<(String, usize, usize)>,
    headers: HeaderMap,
) -> Response {
    match stream_torrent_file_inner(state, token, torrent_id, file_id, headers).await {
        Ok(response) => response,
        Err(error) => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    }
}

async fn stream_torrent_file_inner(
    state: StreamServerState,
    token: String,
    torrent_id: usize,
    file_id: usize,
    headers: HeaderMap,
) -> Result<Response> {
    if token != state.token || torrent_id != state.torrent_id {
        bail!("invalid local torrent stream token");
    }
    let mut stream = state
        .api
        .api_stream(TorrentIdOrHash::Id(torrent_id), file_id)
        .await
        .context("could not open torrent file stream")?;
    let total = stream.len();
    let requested_range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let range = match requested_range {
        Some(value) => match parse_single_byte_range(value, total) {
            Ok(range) => Some(range),
            Err(_) => {
                let response = Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .body(Body::empty())
                    .context("could not construct unsatisfied range response")?;
                return Ok(response);
            }
        },
        None => None,
    };
    let (start, end) = range.unwrap_or((0, total));
    let status = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if range.is_some() {
        stream.seek(SeekFrom::Start(start)).await?;
        response_headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{}/{total}", end - 1))?,
        );
    }
    let length = end.saturating_sub(start);
    response_headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string())?,
    );
    let body = Body::from_stream(ReaderStream::new(stream.take(length)));
    let mut response = Response::builder()
        .status(status)
        .body(body)
        .context("could not construct torrent stream response")?;
    response.headers_mut().extend(response_headers);
    Ok(response)
}

fn parse_single_byte_range(value: &str, total: u64) -> Result<(u64, u64)> {
    let range = value
        .strip_prefix("bytes=")
        .context("unsupported range unit")?;
    if range.contains(',') {
        bail!("multiple byte ranges are not supported");
    }
    let (start, end) = range.split_once('-').context("invalid byte range")?;
    let (start, end) = if start.is_empty() {
        let suffix = end.parse::<u64>().context("invalid suffix range")?;
        if suffix == 0 {
            bail!("suffix byte range must be greater than zero");
        }
        (total.saturating_sub(suffix), total)
    } else {
        let start = start.parse::<u64>().context("invalid range start")?;
        let end = if end.is_empty() {
            total
        } else {
            end.parse::<u64>()
                .context("invalid range end")?
                .checked_add(1)
                .context("range end overflow")?
        };
        (start, end)
    };
    if start >= total || end <= start || end > total {
        bail!("requested byte range is not satisfiable");
    }
    Ok((start, end))
}

fn build_magnet_uri(hash: &str, trackers: &[String]) -> Result<String> {
    let mut magnet = url::Url::parse(&format!("magnet:?xt=urn:btih:{hash}"))?;
    {
        let mut query = magnet.query_pairs_mut();
        for source in trackers.iter().take(32) {
            let tracker = source.strip_prefix("tracker:").unwrap_or(source).trim();
            if tracker.is_empty() {
                continue;
            }
            let parsed =
                url::Url::parse(tracker).context("add-on supplied an invalid torrent tracker")?;
            if !matches!(parsed.scheme(), "udp" | "http" | "https")
                || parsed.host().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                bail!("torrent trackers must be absolute UDP, HTTP, or HTTPS URLs");
            }
            query.append_pair("tr", parsed.as_str());
        }
    }
    Ok(magnet.to_string())
}

fn normalize_v1_info_hash(input: &str) -> Result<String> {
    let hash = input.trim().to_ascii_lowercase();
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("this torrent engine currently supports 40-character BitTorrent v1 info hashes");
    }
    Ok(hash)
}

fn video_files_from_response(response: &ApiAddTorrentResponse) -> Vec<TorrentVideoFile> {
    response
        .details
        .files
        .as_ref()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, file)| file.included && is_video_filename(&file.name))
        .map(|(index, file)| TorrentVideoFile {
            index,
            name: file.name.clone(),
            size_bytes: file.length,
        })
        .collect()
}

fn is_video_filename(name: &str) -> bool {
    let extension = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "mkv"
            | "mp4"
            | "m4v"
            | "avi"
            | "mov"
            | "webm"
            | "wmv"
            | "flv"
            | "ts"
            | "m2ts"
            | "mpg"
            | "mpeg"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_only_v1_info_hashes() {
        assert!(normalize_v1_info_hash(&"a".repeat(40)).is_ok());
        assert!(normalize_v1_info_hash(&"A".repeat(40)).is_ok());
        assert!(normalize_v1_info_hash(&"a".repeat(64)).is_err());
        assert!(normalize_v1_info_hash("magnet:?xt=urn:btih:abcd").is_err());
        assert!(normalize_v1_info_hash(&"g".repeat(40)).is_err());
    }

    #[test]
    fn builds_magnet_with_only_valid_explicit_trackers() {
        let magnet = build_magnet_uri(
            &"a".repeat(40),
            &[
                "tracker:udp://tracker.example:1337/announce".into(),
                "https://tracker.example/announce".into(),
            ],
        )
        .unwrap();
        let parsed = url::Url::parse(&magnet).unwrap();
        let pairs = parsed.query_pairs().collect::<Vec<_>>();
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].0, "xt");
        assert_eq!(pairs[1].0, "tr");
        assert!(build_magnet_uri(&"a".repeat(40), &["file:///etc/passwd".into()]).is_err());
    }

    #[test]
    fn validates_single_byte_ranges() {
        assert_eq!(
            parse_single_byte_range("bytes=10-19", 100).unwrap(),
            (10, 20)
        );
        assert_eq!(
            parse_single_byte_range("bytes=90-", 100).unwrap(),
            (90, 100)
        );
        assert_eq!(
            parse_single_byte_range("bytes=-10", 100).unwrap(),
            (90, 100)
        );
        assert_eq!(
            parse_single_byte_range("bytes=-200", 100).unwrap(),
            (0, 100)
        );
        assert!(parse_single_byte_range("bytes=0-10,20-30", 100).is_err());
        assert!(parse_single_byte_range("bytes=100-", 100).is_err());
        assert!(parse_single_byte_range("bytes=20-10", 100).is_err());
        assert!(parse_single_byte_range("bytes=-0", 100).is_err());
    }

    #[test]
    fn recognizes_common_video_files() {
        assert!(is_video_filename("movie.MKV"));
        assert!(is_video_filename("episode.mp4"));
        assert!(!is_video_filename("poster.jpg"));
        assert!(!is_video_filename("sample.nfo"));
    }
}
