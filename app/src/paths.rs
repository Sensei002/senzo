use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::{Context, Result, anyhow};

static APP_PATHS: OnceLock<AppPaths> = OnceLock::new();

#[derive(Clone, Debug)]
pub(crate) struct AppPaths {
    root: PathBuf,
    database: PathBuf,
    logs: PathBuf,
    image_cache: PathBuf,
    streaming_server: PathBuf,
    streaming_server_cache: PathBuf,
    mpv: PathBuf,
    plugins: PathBuf,
    updates: PathBuf,
    downloads: PathBuf,
}

impl AppPaths {
    fn from_root(root: PathBuf) -> Self {
        let cache = root.join("cache");
        Self {
            database: root.join("stremio.db"),
            logs: root.join("logs"),
            image_cache: cache.join("image-cache-v1"),
            streaming_server: root.join("streaming-server"),
            streaming_server_cache: cache.join("streaming-server"),
            mpv: root.join("mpv"),
            plugins: root.join("plugins"),
            updates: root.join("updates"),
            downloads: root.join("downloads"),
            root,
        }
    }

    fn resolve() -> Result<Self> {
        let platform_root = dirs::data_local_dir().ok_or_else(|| {
            anyhow!("the operating system did not provide a local application-data directory")
        })?;
        let legacy_root = platform_root.join("stremio-native");
        let new_root = platform_root.join("SENZO");
        if legacy_root.exists() && !new_root.exists() {
            std::fs::rename(&legacy_root, &new_root).with_context(|| format!(
                "failed to migrate legacy SENZO data from {} to {}",
                legacy_root.display(), new_root.display()
            ))?;
        } else if legacy_root.exists() && new_root.exists() {
            copy_missing_entries(&legacy_root, &new_root)?;
        }
        Ok(Self::from_root(new_root))
    }

    fn create_directories(&self) -> Result<()> {
        for directory in [
            &self.root,
            &self.logs,
            &self.image_cache,
            &self.streaming_server,
            &self.streaming_server.join("logs"),
            &self.streaming_server.join("localFiles"),
            &self.streaming_server_cache,
            &self.mpv,
            &self.plugins,
            &self.updates,
            &self.downloads,
        ] {
            std::fs::create_dir_all(directory)
                .with_context(|| format!("failed to create {}", directory.display()))?;
        }
        Ok(())
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn database(&self) -> &Path {
        &self.database
    }

    pub(crate) fn logs(&self) -> &Path {
        &self.logs
    }

    pub(crate) fn image_cache(&self) -> &Path {
        &self.image_cache
    }

    pub(crate) fn streaming_server(&self) -> &Path {
        &self.streaming_server
    }

    pub(crate) fn streaming_server_cache(&self) -> &Path {
        &self.streaming_server_cache
    }

    pub(crate) fn mpv(&self) -> &Path {
        &self.mpv
    }

    #[cfg(any(feature = "plugins", test))]
    pub(crate) fn plugins(&self) -> &Path {
        &self.plugins
    }

    #[cfg(any(target_os = "windows", test))]
    pub(crate) fn updates(&self) -> &Path {
        &self.updates
    }

    pub(crate) fn downloads(&self) -> &Path {
        &self.downloads
    }
}

fn copy_missing_entries(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("failed to read legacy directory {}", source.display()))? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            if destination_path.exists() {
                copy_missing_entries(&source_path, &destination_path)?;
            } else {
                std::fs::rename(&source_path, &destination_path).or_else(|_| {
                    copy_missing_entries(&source_path, &destination_path)
                        .map_err(|error| std::io::Error::other(error.to_string()))
                })?;
            }
        } else if !destination_path.exists() {
            std::fs::copy(&source_path, &destination_path).with_context(|| format!(
                "failed to preserve legacy file {}", source_path.display()
            ))?;
        }
    }
    Ok(())
}

pub(crate) fn initialize() -> Result<&'static AppPaths> {
    if let Some(paths) = APP_PATHS.get() {
        return Ok(paths);
    }

    let paths = AppPaths::resolve()?;
    paths.create_directories()?;
    let _ = APP_PATHS.set(paths);
    APP_PATHS
        .get()
        .ok_or_else(|| anyhow!("application path registry could not be initialized"))
}

pub(crate) fn get() -> &'static AppPaths {
    APP_PATHS
        .get()
        .expect("application paths must be initialized before subsystem startup")
}

#[cfg(test)]
mod tests {
    use super::AppPaths;
    use std::path::PathBuf;

    #[test]
    fn resolver_uses_senzo_suffix() {
        let paths = AppPaths::resolve().expect("platform data directory should resolve");

        assert_eq!(
            paths.root().file_name().and_then(|name| name.to_str()),
            Some("SENZO")
        );
    }

    #[test]
    fn every_derived_path_stays_below_root() {
        let paths = AppPaths::from_root(PathBuf::from("platform-data").join("SENZO"));
        let derived = [
            paths.database(),
            paths.logs(),
            paths.image_cache(),
            paths.streaming_server(),
            paths.streaming_server_cache(),
            paths.mpv(),
            paths.plugins(),
            paths.updates(),
            paths.downloads(),
        ];

        assert!(
            derived
                .into_iter()
                .all(|path| path.starts_with(paths.root()))
        );
    }
}
