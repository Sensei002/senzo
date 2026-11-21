use anyhow::{Context, Result};
use libtorrent_sys::{LibtorrentSession, SessionSettings};

fn main() -> Result<()> {
    println!("libtorrent version: {}", libtorrent_sys::version());

    // Mirror the key runtime options without compiling the full stream-server
    // and Slint desktop dependency graph just to check native session startup.
    let settings = SessionSettings {
        user_agent: "stream-server/1.0".to_owned(),
        enable_dht: true,
        enable_pex: true,
        enable_lsd: true,
        enable_upnp: true,
        enable_natpmp: true,
        active_downloads: 8,
        active_seeds: 16,
        active_limit: 24,
        announce_to_all_trackers: true,
        announce_to_all_tiers: true,
        ..SessionSettings::default()
    };

    println!("Creating disk-backed libtorrent session...");
    let session = LibtorrentSession::new_disk_backed(settings)
        .context("native libtorrent session construction failed")?;
    println!("Disk-backed libtorrent session started successfully.");

    drop(session);
    println!("Native libtorrent session shut down cleanly.");
    Ok(())
}
