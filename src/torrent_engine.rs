//! Embedded torrent engine and loopback-only HTTP range streaming.
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, bail};
use axum::{
    Router, body::Body, extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response, routing::get,
};
use librqbit::{
    AddTorrent, AddTorrentOptions, Session, SessionOptions,
    api::{Api, ApiAddTorrentResponse, TorrentIdOrHash},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
    net::TcpListener,
    task::JoinHandle,
};
use tokio_util::{io::ReaderStream, sync::CancellationToken};
use uuid::Uuid;

use crate::torrent::TorrentSessionDirectory;

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
}

impl TorrentPlaybackSession {
    /// Resolve metadata and enumerate video files without downloading payload data.
    pub async fn inspect(info_hash: &str) -> Result<(Self, Vec<TorrentVideoFile>)> {
        let hash = normalize_v1_info_hash(info_hash)?;
        let temp = TorrentSessionDirectory::create_default()?;
        let output_folder = temp.path().to_string_lossy().into_owned();
        let magnet_uri = format!("magnet:?xt=urn:btih:{hash}");

        let mut options = SessionOptions::default();
        options.disable_upload = true;
        options.listen = None;
        options.persistence = None;
        options.fastresume = false;
        options.disable_local_service_discovery = true;
        if let Some(dht) = options.dht.as_mut() {
            dht.persistence = None;
        }

        let session = Session::new_with_opts(PathBuf::from(&output_folder), options)
            .await
            .context("could not start the temporary torrent engine")?;
        let api = Api::new(session.clone(), None);
        let list_options = AddTorrentOptions {
            list_only: true,
            output_folder: Some(output_folder),
            ..Default::default()
        };
        let listing = api
            .api_add_torrent(AddTorrent::from_url(magnet_uri.as_str()), Some(list_options))
            .await
            .context("could not resolve torrent metadata or list its files")?;
        let files = video_files_from_response(&listing);
        if files.is_empty() {
            session.cancellation_token().cancel();
            bail!("this torrent does not contain a supported video file");
        }

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
        let output_folder = self.temp.as_ref()
            .context("torrent session is closed")?
            .path().to_string_lossy().into_owned();
        let api = self.api.as_ref().context("torrent engine is closed")?;
        let add_options = AddTorrentOptions {
            only_files: Some(vec![file_index]),
            output_folder: Some(output_folder),
            ..Default::default()
        };
        let added: ApiAddTorrentResponse = api
            .api_add_torrent(AddTorrent::from_url(self.magnet_uri.as_str()), Some(add_options))
            .await
            .context("could not start downloading the selected video")?;
        let torrent_id = added.id.context("torrent engine returned no active torrent ID")?;

        let token = Uuid::new_v4().simple().to_string();
        let cancel = CancellationToken::new();
        let state = StreamServerState {
            api: api.clone(),
            torrent_id,
            token: token.clone(),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await
            .context("could not bind the loopback torrent stream endpoint")?;
        let address: SocketAddr = listener.local_addr()?;
        let router = Router::new()
            .route("/{token}/{torrent_id}/{file_id}", get(stream_torrent_file))
            .with_state(state);
        let shutdown = cancel.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
                .context("local torrent stream server failed")
        });
        let url = format!("http://{address}/{token}/{torrent_id}/{file_index}");
        self.torrent_id = Some(torrent_id);
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
        }))
    }

    /// Stop the HTTP endpoint and torrent before deleting temporary payloads.
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(cancel) = self.stream_cancel.take() {
            cancel.cancel();
        }
        if let Some(task) = self.stream_task.take() {
            task.await.context("torrent stream server task panicked")??;
        }
        if let (Some(api), Some(id)) = (self.api.as_ref(), self.torrent_id.take()) {
            api.api_torrent_action_delete(TorrentIdOrHash::Id(id))
                .await
                .context("could not stop the torrent and close its files")?;
        }
        if let Some(session) = self.session.take() {
            session.cancellation_token().cancel();
            drop(session);
        }
        self.api.take();
        if let Some(temp) = self.temp.take() {
            temp.close()?;
        }
        Ok(())
    }
}

impl Drop for TorrentPlaybackSession {
    fn drop(&mut self) {
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
        if let Some(temp) = self.temp.take() {
            temp.preserve_for_recovery();
        }
    }
}

async fn stream_torrent_file(
    State(state): State<StreamServerState>,
    Path((token, torrent_id, file_id)): Path<(String, usize, usize)>,
    headers: HeaderMap,
) -> Result<Response> {
    if token != state.token || torrent_id != state.torrent_id {
        bail!("invalid local torrent stream token");
    }
    let mut stream = state.api
        .api_stream(TorrentIdOrHash::Id(torrent_id), file_id)
        .await
        .context("could not open torrent file stream")?;
    let total = stream.len();
    let (start, end) = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        Some(value) => parse_single_byte_range(value, total)?,
        None => (0, total),
    };
    let mut status = StatusCode::OK;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if start > 0 || end < total {
        status = StatusCode::PARTIAL_CONTENT;
        stream.seek(SeekFrom::Start(start)).await?;
        response_headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{}/{total}", end - 1))?,
        );
    }
    let length = end.saturating_sub(start);
    response_headers.insert(header::CONTENT_LENGTH, HeaderValue::from_str(&length.to_string())?);
    let body = Body::from_stream(ReaderStream::new(stream.take(length)));
    let mut response = Response::builder().status(status).body(body)
        .context("could not construct torrent stream response")?;
    response.headers_mut().extend(response_headers);
    Ok(response)
}

fn parse_single_byte_range(value: &str, total: u64) -> Result<(u64, u64)> {
    let range = value.strip_prefix("bytes=").context("unsupported range unit")?;
    if range.contains(',') {
        bail!("multiple byte ranges are not supported");
    }
    let (start, end) = range.split_once('-').context("invalid byte range")?;
    let start = start.parse::<u64>().context("invalid range start")?;
    let end = if end.is_empty() {
        total
    } else {
        end.parse::<u64>().context("invalid range end")?
            .checked_add(1).context("range end overflow")?
    };
    if start >= total || end <= start || end > total {
        bail!("requested byte range is not satisfiable");
    }
    Ok((start, end))
}

fn normalize_v1_info_hash(input: &str) -> Result<String> {
    let hash = input.trim().to_ascii_lowercase();
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("this torrent engine currently supports 40-character BitTorrent v1 info hashes");
    }
    Ok(hash)
}

fn video_files_from_response(response: &ApiAddTorrentResponse) -> Vec<TorrentVideoFile> {
    response.details.files.as_ref().into_iter().flatten().enumerate()
        .filter(|(_, file)| file.included && is_video_filename(&file.name))
        .map(|(index, file)| TorrentVideoFile {
            index,
            name: file.name.clone(),
            size_bytes: file.length,
        })
        .collect()
}

fn is_video_filename(name: &str) -> bool {
    let extension = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    matches!(extension.as_str(),
        "mkv" | "mp4" | "m4v" | "avi" | "mov" | "webm" | "wmv" | "flv" | "ts" | "m2ts" | "mpg" | "mpeg"
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
    fn validates_single_byte_ranges() {
        assert_eq!(parse_single_byte_range("bytes=10-19", 100).unwrap(), (10, 20));
        assert_eq!(parse_single_byte_range("bytes=90-", 100).unwrap(), (90, 100));
        assert!(parse_single_byte_range("bytes=0-10,20-30", 100).is_err());
        assert!(parse_single_byte_range("bytes=100-", 100).is_err());
        assert!(parse_single_byte_range("bytes=20-10", 100).is_err());
    }

    #[test]
    fn recognizes_common_video_files() {
        assert!(is_video_filename("movie.MKV"));
        assert!(is_video_filename("episode.mp4"));
        assert!(!is_video_filename("poster.jpg"));
        assert!(!is_video_filename("sample.nfo"));
    }
}
