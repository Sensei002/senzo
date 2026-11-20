//! Local HTTP downloads and opt-in BitTorrent playback transport.
//!
//! Download payloads are stored separately from metadata caches. Source
//! headers are kept in the operating-system credential vault rather than the
//! queue file. Torrent playback binds its temporary byte-range server to
//! loopback only and removes session data after an orderly engine shutdown.

mod downloads;
mod torrent;
mod torrent_engine;

pub use downloads::{
    DownloadError, DownloadItem, DownloadManager, DownloadRequest, DownloadStatus,
};
pub use torrent::TorrentSessionDirectory;
pub use torrent_engine::{TorrentPlaybackSession, TorrentProgress, TorrentVideoFile};
