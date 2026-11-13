//! Minimal native Win32 child-window owner for MPV video embedding.
use anyhow::{Context as _, bail};
use gpui::Window;
use raw_window_handle::RawWindowHandle;
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
    let handle = raw_window_handle::HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("GPUI did not expose a native window handle: {error:?}"))?
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

    /// Convert GPUI logical-pixel bounds to physical Win32 client pixels.
    ///
    /// GPUI reports layout geometry in logical pixels. Win32 child-window
    /// placement uses physical pixels, so the current window scale factor
    /// must be applied on every layout/DPI update.
    pub fn from_logical_pixels(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        scale_factor: f32,
    ) -> anyhow::Result<Self> {
        if ![x, y, width, height, scale_factor]
            .iter()
            .all(|value| value.is_finite())
        {
            bail!("video surface geometry and scale factor must be finite");
        }
        if width <= 0.0 || height <= 0.0 || scale_factor <= 0.0 {
            bail!("video surface dimensions and scale factor must be positive");
        }

        fn physical(value: f32, scale: f32) -> anyhow::Result<i32> {
            let scaled = (f64::from(value) * f64::from(scale)).round();
            if scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
                bail!("video surface coordinate exceeds Win32 limits");
            }
            Ok(scaled as i32)
        }

        let bounds = Self {
            x: physical(x, scale_factor)?,
            y: physical(y, scale_factor)?,
            width: physical(width, scale_factor)?,
            height: physical(height, scale_factor)?,
        };
        bounds.validate()
    }
}

/// Owns a child HWND. Create, resize, and drop it on its parent window's UI thread.
pub struct WindowsVideoSurface {
    hwnd: usize,
    bounds: SurfaceBounds,
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
        Ok(Self {
            hwnd: hwnd as usize,
            bounds,
        })
    }

    /// Return the Win32 HWND value required by MPV's wid option.
    pub fn mpv_window_id(&self) -> anyhow::Result<u32> {
        let id = u32::try_from(self.hwnd)?;
        if id == 0 {
            bail!("child HWND unexpectedly resolved to zero");
        }
        Ok(id)
    }

    pub fn set_bounds(&mut self, bounds: SurfaceBounds) -> anyhow::Result<()> {
        let bounds = bounds.validate()?;
        if self.bounds == bounds {
            return Ok(());
        }
        // SAFETY: this object owns hwnd until Drop.
        let ok = unsafe {
            SetWindowPos(
                self.hwnd as HWND,
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
        self.bounds = bounds;
        Ok(())
    }
}

impl Drop for WindowsVideoSurface {
    fn drop(&mut self) {
        if self.hwnd != 0 {
            // SAFETY: this object uniquely owns the child HWND.
            unsafe {
                DestroyWindow(self.hwnd as HWND);
            }
            self.hwnd = 0;
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

    #[test]
    fn logical_bounds_convert_at_100_percent_scale() {
        let bounds = SurfaceBounds::from_logical_pixels(12.0, 24.0, 640.0, 360.0, 1.0).unwrap();
        assert_eq!(
            bounds,
            SurfaceBounds {
                x: 12,
                y: 24,
                width: 640,
                height: 360,
            }
        );
    }

    #[test]
    fn logical_bounds_convert_at_150_percent_scale() {
        let bounds = SurfaceBounds::from_logical_pixels(10.0, 20.0, 640.0, 360.0, 1.5).unwrap();
        assert_eq!(
            bounds,
            SurfaceBounds {
                x: 15,
                y: 30,
                width: 960,
                height: 540,
            }
        );
    }

    #[test]
    fn logical_bounds_round_fractional_physical_pixels() {
        let bounds = SurfaceBounds::from_logical_pixels(1.0, 1.0, 10.3, 5.7, 1.25).unwrap();
        assert_eq!(
            bounds,
            SurfaceBounds {
                x: 1,
                y: 1,
                width: 13,
                height: 7,
            }
        );
    }

    #[test]
    fn logical_bounds_reject_invalid_geometry_and_scale() {
        assert!(SurfaceBounds::from_logical_pixels(0.0, 0.0, 10.0, 10.0, 0.0).is_err());
        assert!(SurfaceBounds::from_logical_pixels(0.0, 0.0, -1.0, 10.0, 1.0).is_err());
        assert!(SurfaceBounds::from_logical_pixels(f32::NAN, 0.0, 10.0, 10.0, 1.0).is_err());
        assert!(SurfaceBounds::from_logical_pixels(0.0, 0.0, f32::INFINITY, 10.0, 1.0).is_err());
    }

    #[test]
    fn logical_bounds_reject_coordinates_outside_win32_range() {
        assert!(SurfaceBounds::from_logical_pixels(i32::MAX as f32, 0.0, 10.0, 10.0, 2.0).is_err());
    }
}
