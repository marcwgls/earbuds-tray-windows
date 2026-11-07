#[cfg(windows)]
pub fn tray_icon_size() -> u32 {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;

    let dpi = unsafe { GetDpiForSystem() };
    if dpi == 0 {
        return 16;
    }
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) };
    if size <= 0 {
        16
    } else {
        (size as u32).clamp(16, 64)
    }
}

#[cfg(not(windows))]
pub fn tray_icon_size() -> u32 {
    16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tray_icon_size_reasonable_range() {
        let size = tray_icon_size();
        assert!(
            (16..=64).contains(&size),
            "DPI tray icon size out of bounds: {}",
            size
        );
    }
}
