//! Windows display capability queries used to gate HDR output options.
//!
//! This module only reads the active display configuration. It never changes
//! Windows HDR settings. If the API cannot establish support and enabled state,
//! the result is false so SENZO fails closed.

#[cfg(windows)]
pub fn hdr_output_enabled() -> bool {
    use std::mem::size_of;
    use windows_sys::Win32::Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
        DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO,
        DISPLAYCONFIG_PATH_INFO, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
        QueryDisplayConfig,
    };

    const QDC_ONLY_ACTIVE_PATHS: u32 = 0x0000_0002;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_INSUFFICIENT_BUFFER: i32 = 122;

    // Display topology can change between sizing and querying. Retry once on
    // ERROR_INSUFFICIENT_BUFFER and otherwise fail closed.
    for _ in 0..2 {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        let sized = unsafe {
            GetDisplayConfigBufferSizes(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                &mut mode_count,
            )
        };
        if sized != ERROR_SUCCESS || path_count == 0 || path_count > 64 || mode_count > 256 {
            return false;
        }

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let result = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if result == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if result != ERROR_SUCCESS || path_count == 0 {
            return false;
        }

        let mut found_active_display = false;
        for path in paths.iter().take(path_count as usize) {
            let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default();
            info.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
            info.header.size = size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32;
            info.header.adapterId = path.targetInfo.adapterId;
            info.header.id = path.targetInfo.id;

            let queried = unsafe { DisplayConfigGetDeviceInfo(&mut info.header) };
            if queried != ERROR_SUCCESS {
                return false;
            }

            // The first DWORD of the SDK's anonymous bitfield contains:
            // bit 0 = advancedColorSupported, bit 1 = advancedColorEnabled.
            let flags = unsafe {
                std::ptr::read_unaligned(std::ptr::addr_of!(info.Anonymous).cast::<u32>())
            };
            let supported = flags & 0x1 != 0;
            let enabled = flags & 0x2 != 0;
            found_active_display = true;

            // Conservative multi-monitor policy: only claim HDR passthrough
            // when every active display supports HDR and has it enabled.
            if !supported || !enabled {
                return false;
            }
        }

        return found_active_display;
    }

    false
}

#[cfg(not(windows))]
pub fn hdr_output_enabled() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn hdr_display_detection_fails_closed_on_non_windows_targets() {
        #[cfg(not(windows))]
        assert!(!super::hdr_output_enabled());
    }
}
