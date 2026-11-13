//! Minimal native Win32 child-window owner for MPV video embedding.
use anyhow::{Context as _, bail};
use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ptr;
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, IsWindow, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW,
        SetWindowPos, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
    },
};


/// Obtain the native Win32 parent HWND from GPUI's public raw-window-handle API.
pub fn gpui_parent_hwnd(window: &Window) -> anyhow::Result<usize> {
    let handle = window
        .window_handle()
        .context("GPUI did not expose a native window handle")?
        .as_raw();

    match handle {
        RawWindowHandle::Win32(handle) => {
            usize::try_from(handle.hwnd.get()).context("GPUI returned an invalid Win32 HWND")
        }
        _ => bail!("SENZO's native MPV video surface currently requires a Win32 GPUI window"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl SurfaceBounds {
    fn validate(self) -> anyhow::Result<Self> {
        if self.width <= 0 || self.height <= 0 {
            bail!("surface dimensions must be positive");
        }
        Ok(self)
    }
}

/// Owns a child HWND. Create, resize, and drop it on its parent window's UI thread.
pub struct WindowsVideoSurface {
    hwnd: HWND,
}

impl WindowsVideoSurface {
    pub fn create(parent: usize, bounds: SurfaceBounds) -> anyhow::Result<Self> {
        let bounds = bounds.validate()?;
        if parent == 0 {
            bail!("parent HWND must be non-zero");
        }
        let parent = parent as HWND;
        // SAFETY: IsWindow only inspects the supplied native handle.
        if unsafe { IsWindow(parent) } == 0 {
            bail!("parent HWND is not a valid window");
        }
        const STATIC_CLASS: [u16; 7] = [83, 84, 65, 84, 73, 67, 0];
        // SAFETY: STATIC is a predefined Win32 class and parent was validated.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                STATIC_CLASS.as_ptr(),
                ptr::null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                parent,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            // SAFETY: GetLastError has no pointer requirements.
            let error = unsafe { GetLastError() };
            return Err(std::io::Error::from_raw_os_error(error as i32).into());
        }
        Ok(Self { hwnd })
    }

    /// Return the Win32 HWND value required by MPV's wid option.
    pub fn mpv_window_id(&self) -> anyhow::Result<u32> {
        let id = u32::try_from(self.hwnd as usize)?;
        if id == 0 {
            bail!("child HWND unexpectedly resolved to zero");
        }
        Ok(id)
    }

    pub fn set_bounds(&self, bounds: SurfaceBounds) -> anyhow::Result<()> {
        let bounds = bounds.validate()?;
        // SAFETY: this object owns hwnd until Drop.
        let ok = unsafe {
            SetWindowPos(
                self.hwnd,
                ptr::null_mut(),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
        if ok == 0 {
            bail!("SetWindowPos failed (Win32 error {})", unsafe {
                GetLastError()
            });
        }
        Ok(())
    }
}

impl Drop for WindowsVideoSurface {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            // SAFETY: this object uniquely owns the child HWND.
            unsafe {
                DestroyWindow(self.hwnd);
            }
            self.hwnd = ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_reject_non_positive_dimensions() {
        assert!(
            SurfaceBounds {
                x: 0,
                y: 0,
                width: 0,
                height: 10
            }
            .validate()
            .is_err()
        );
        assert!(
            SurfaceBounds {
                x: 0,
                y: 0,
                width: 10,
                height: -1
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn bounds_accept_positive_dimensions() {
        let bounds = SurfaceBounds {
            x: -5,
            y: 8,
            width: 640,
            height: 360,
        };
        assert_eq!(bounds.validate().unwrap(), bounds);
    }
}
