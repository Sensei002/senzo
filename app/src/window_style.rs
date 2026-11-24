#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_os = "windows")]
use slint::winit_030::winit;

#[cfg(target_os = "windows")]
static CAPTION_HWND: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_os = "windows")]
pub fn apply(window: &winit::window::Window, caption: slint::Color) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        tracing::warn!("failed to obtain the native window handle for caption styling");
        return false;
    };
    CAPTION_HWND.store(hwnd, Ordering::Relaxed);
    apply_caption(hwnd, caption)
}

#[cfg(not(target_os = "windows"))]
pub fn apply(_window: &slint::winit_030::winit::window::Window, _caption: slint::Color) -> bool {
    true
}

/// Re-apply the native caption colour after a live theme edit.
pub fn refresh_caption(caption: slint::Color) {
    #[cfg(target_os = "windows")]
    {
        let hwnd = CAPTION_HWND.load(Ordering::Relaxed);
        if hwnd != 0 {
            let _ = apply_caption(hwnd, caption);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = caption;
    }
}

/// The native `HWND` as a `usize`, for handing to OS integrations (media
/// controls) that need it. `None` where the platform has no such handle, so the
/// caller stays platform-agnostic.
#[cfg(target_os = "windows")]
pub fn window_hwnd(window: &winit::window::Window) -> Option<usize> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get() as usize)
}

#[cfg(not(target_os = "windows"))]
pub fn window_hwnd(_window: &slint::winit_030::winit::window::Window) -> Option<usize> {
    None
}

#[cfg(target_os = "windows")]
fn apply_caption(hwnd: usize, caption: slint::Color) -> bool {
    use std::{ffi::c_void, mem::size_of};

    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DwmSetWindowAttribute,
    };

    let red = caption.red() as f32 / 255.0;
    let green = caption.green() as f32 / 255.0;
    let blue = caption.blue() as f32 / 255.0;
    let luminance = red * 0.299 + green * 0.587 + blue * 0.114;
    let light_caption = luminance > 0.6;
    let caption_color = colorref(
        caption.red() as u32,
        caption.green() as u32,
        caption.blue() as u32,
    );
    let text_color = if light_caption {
        colorref(0x00, 0x00, 0x00)
    } else {
        colorref(0xff, 0xff, 0xff)
    };
    let dark_caption: i32 = if light_caption { 0 } else { 1 };
    let hwnd = hwnd as *mut c_void;

    // SAFETY: `hwnd` comes from the live winit window and every attribute
    // points to a correctly sized value for the duration of its call.
    let (caption_result, border_result, text_result, dark_result) = unsafe {
        (
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR as u32,
                &caption_color as *const u32 as *const c_void,
                size_of::<u32>() as u32,
            ),
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR as u32,
                &caption_color as *const u32 as *const c_void,
                size_of::<u32>() as u32,
            ),
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_TEXT_COLOR as u32,
                &text_color as *const u32 as *const c_void,
                size_of::<u32>() as u32,
            ),
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &dark_caption as *const i32 as *const c_void,
                size_of::<i32>() as u32,
            ),
        )
    };

    if caption_result < 0 {
        tracing::warn!(
            caption_result,
            "Windows rejected the title-bar caption color"
        );
        return false;
    }
    if border_result < 0 {
        tracing::debug!(border_result, "Windows rejected the title-bar border color");
    }
    if text_result < 0 {
        tracing::debug!(text_result, "Windows rejected the title-bar text color");
    }
    if dark_result < 0 {
        tracing::debug!(
            dark_result,
            "Windows rejected the immersive dark title-bar mode"
        );
    }
    tracing::info!(?caption, "title-bar colors applied");
    true
}

#[cfg(target_os = "windows")]
const fn colorref(red: u32, green: u32, blue: u32) -> u32 {
    red | (green << 8) | (blue << 16)
}

/// Stable identity used by the taskbar and the System Media Transport Controls.
/// The installer's Start Menu shortcut must carry the same id for Windows to
/// resolve the friendly name and icon shown in the media overlay.
#[cfg(target_os = "windows")]
const APP_USER_MODEL_ID: &str = "com.senzo.desktop";

/// Give the process an explicit AppUserModelID. Windows keys the media overlay's
/// app identity off this, not the executable's version info, so without it the
/// System Media Transport Controls show "Unknown". Must run before the window
/// (and therefore the media controls) is created.
#[cfg(target_os = "windows")]
pub fn set_app_user_model_id() {
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

    let app_id: Vec<u16> = APP_USER_MODEL_ID
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `app_id` is a NUL-terminated UTF-16 string that outlives the call.
    let result = unsafe { SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr()) };
    if result < 0 {
        tracing::warn!(result, "failed to set AppUserModelID for media controls");
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_app_user_model_id() {}
