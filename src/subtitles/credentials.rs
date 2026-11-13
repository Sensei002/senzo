//! Operating-system-backed credential storage.
//!
//! Provider secrets must never be placed in SQLite, settings JSON, logs, or support bundles.
//! On Windows this adapter uses Windows Credential Manager through the keyring crate.

use anyhow::{Context, bail};
use keyring::Entry;

const SERVICE: &str = "SENZO";
const OPENSUBTITLES_ACCOUNT: &str = "opensubtitles.com/api-key";
const MAX_CREDENTIAL_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, Default)]
pub struct OpenSubtitlesCredentialStore;

impl OpenSubtitlesCredentialStore {
    fn entry() -> anyhow::Result<Entry> {
        Entry::new(SERVICE, OPENSUBTITLES_ACCOUNT)
            .context("could not access the Windows credential entry")
    }

    /// Saves a user-provided OpenSubtitles application key to Windows Credential Manager.
    pub fn save_api_key(api_key: &str) -> anyhow::Result<()> {
        let api_key = validate_api_key(api_key)?;
        Self::entry()?
            .set_password(api_key)
            .context("could not save the OpenSubtitles key to Windows Credential Manager")
    }

    /// Returns the stored key, None only when no credential has been configured.
    pub fn load_api_key() -> anyhow::Result<Option<String>> {
        let entry = Self::entry()?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).context("could not read Windows Credential Manager"),
        }
    }

    /// Removes the saved key. Missing credentials are treated as an already-cleared state.
    pub fn clear_api_key() -> anyhow::Result<()> {
        let entry = Self::entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error)
                .context("could not remove the OpenSubtitles key from Windows Credential Manager"),
        }
    }

    /// Reports presence without returning or displaying the secret itself.
    pub fn has_api_key() -> anyhow::Result<bool> {
        Ok(Self::load_api_key()?.is_some())
    }
}

fn validate_api_key(value: &str) -> anyhow::Result<&str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("OpenSubtitles API key cannot be empty");
    }
    if value.len() > MAX_CREDENTIAL_BYTES {
        bail!("OpenSubtitles API key exceeds the supported length");
    }
    if value.chars().any(char::is_control) {
        bail!("OpenSubtitles API key contains unsupported control characters");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_validation_trims_whitespace_and_rejects_empty_values() {
        assert_eq!(validate_api_key("  test-key  ").unwrap(), "test-key");
        assert!(validate_api_key(" \n ").is_err());
    }

    #[test]
    fn api_key_validation_rejects_oversized_and_control_input() {
        assert!(validate_api_key(&"x".repeat(MAX_CREDENTIAL_BYTES + 1)).is_err());
        assert!(validate_api_key("key\nvalue").is_err());
    }
}
