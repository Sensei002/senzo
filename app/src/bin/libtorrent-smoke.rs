//! Minimal Windows smoke test for native libtorrent session startup.
//!
//! Starting the embedded stream server initializes EngineFS and constructs the
//! libtorrent session through the same Rust/C++ bridge used by playback. This
//! catches native startup crashes without launching the full Slint/MPV UI.

use std::{
    net::{Ipv4Addr, SocketAddr},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn main() -> anyhow::Result<()> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "senzo-libtorrent-smoke-{}-{nonce}",
        std::process::id()
    ));

    let mut config = stream_server::ServerConfig::embedded();
    config.http_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    config.config_dir = Some(root.join("config"));
    config.cache_dir = Some(root.join("cache"));
    config.enable_cache_cleaner = false;
    config.graceful_shutdown_timeout = Duration::from_secs(10);

    println!("Starting embedded stream server to verify native libtorrent startup...");
    let server = stream_server::start(config)?;
    let address = server.bound_http_addr();
    println!("Libtorrent session initialized; embedded server bound to {address}");

    server.shutdown()?;
    let shutdown_source = server.join()?;
    println!("Embedded stream server stopped cleanly ({shutdown_source:?})");

    std::fs::remove_dir_all(&root)?;
    Ok(())
}
