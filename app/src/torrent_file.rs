//! Minimal `.torrent` parsing for click-to-play file opening.
//!
//! The streaming server already understands torrent blobs, but the desktop
//! shell never decoded one for playback. This module extracts just enough to
//! identify the torrent, pick the main video file, and build a native player
//! stream without depending on the core model's private parser.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_bencode::value::Value;
use sha1::{Digest, Sha1};

const MAX_TORRENT_BYTES: u64 = 16 * 1024 * 1024;
const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "webm", "ts", "m2ts", "mpg", "mpeg", "wmv", "flv",
];

#[derive(Debug, Clone)]
pub struct TorrentFile {
    pub info_hash: [u8; 20],
    pub announce: Vec<String>,
    pub name: String,
    pub file_idx: Option<u16>,
    pub file_path: Option<String>,
}

#[derive(Deserialize)]
struct RawTorrent {
    info: Value,
    #[serde(default)]
    announce: Option<String>,
    #[serde(default, rename = "announce-list")]
    announce_list: Option<Vec<Vec<String>>>,
}

#[derive(Debug)]
struct Candidate {
    idx: usize,
    path: String,
    length: u64,
}

pub fn read(path: &Path) -> Result<(Vec<u8>, TorrentFile)> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("could not inspect torrent {}", path.display()))?;
    if metadata.len() > MAX_TORRENT_BYTES {
        bail!("torrent file is larger than 16 MiB");
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("could not read torrent {}", path.display()))?;
    let parsed = parse(&bytes)?;
    Ok((bytes, parsed))
}

pub fn parse(bytes: &[u8]) -> Result<TorrentFile> {
    if bytes.len() as u64 > MAX_TORRENT_BYTES {
        bail!("torrent file is larger than 16 MiB");
    }
    let raw = serde_bencode::from_bytes::<RawTorrent>(bytes)
        .context("torrent is not valid bencoded data")?;
    let Value::Dict(info) = &raw.info else {
        bail!("torrent info dictionary is missing");
    };
    let info_bytes = serde_bencode::to_bytes(&raw.info)
        .context("could not re-encode the torrent info dictionary")?;
    let mut info_hash = [0u8; 20];
    info_hash.copy_from_slice(&Sha1::digest(&info_bytes));

    let mut announce = Vec::new();
    if let Some(tracker) = raw.announce {
        announce.push(tracker);
    }
    if let Some(groups) = raw.announce_list {
        for group in groups {
            announce.extend(group);
        }
    }
    announce.retain(|tracker| !tracker.trim().is_empty());
    announce.sort();
    announce.dedup();

    let name = info
        .get(b"name".as_slice())
        .and_then(value_text)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "SENZO torrent".to_owned());

    let mut candidates = Vec::new();
    if let Some(Value::List(files)) = info.get(b"files".as_slice()) {
        for (idx, file) in files.iter().enumerate() {
            let Value::Dict(file) = file else { continue };
            let Some(length) = value_int(file.get(b"length".as_slice())) else {
                continue;
            };
            if length <= 0 {
                continue;
            }
            let path = file
                .get(b"path".as_slice())
                .and_then(|value| match value {
                    Value::List(parts) => Some(
                        parts
                            .iter()
                            .filter_map(value_text)
                            .filter(|part| !part.is_empty())
                            .collect::<Vec<_>>()
                            .join("/"),
                    ),
                    _ => None,
                })
                .filter(|path| !path.is_empty());
            let Some(path) = path else { continue };
            candidates.push(Candidate {
                idx,
                path,
                length: length as u64,
            });
        }
    }

    if candidates.is_empty() {
        return Ok(TorrentFile {
            info_hash,
            announce,
            name,
            file_idx: None,
            file_path: None,
        });
    }

    let selected = candidates
        .iter()
        .filter(|candidate| is_video(&candidate.path))
        .max_by_key(|candidate| candidate.length)
        .or_else(|| candidates.iter().max_by_key(|candidate| candidate.length))
        .expect("candidates is not empty");
    let file_idx = u16::try_from(selected.idx).ok();
    let display = selected
        .path
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(&name)
        .to_owned();

    Ok(TorrentFile {
        info_hash,
        announce,
        name: display,
        file_idx,
        file_path: Some(selected.path.clone()),
    })
}

fn is_video(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            VIDEO_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

fn value_int(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Int(value)) => Some(*value),
        _ => None,
    }
}

pub fn looks_like_torrent_path(value: &str) -> bool {
    if value.contains("://") && !value.to_ascii_lowercase().starts_with("file://") {
        return false;
    }
    let path = PathBuf::from(value);
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bencode_single() -> Vec<u8> {
        b"d4:infod6:length4:10004:name8:test.mkvee".to_vec()
    }

    #[test]
    fn parses_single_file_torrent() {
        let torrent = parse(&bencode_single()).expect("single-file torrent");
        assert_eq!(torrent.name, "test.mkv");
        assert_eq!(torrent.file_idx, None);
        assert_eq!(torrent.file_path, None);
        assert_eq!(
            hex::encode(torrent.info_hash),
            "5c33a2df62b1d462851076f0f767f701bf87e1ef"
        );
    }

    #[test]
    fn selects_largest_video_from_multi_file_torrent() {
        let bytes = b"d4:infod5:filesld6:lengthi10e4:pathl5:a.srteed6:lengthi99e4:pathl5:b.mkveed6:lengthi20e4:pathl5:c.txteee4:name4:showee".to_vec();
        let torrent = parse(&bytes).expect("multi-file torrent");
        assert_eq!(torrent.name, "b.mkv");
        assert_eq!(torrent.file_idx, Some(1));
        assert_eq!(torrent.file_path.as_deref(), Some("b.mkv"));
    }

    #[test]
    fn ignores_non_torrent_paths() {
        assert!(looks_like_torrent_path("C:\\media\\movie.torrent"));
        assert!(looks_like_torrent_path("movie.TORRENT"));
        assert!(!looks_like_torrent_path("movie.mp4"));
    }
}
