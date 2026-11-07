pub const RUN_VALUE_NAME: &str = "EarbudsTray";

/// The exact string stored in the Run value: the exe path in double quotes.
pub fn run_command(exe: &str) -> String {
    format!("\"{exe}\"")
}

fn normalize_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

/// True if the Run value starts this exe. Accepts the path with or without
/// quotes, compares case-insensitively and with `/` treated as `\`.
/// Arguments after the closing quote are ignored.
pub fn is_enabled_for(value: Option<&str>, exe: &str) -> bool {
    let Some(raw_val) = value else {
        return false;
    };

    let trimmed = raw_val.trim();
    let val_path = if let Some(stripped) = trimmed.strip_prefix('"') {
        if let Some(end_quote) = stripped.find('"') {
            &stripped[..end_quote]
        } else {
            stripped
        }
    } else {
        trimmed
    };

    normalize_path(val_path) == normalize_path(exe.trim())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey,
        RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows::core::{PCWSTR, w};

    const RUN_KEY_PATH: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const RUN_VAL_NAME: PCWSTR = w!("EarbudsTray");

    struct SafeKey(HKEY);
    impl Drop for SafeKey {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = RegCloseKey(self.0);
                }
            }
        }
    }

    pub fn read_run_value() -> Option<String> {
        let mut hkey = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                RUN_KEY_PATH,
                0,
                KEY_QUERY_VALUE,
                &mut hkey,
            )
        };
        if status != ERROR_SUCCESS {
            tracing::debug!("read_run_value: failed to open Run key: {status:?}");
            return None;
        }
        let safe_key = SafeKey(hkey);

        let mut val_type = REG_SZ;
        let mut data_len = 0u32;
        let status = unsafe {
            RegQueryValueExW(
                safe_key.0,
                RUN_VAL_NAME,
                None,
                Some(&mut val_type),
                None,
                Some(&mut data_len),
            )
        };
        if status != ERROR_SUCCESS || val_type != REG_SZ || data_len == 0 {
            return None;
        }

        // u16 buffer so the UTF-16 data is correctly aligned.
        let mut buf = vec![0u16; (data_len as usize).div_ceil(2)];
        let status = unsafe {
            RegQueryValueExW(
                safe_key.0,
                RUN_VAL_NAME,
                None,
                Some(&mut val_type),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut data_len),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }

        let u16_slice = &buf[..(data_len as usize / 2).min(buf.len())];
        let val = String::from_utf16_lossy(u16_slice.split(|&c| c == 0).next().unwrap_or(&[]));
        Some(val)
    }

    pub fn enable(exe: &str) -> windows::core::Result<()> {
        let mut hkey = HKEY::default();
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY_PATH, 0, KEY_SET_VALUE, &mut hkey) };
        if status != ERROR_SUCCESS {
            return Err(windows::core::Error::from(status));
        }
        let safe_key = SafeKey(hkey);

        let cmd = run_command(exe);
        let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                wide.as_ptr() as *const u8,
                wide.len() * std::mem::size_of::<u16>(),
            )
        };

        let status = unsafe { RegSetValueExW(safe_key.0, RUN_VAL_NAME, 0, REG_SZ, Some(bytes)) };
        if status != ERROR_SUCCESS {
            return Err(windows::core::Error::from(status));
        }
        Ok(())
    }

    pub fn disable() -> windows::core::Result<()> {
        let mut hkey = HKEY::default();
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY_PATH, 0, KEY_SET_VALUE, &mut hkey) };
        if status != ERROR_SUCCESS {
            return Err(windows::core::Error::from(status));
        }
        let safe_key = SafeKey(hkey);

        let status = unsafe { RegDeleteValueW(safe_key.0, RUN_VAL_NAME) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(windows::core::Error::from(status))
        }
    }
}

#[cfg(windows)]
pub fn read_run_value() -> Option<String> {
    imp::read_run_value()
}

#[cfg(windows)]
pub fn enable(exe: &str) -> windows::core::Result<()> {
    imp::enable(exe)
}

#[cfg(windows)]
pub fn disable() -> windows::core::Result<()> {
    imp::disable()
}

#[cfg(not(windows))]
pub fn read_run_value() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn enable(_exe: &str) -> windows::core::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn disable() -> windows::core::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_command_quoted() {
        assert_eq!(
            run_command(r"C:\Users\Marc\AppData\Local\Programs\Earbuds Tray\earbuds-tray.exe"),
            "\"C:\\Users\\Marc\\AppData\\Local\\Programs\\Earbuds Tray\\earbuds-tray.exe\""
        );
    }

    #[test]
    fn test_is_enabled_for_quoted_match() {
        let exe = r"C:\Programs\Earbuds Tray\earbuds-tray.exe";
        let val = Some("\"C:\\Programs\\Earbuds Tray\\earbuds-tray.exe\"");
        assert!(is_enabled_for(val, exe));
    }

    #[test]
    fn test_is_enabled_for_unquoted_match() {
        let exe = r"C:\Programs\Earbuds Tray\earbuds-tray.exe";
        let val = Some(r"C:\Programs\Earbuds Tray\earbuds-tray.exe");
        assert!(is_enabled_for(val, exe));
    }

    #[test]
    fn test_is_enabled_for_different_case() {
        let exe = r"C:\Programs\Earbuds Tray\earbuds-tray.exe";
        let val = Some("\"c:\\programs\\EARBUDS TRAY\\EARBUDS-TRAY.EXE\"");
        assert!(is_enabled_for(val, exe));
    }

    #[test]
    fn test_is_enabled_for_slash_differences() {
        let exe = "C:/Programs/Earbuds Tray/earbuds-tray.exe";
        let val = Some("\"C:\\Programs\\Earbuds Tray\\earbuds-tray.exe\"");
        assert!(is_enabled_for(val, exe));

        let exe2 = r"C:\Programs\Earbuds Tray\earbuds-tray.exe";
        let val2 = Some("\"C:/Programs/Earbuds Tray/earbuds-tray.exe\"");
        assert!(is_enabled_for(val2, exe2));
    }

    #[test]
    fn test_is_enabled_for_trailing_arguments_ignored() {
        let exe = r"C:\Programs\Earbuds Tray\earbuds-tray.exe";
        let val = Some("\"C:\\Programs\\Earbuds Tray\\earbuds-tray.exe\" --arg1 --arg2");
        assert!(is_enabled_for(val, exe));
    }

    #[test]
    fn test_is_enabled_for_other_exe_is_false() {
        let exe = r"C:\Programs\Earbuds Tray\earbuds-tray.exe";
        let val = Some("\"C:\\Other\\app.exe\"");
        assert!(!is_enabled_for(val, exe));

        let debug_exe = r"C:\earbuds-tray\target\debug\earbuds-tray.exe";
        let installed_val =
            Some("\"C:\\Users\\Marc\\AppData\\Local\\Programs\\Earbuds Tray\\earbuds-tray.exe\"");
        assert!(!is_enabled_for(installed_val, debug_exe));
    }

    #[test]
    fn test_is_enabled_for_none_is_false() {
        let exe = r"C:\Programs\Earbuds Tray\earbuds-tray.exe";
        assert!(!is_enabled_for(None, exe));
    }
}
