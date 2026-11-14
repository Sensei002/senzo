//! Ephemeral torrent playback-session storage and crash recovery.
//!
//! Torrent payloads are never stored alongside the permanent metadata cache.
//! A session directory is created under the user's local application data and
//! removed when the session closes. Leftover session directories are removed
//! during the next application startup.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

const SESSION_PREFIX: &str = "session-";
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Owns one isolated directory for transient torrent payloads.
#[derive(Debug)]
pub struct TorrentSessionDirectory {
    root: PathBuf,
    path: Option<PathBuf>,
}

impl TorrentSessionDirectory {
    /// Creates the per-user torrent temporary root and removes abandoned sessions.
    ///
    /// This must be called once during startup, before a new session is created.
    pub fn recover_default_root() -> Result<PathBuf> {
        let local_data = dirs::data_local_dir()
            .context("the operating system did not provide a local app-data directory")?;
        let root = local_data.join("SENZO").join("torrent-temp");
        Self::recover_abandoned_sessions_at(&root)?;
        Ok(root)
    }

    /// Removes only direct child directories using SENZO's session naming convention.
    ///
    /// Symlinks, files, and unrelated directories are deliberately left untouched.
    pub fn recover_abandoned_sessions_at(root: &Path) -> Result<usize> {
        fs::create_dir_all(root)
            .with_context(|| format!("failed to create torrent temp root {}", root.display()))?;
        let mut removed = 0usize;
        for entry in fs::read_dir(root)
            .with_context(|| format!("failed to enumerate torrent temp root {}", root.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_dir()
                || !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(SESSION_PREFIX)
            {
                continue;
            }
            match fs::remove_dir_all(entry.path()) {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    // Windows may still have a locked file after an abnormal shutdown.
                    // Leave it for another startup cleanup attempt.
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to remove abandoned torrent session {}",
                            entry.path().display()
                        )
                    });
                }
            }
        }
        Ok(removed)
    }

    /// Creates a collision-safe, uniquely named session directory.
    pub fn create_at(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create torrent temp root {}", root.display()))?;
        for _ in 0..128 {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "{SESSION_PREFIX}{timestamp}-{}-{counter}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        root,
                        path: Some(path),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to create torrent session {}", path.display())
                    });
                }
            }
        }
        bail!("could not allocate a unique torrent session directory")
    }

    pub fn create_default() -> Result<Self> {
        let root = Self::recover_default_root()?;
        Self::create_at(root)
    }

    pub fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("session path is present until close")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Explicitly removes the session directory after the torrent engine has stopped.
    ///
    /// If Windows still holds a file open, the directory remains for startup recovery.
    pub fn close(mut self) -> Result<()> {
        self.remove_directory()
    }

    fn remove_directory(&mut self) -> Result<()> {
        let Some(path) = self.path.take() else {
            return Ok(());
        };
        match fs::remove_dir_all(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                self.path = Some(path);
                Ok(())
            }
            Err(error) => {
                self.path = Some(path.clone());
                Err(error)
                    .with_context(|| format!("failed to remove torrent session {}", path.display()))
            }
        }
    }
}

impl Drop for TorrentSessionDirectory {
    fn drop(&mut self) {
        let _ = self.remove_directory();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_unique_isolated_session_directories() {
        let temp = tempdir().unwrap();
        let first = TorrentSessionDirectory::create_at(temp.path()).unwrap();
        let second = TorrentSessionDirectory::create_at(temp.path()).unwrap();
        assert_ne!(first.path(), second.path());
        assert!(first.path().is_dir());
        assert!(second.path().is_dir());
        assert!(first.path().starts_with(first.root()));
    }

    #[test]
    fn close_removes_only_its_session_payload() {
        let temp = tempdir().unwrap();
        let session = TorrentSessionDirectory::create_at(temp.path()).unwrap();
        let payload = session.path().join("movie.mkv");
        fs::write(&payload, b"temporary payload").unwrap();
        let path = session.path().to_path_buf();
        session.close().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn startup_recovery_removes_abandoned_sessions_and_preserves_unrelated_entries() {
        let temp = tempdir().unwrap();
        let abandoned = temp.path().join("session-old-crash");
        let unrelated = temp.path().join("keep-me");
        let file = temp.path().join("notes.txt");
        fs::create_dir_all(&abandoned).unwrap();
        fs::create_dir_all(&unrelated).unwrap();
        fs::write(abandoned.join("partial.mkv"), b"partial").unwrap();
        fs::write(unrelated.join("important.txt"), b"keep").unwrap();
        fs::write(&file, b"keep").unwrap();

        assert_eq!(
            TorrentSessionDirectory::recover_abandoned_sessions_at(temp.path()).unwrap(),
            1
        );
        assert!(!abandoned.exists());
        assert!(unrelated.join("important.txt").exists());
        assert!(file.exists());
    }

    #[cfg(unix)]
    #[test]
    fn startup_recovery_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;
        let temp = tempdir().unwrap();
        let outside = temp.path().join("outside");
        let root = temp.path().join("root");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        symlink(&outside, root.join("session-link")).unwrap();

        assert_eq!(
            TorrentSessionDirectory::recover_abandoned_sessions_at(&root).unwrap(),
            0
        );
        assert!(outside.join("keep.txt").exists());
    }

    #[test]
    fn dropping_session_removes_its_directory() {
        let temp = tempdir().unwrap();
        let session = TorrentSessionDirectory::create_at(temp.path()).unwrap();
        let path = session.path().to_path_buf();
        drop(session);
        assert!(!path.exists());
    }
}
