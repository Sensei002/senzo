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
    pub fn artwork_cache_dir(&self) -> PathBuf {
        self.cache.join("artwork")
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

/// Native child HWND used as an in-process video host for libmpv on Windows.
#[cfg(target_os = "windows")]
pub struct NativeVideoSurface {
    hwnd: windows_sys::Win32::Foundation::HWND,
}

#[cfg(target_os = "windows")]
impl NativeVideoSurface {
    pub fn create(parent_hwnd: isize) -> Result<Self, String> {
        use windows_sys::Win32::{
            Foundation::GetLastError,
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{
                CreateWindowExW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
            },
        };
        if parent_hwnd == 0 {
            return Err("the SENZO main window handle is not available".to_owned());
        }
        let class_name = "STATIC\0".encode_utf16().collect::<Vec<_>>();
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                0, 0, 1, 1,
                parent_hwnd as windows_sys::Win32::Foundation::HWND,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(format!(
                "could not create the embedded video surface (Win32 error {})",
                unsafe { GetLastError() }
            ));
        }
        Ok(Self { hwnd })
    }

    pub fn hwnd(&self) -> isize { self.hwnd as isize }

    pub fn set_bounds(&self, x: i32, y: i32, width: i32, height: i32) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::GetLastError,
            UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW},
        };
        let result = unsafe {
            SetWindowPos(
                self.hwnd, std::ptr::null_mut(), x, y, width.max(1), height.max(1),
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
            )
        };
        if result == 0 {
            return Err(format!(
                "could not position the embedded video surface (Win32 error {})",
                unsafe { GetLastError() }
            ));
        }
        Ok(())
    }

    pub fn hide(&self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe { ShowWindow(self.hwnd, SW_HIDE); }
    }
}

#[cfg(target_os = "windows")]
impl Drop for NativeVideoSurface {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
        if !self.hwnd.is_null() { unsafe { DestroyWindow(self.hwnd); } }
    }
}

/// Non-Windows builds retain the platform API while the Linux player surface is deferred.
#[cfg(not(target_os = "windows"))]
pub struct NativeVideoSurface;

#[cfg(not(target_os = "windows"))]
impl NativeVideoSurface {
    pub fn create(_parent_hwnd: isize) -> Result<Self, String> {
        Err("embedded MPV surfaces are currently supported on Windows only".to_owned())
    }
    pub fn hwnd(&self) -> isize { 0 }
    pub fn set_bounds(&self, _x: i32, _y: i32, _width: i32, _height: i32) -> Result<(), String> { Ok(()) }
    pub fn hide(&self) {}
}
