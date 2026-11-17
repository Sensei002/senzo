//! Operating-system paths and native credential storage.

use std::path::PathBuf;

use keyring::Entry;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("the operating system did not provide a local data directory")]
    DataDirectoryUnavailable,
    #[error("the operating system did not provide a configuration directory")]
    ConfigDirectoryUnavailable,
    #[error("the operating system did not provide a cache directory")]
    CacheDirectoryUnavailable,
    #[error("could not create an application directory: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct AppDirectories {
    data: PathBuf,
    config: PathBuf,
    cache: PathBuf,
}

impl AppDirectories {
    pub fn discover() -> Result<Self, PlatformError> {
        let data = dirs::data_local_dir()
            .ok_or(PlatformError::DataDirectoryUnavailable)?
            .join("SENZO");
        let config = dirs::config_dir()
            .ok_or(PlatformError::ConfigDirectoryUnavailable)?
            .join("SENZO");
        let cache = dirs::cache_dir()
            .ok_or(PlatformError::CacheDirectoryUnavailable)?
            .join("SENZO");
        for directory in [&data, &config, &cache] {
            std::fs::create_dir_all(directory)?;
        }
        Ok(Self {
            data,
            config,
            cache,
        })
    }

    pub fn database_file(&self) -> PathBuf {
        self.data.join("senzo.sqlite3")
    }
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data
    }
    pub fn config_dir(&self) -> &std::path::Path {
        &self.config
    }
    pub fn cache_dir(&self) -> &std::path::Path {
        &self.cache
    }
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("native credential storage operation failed")]
    Backend(#[source] keyring::Error),
}

pub trait CredentialStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, CredentialError>;
    fn set(&self, account: &str, secret: &str) -> Result<(), CredentialError>;
    fn delete(&self, account: &str) -> Result<(), CredentialError>;
}

/// Windows uses Credential Manager; Linux uses Secret Service and keyutils.
pub struct KeyringCredentialStore {
    service: String,
}

impl KeyringCredentialStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> Result<Entry, CredentialError> {
        Entry::new(&self.service, account).map_err(CredentialError::Backend)
    }
}

impl CredentialStore for KeyringCredentialStore {
    fn get(&self, account: &str) -> Result<Option<String>, CredentialError> {
        match self.entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(CredentialError::Backend(error)),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), CredentialError> {
        self.entry(account)?
            .set_password(secret)
            .map_err(CredentialError::Backend)
    }

    fn delete(&self, account: &str) -> Result<(), CredentialError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(CredentialError::Backend(error)),
        }
    }
}
