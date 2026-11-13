//! Operating-system-backed credential storage.
//!
//! Provider secrets must never be placed in SQLite, settings JSON, logs, or support bundles.
//! On Windows this adapter uses Windows Credential Manager through the keyring crate.

use anyhow::{Context, bail};
use keyring::Entry;
use std::sync::Mutex;
use zeroize::Zeroize;

const SERVICE: &str = "SENZO";
const OPENSUBTITLES_ACCOUNT: &str = "opensubtitles.com/api-key";
const MAX_CREDENTIAL_BYTES: usize = 4096;
static CREDENTIAL_LOCK: Mutex<()> = Mutex::new(());

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
        let _guard = CREDENTIAL_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("credential store lock is unavailable"))?;
        Self::entry()?
            .set_password(api_key)
            .context("could not save the OpenSubtitles key to Windows Credential Manager")
    }

    /// Returns the stored key, None only when no credential has been configured.
    pub fn load_api_key() -> anyhow::Result<Option<String>> {
        let _guard = CREDENTIAL_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("credential store lock is unavailable"))?;
        let entry = Self::entry()?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).context("could not read Windows Credential Manager"),
        }
    }

    /// Removes the saved key. Missing credentials are treated as an already-cleared state.
    pub fn clear_api_key() -> anyhow::Result<()> {
        let _guard = CREDENTIAL_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("credential store lock is unavailable"))?;
        let entry = Self::entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error)
                .context("could not remove the OpenSubtitles key from Windows Credential Manager"),
        }
    }

    /// Prompts for the API key using Windows' masked native credential dialog.
    /// The dialog does not persist credentials; SENZO stores the validated value
    /// in its dedicated Windows Credential Manager entry.
    pub fn prompt_and_save_api_key() -> anyhow::Result<bool> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Security::Credentials::{
                CredUIPromptForCredentialsW, CREDUI_FLAGS_ALWAYS_SHOW_UI,
                CREDUI_FLAGS_DO_NOT_PERSIST, CREDUI_FLAGS_GENERIC_CREDENTIALS,
                CREDUI_FLAGS_PASSWORD_ONLY_OK,
            };

            let target: Vec<u16> = "OpenSubtitles.com API key - SENZO"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut username = [0u16; 1];
            let mut password = vec![0u16; MAX_CREDENTIAL_BYTES + 1];
            let flags = CREDUI_FLAGS_ALWAYS_SHOW_UI
                | CREDUI_FLAGS_DO_NOT_PERSIST
                | CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_PASSWORD_ONLY_OK;

            let result = unsafe {
                CredUIPromptForCredentialsW(
                    std::ptr::null(),
                    target.as_ptr(),
                    std::ptr::null(),
                    0,
                    username.as_mut_ptr(),
                    username.len() as u32,
                    password.as_mut_ptr(),
                    password.len() as u32,
                    std::ptr::null_mut(),
                    flags,
                )
            };

            if result == 1223 {
                password.zeroize();
                return Ok(false);
            }
            if result != 0 {
                password.zeroize();
                bail!("Windows secure credential prompt failed (error {result})");
            }

            let length = password.iter().position(|unit| *unit == 0).unwrap_or(password.len());
            let decoded = String::from_utf16(&password[..length]);
            password.zeroize();
            let mut api_key = decoded.context("the entered API key contains invalid text")?;
            let save_result = Self::save_api_key(&api_key);
            api_key.zeroize();
            save_result?;
            Ok(true)
        }

        #[cfg(not(windows))]
        {
            bail!("secure API-key entry is currently supported on Windows only");
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
