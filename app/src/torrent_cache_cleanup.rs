//! Startup cleanup for the embedded stream server's non-persistent torrent payloads.
//!
//! The disk-backed EngineFS is recreated with an empty engine registry on each
//! application launch. Files left in "torrent-cache" therefore cannot be resumed.
//! SENZO's user-managed downloads are stored separately under the app's "downloads"
//! directory and are deliberately outside this cleanup's scope.

use std::path::Path;
use std::{fs, io};

use anyhow::{Context, anyhow};

pub(crate) fn clear_stale_torrent_cache() -> anyhow::Result<usize> {
    clear_torrent_cache_under(crate::paths::get().streaming_server_cache())
}

fn clear_torrent_cache_under(cache_root: &Path) -> anyhow::Result<usize> {
    let canonical_cache_root = fs::canonicalize(cache_root)
        .with_context(|| format!("could not resolve stream-server cache root: {}", cache_root.display()))?;
    let torrent_cache = canonical_cache_root.join("torrent-cache");

    let metadata = match fs::symlink_metadata(&torrent_cache) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("could not inspect torrent cache: {}", torrent_cache.display())
            });
        }
    };

    // Never follow a redirected cache root or replace an unexpected filesystem entry.
    if metadata.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to clean torrent cache because {} is a symbolic link",
            torrent_cache.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(anyhow!(
            "refusing to clean torrent cache because {} is not a directory",
            torrent_cache.display()
        ));
    }

    let canonical_torrent_cache = fs::canonicalize(&torrent_cache)
        .with_context(|| format!("could not resolve torrent cache: {}", torrent_cache.display()))?;
    if canonical_torrent_cache.parent() != Some(canonical_cache_root.as_path()) {
        return Err(anyhow!(
            "refusing to clean torrent cache outside the expected directory: {}",
            canonical_torrent_cache.display()
        ));
    }

    // remove_dir_all removes symlink entries without traversing their targets.
    // The app's single-instance lock is already held before this runs, so no live
    // SENZO streaming session can be writing into this directory.
    let mut removed_entries = 0usize;
    for entry in fs::read_dir(&torrent_cache)
        .with_context(|| format!("could not read torrent cache: {}", torrent_cache.display()))?
    {
        entry?;
        removed_entries = removed_entries.saturating_add(1);
    }

    fs::remove_dir_all(&torrent_cache)
        .with_context(|| format!("could not remove stale torrent cache: {}", torrent_cache.display()))?;
    fs::create_dir_all(&torrent_cache)
        .with_context(|| format!("could not recreate torrent cache: {}", torrent_cache.display()))?;

    Ok(removed_entries)
}

#[cfg(test)]
mod tests {
    use super::clear_torrent_cache_under;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn clears_only_the_torrent_cache_and_preserves_neighboring_data() -> anyhow::Result<()> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let temp_root = std::env::temp_dir().join(format!(
            "senzo-torrent-cache-cleanup-{}-{unique}",
            std::process::id()
        ));
        let cache_root = temp_root.join("streaming-server");
        let torrent_cache = cache_root.join("torrent-cache");
        let movie_dir = torrent_cache.join("abandoned-movie");
        fs::create_dir_all(&movie_dir)?;
        fs::write(movie_dir.join("movie.mkv"), b"temporary movie data")?;
        fs::create_dir_all(torrent_cache.join(".metadata"))?;
        fs::write(torrent_cache.join(".metadata").join("old.torrent"), b"metadata")?;
        let keep_file = cache_root.join("keep-settings.json");
        fs::write(&keep_file, b"preserve")?;

        let removed_entries = clear_torrent_cache_under(&cache_root)?;

        assert_eq!(removed_entries, 2);
        assert!(torrent_cache.is_dir());
        assert_eq!(fs::read_dir(&torrent_cache)?.count(), 0);
        assert_eq!(fs::read(&keep_file)?, b"preserve");

        fs::remove_dir_all(temp_root)?;
        Ok(())
    }
}
