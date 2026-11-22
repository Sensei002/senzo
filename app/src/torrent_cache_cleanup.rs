//! Startup rotation and background cleanup for the embedded stream server's
//! non-persistent torrent payloads.
//!
//! The disk-backed EngineFS is recreated with an empty engine registry on each
//! application launch. Files left in "torrent-cache" therefore cannot be resumed.
//! User-managed downloads are stored separately and are deliberately untouched.
//!
//! Rotation is a same-volume directory rename: the stream server gets a fresh,
//! empty torrent-cache immediately, while a detached worker removes the old tree.
//! This keeps large cache deletion and NTFS directory traversal off the startup path.

use std::path::{Path, PathBuf};
use std::{fs, io, thread, time::{SystemTime, UNIX_EPOCH}};

use anyhow::{Context, anyhow};

const STALE_CACHE_PREFIX: &str = "torrent-cache.stale-";

pub(crate) fn prepare_stale_torrent_cache() -> anyhow::Result<Vec<PathBuf>> {
    prepare_torrent_cache_under(crate::paths::get().streaming_server_cache())
}

fn prepare_torrent_cache_under(cache_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let canonical_cache_root = fs::canonicalize(cache_root)
        .with_context(|| format!("could not resolve stream-server cache root: {}", cache_root.display()))?;
    let torrent_cache = canonical_cache_root.join("torrent-cache");

    // Retry cleanup trees left behind if the application was terminated while a
    // previous background cleanup was running.
    let mut stale_paths = Vec::new();
    for entry in fs::read_dir(&canonical_cache_root)
        .with_context(|| format!("could not read stream-server cache root: {}", canonical_cache_root.display()))?
    {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with(STALE_CACHE_PREFIX) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("could not inspect stale cache entry: {}", path.display()))?;
        if metadata.is_dir() || metadata.file_type().is_symlink() {
            stale_paths.push(path);
        }
    }

    let metadata = match fs::symlink_metadata(&torrent_cache) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(stale_paths),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("could not inspect torrent cache: {}", torrent_cache.display())
            });
        }
    };

    // Never follow a redirected cache root or replace an unexpected filesystem entry.
    if metadata.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to rotate torrent cache because {} is a symbolic link",
            torrent_cache.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(anyhow!(
            "refusing to rotate torrent cache because {} is not a directory",
            torrent_cache.display()
        ));
    }

    let canonical_torrent_cache = fs::canonicalize(&torrent_cache)
        .with_context(|| format!("could not resolve torrent cache: {}", torrent_cache.display()))?;
    if canonical_torrent_cache.parent() != Some(canonical_cache_root.as_path()) {
        return Err(anyhow!(
            "refusing to rotate torrent cache outside the expected directory: {}",
            canonical_torrent_cache.display()
        ));
    }

    let stale_cache = unique_stale_cache_path(&canonical_cache_root)?;
    // This rename is a same-volume metadata operation. If Windows reports that a
    // file is still open, leave the original cache intact and let startup continue.
    fs::rename(&torrent_cache, &stale_cache)
        .with_context(|| format!("could not detach stale torrent cache: {}", torrent_cache.display()))?;

    if let Err(error) = fs::create_dir(&torrent_cache) {
        let restore_result = fs::rename(&stale_cache, &torrent_cache);
        return match restore_result {
            Ok(()) => Err(error).with_context(|| {
                format!("could not recreate torrent cache: {}", torrent_cache.display())
            }),
            Err(restore_error) => Err(anyhow!(
                "could not recreate torrent cache {} ({error}); restoring previous cache also failed ({restore_error})",
                torrent_cache.display()
            )),
        };
    }

    stale_paths.push(stale_cache);
    Ok(stale_paths)
}

fn unique_stale_cache_path(cache_root: &Path) -> anyhow::Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for attempt in 0..100_u32 {
        let candidate = cache_root.join(format!(
            "{STALE_CACHE_PREFIX}{}-{timestamp}-{attempt}",
            std::process::id()
        ));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("could not inspect stale cache destination: {}", candidate.display())
                });
            }
        }
    }

    Err(anyhow!("could not allocate a unique stale torrent-cache directory"))
}

fn remove_stale_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)
    } else if metadata.is_dir() {
        // remove_dir_all does not traverse a symlink entry itself.
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub(crate) fn remove_stale_cache_in_background(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }

    let worker = thread::Builder::new()
        .name("senzo-torrent-cache-cleanup".to_owned())
        .spawn(move || {
            for path in paths {
                match remove_stale_path(&path) {
                    Ok(()) => tracing::info!(
                        path = %path.display(),
                        "background stale torrent-cache removal completed"
                    ),
                    Err(error) => tracing::warn!(
                        path = %path.display(),
                        %error,
                        "background stale torrent-cache removal failed; it will be retried next launch"
                    ),
                }
            }
        });

    if let Err(error) = worker {
        tracing::warn!(
            %error,
            "could not start background torrent-cache cleanup; stale data will be retried next launch"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{prepare_torrent_cache_under, remove_stale_path};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn rotates_cache_before_removal_and_preserves_neighboring_data() -> anyhow::Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp_root = std::env::temp_dir().join(format!(
            "senzo-torrent-cache-rotation-{}-{unique}",
            std::process::id()
        ));
        let cache_root = temp_root.join("streaming-server");
        let torrent_cache = cache_root.join("torrent-cache");
        let movie_dir = torrent_cache.join("abandoned-movie");
        fs::create_dir_all(&movie_dir)?;
        fs::write(movie_dir.join("episode.mkv"), b"temporary episode data")?;
        fs::create_dir_all(torrent_cache.join(".metadata"))?;
        fs::write(torrent_cache.join(".metadata").join("old.torrent"), b"metadata")?;
        let keep_file = cache_root.join("keep-settings.json");
        fs::write(&keep_file, b"preserve")?;

        let stale_paths = prepare_torrent_cache_under(&cache_root)?;

        assert_eq!(stale_paths.len(), 1);
        assert!(torrent_cache.is_dir());
        assert_eq!(fs::read_dir(&torrent_cache)?.count(), 0);
        let stale_cache = &stale_paths[0];
        assert_eq!(
            fs::read(stale_cache.join("abandoned-movie").join("episode.mkv"))?,
            b"temporary episode data"
        );

        remove_stale_path(stale_cache)?;
        assert!(!stale_cache.exists());
        assert_eq!(fs::read(&keep_file)?, b"preserve");

        fs::remove_dir_all(temp_root)?;
        Ok(())
    }

    #[test]
    fn retries_leftover_stale_cache_directories() -> anyhow::Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp_root = std::env::temp_dir().join(format!(
            "senzo-stale-cache-retry-{}-{unique}",
            std::process::id()
        ));
        let cache_root = temp_root.join("streaming-server");
        let leftover = cache_root.join("torrent-cache.stale-previous-run");
        fs::create_dir_all(&leftover)?;
        fs::write(leftover.join("leftover.bin"), b"stale")?;

        let stale_paths = prepare_torrent_cache_under(&cache_root)?;

        assert_eq!(stale_paths, vec![leftover.clone()]);
        remove_stale_path(&leftover)?;
        fs::remove_dir_all(temp_root)?;
        Ok(())
    }
}
