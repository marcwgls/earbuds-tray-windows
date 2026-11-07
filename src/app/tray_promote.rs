#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayEntry {
    pub key: String,
    pub exe_path: String,
    pub is_promoted: Option<u32>,
}

pub fn expand_known_folder(path: &str, resolve: impl Fn(&str) -> Option<String>) -> String {
    if path.starts_with('{')
        && let Some(close_idx) = path.find('}')
    {
        let rest = &path[close_idx + 1..];
        if rest.starts_with('\\') || rest.starts_with('/') {
            let guid = &path[1..close_idx];
            if let Some(folder) = resolve(guid) {
                let folder_trimmed = folder.trim_end_matches(['\\', '/']);
                return format!("{}{}", folder_trimmed, rest);
            }
        }
    }
    path.to_string()
}

fn normalize_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

pub fn entries_to_promote(
    entries: &[TrayEntry],
    own_exe: &str,
    resolve: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let norm_own = normalize_path(own_exe);
    entries
        .iter()
        .filter(|entry| {
            entry.is_promoted.is_none() && {
                let expanded = expand_known_folder(&entry.exe_path, &resolve);
                normalize_path(&expanded) == norm_own
            }
        })
        .map(|entry| entry.key.clone())
        .collect()
}

#[cfg(windows)]
struct SafeHKey(windows::Win32::System::Registry::HKEY);

#[cfg(windows)]
impl Drop for SafeHKey {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = windows::Win32::System::Registry::RegCloseKey(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn resolve_known_folder(guid: &str) -> Option<String> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Com::{CLSIDFromString, CoTaskMemFree};
    use windows::Win32::UI::Shell::{KNOWN_FOLDER_FLAG, SHGetKnownFolderPath};
    use windows::core::PCWSTR;

    let with_braces = format!("{{{}}}", guid.trim_matches(['{', '}']));
    let wide: Vec<u16> = with_braces
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let clsid = match unsafe { CLSIDFromString(PCWSTR::from_raw(wide.as_ptr())) } {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("CLSIDFromString failed for '{guid}': {e}");
            return None;
        }
    };

    let pwstr =
        match unsafe { SHGetKnownFolderPath(&clsid, KNOWN_FOLDER_FLAG(0), HANDLE::default()) } {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("SHGetKnownFolderPath failed for '{guid}': {e}");
                return None;
            }
        };

    let s = unsafe { pwstr.to_string() }.ok();
    unsafe {
        CoTaskMemFree(Some(pwstr.0 as *const _));
    }
    s
}

#[cfg(windows)]
fn read_entries() -> Vec<TrayEntry> {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, REG_SZ, RegEnumKeyExW, RegOpenKeyExW,
        RegQueryValueExW,
    };
    use windows::core::{PCWSTR, PWSTR};

    let mut entries = Vec::new();
    let root_path = windows::core::w!("Control Panel\\NotifyIconSettings");
    let mut root_hkey = HKEY::default();
    let status =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, root_path, 0, KEY_READ, &mut root_hkey) };
    if status != ERROR_SUCCESS {
        tracing::warn!("read_entries: failed to open NotifyIconSettings: {status:?}");
        return entries;
    }
    let root_key = SafeHKey(root_hkey);

    let mut index = 0u32;
    loop {
        let mut name_buf = [0u16; 256];
        let mut name_len = name_buf.len() as u32;
        let enum_status = unsafe {
            RegEnumKeyExW(
                root_key.0,
                index,
                PWSTR(name_buf.as_mut_ptr()),
                &mut name_len,
                None,
                PWSTR::null(),
                None,
                None,
            )
        };

        if enum_status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if enum_status != ERROR_SUCCESS {
            tracing::warn!("read_entries: RegEnumKeyExW error at index {index}: {enum_status:?}");
            break;
        }

        let subkey_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
        index += 1;

        let subkey_path = format!("Control Panel\\NotifyIconSettings\\{}", subkey_name);
        let subkey_wide: Vec<u16> = subkey_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut sub_hkey = HKEY::default();
        let open_status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR::from_raw(subkey_wide.as_ptr()),
                0,
                KEY_READ,
                &mut sub_hkey,
            )
        };
        if open_status != ERROR_SUCCESS {
            tracing::debug!("read_entries: failed to open subkey '{subkey_name}': {open_status:?}");
            continue;
        }
        let sub_key = SafeHKey(sub_hkey);

        let exe_val_name = windows::core::w!("ExecutablePath");
        let mut val_type = REG_SZ;
        let mut data_len = 0u32;
        let query_status = unsafe {
            RegQueryValueExW(
                sub_key.0,
                exe_val_name,
                None,
                Some(&mut val_type),
                None,
                Some(&mut data_len),
            )
        };
        if query_status != ERROR_SUCCESS || val_type != REG_SZ || data_len == 0 {
            continue;
        }

        let mut byte_buf = vec![0u8; data_len as usize];
        let query_data_status = unsafe {
            RegQueryValueExW(
                sub_key.0,
                exe_val_name,
                None,
                Some(&mut val_type),
                Some(byte_buf.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if query_data_status != ERROR_SUCCESS {
            continue;
        }

        let u16_slice: &[u16] = unsafe {
            std::slice::from_raw_parts(byte_buf.as_ptr() as *const u16, (data_len / 2) as usize)
        };
        let exe_path = String::from_utf16_lossy(u16_slice.split(|&c| c == 0).next().unwrap_or(&[]));
        if exe_path.is_empty() {
            continue;
        }

        let prom_val_name = windows::core::w!("IsPromoted");
        let mut prom_type = REG_DWORD;
        let mut dword_val = 0u32;
        let mut dword_len = std::mem::size_of::<u32>() as u32;
        let prom_status = unsafe {
            RegQueryValueExW(
                sub_key.0,
                prom_val_name,
                None,
                Some(&mut prom_type),
                Some(&mut dword_val as *mut u32 as *mut u8),
                Some(&mut dword_len),
            )
        };

        // Only a really missing value counts as None. Any other error or type
        // skips the entry, so we never overwrite something we could not read.
        let is_promoted = if prom_status == ERROR_SUCCESS && prom_type == REG_DWORD {
            Some(dword_val)
        } else if prom_status == ERROR_FILE_NOT_FOUND {
            None
        } else {
            tracing::debug!(
                "read_entries: unreadable IsPromoted in '{subkey_name}': {prom_status:?}"
            );
            continue;
        };

        entries.push(TrayEntry {
            key: subkey_name,
            exe_path,
            is_promoted,
        });
    }

    entries
}

#[cfg(windows)]
fn promote(key: &str) -> windows::core::Result<()> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, RegOpenKeyExW, RegSetValueExW,
    };
    use windows::core::PCWSTR;

    let subkey_path = format!("Control Panel\\NotifyIconSettings\\{}", key);
    let subkey_wide: Vec<u16> = subkey_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut hkey = HKEY::default();
    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(subkey_wide.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        )
    };
    if open_status != ERROR_SUCCESS {
        return Err(windows::core::Error::from(open_status));
    }
    let safe_key = SafeHKey(hkey);

    let val: u32 = 1;
    let val_bytes = val.to_ne_bytes();
    let set_status = unsafe {
        RegSetValueExW(
            safe_key.0,
            windows::core::w!("IsPromoted"),
            0,
            REG_DWORD,
            Some(&val_bytes),
        )
    };
    if set_status != ERROR_SUCCESS {
        return Err(windows::core::Error::from(set_status));
    }

    Ok(())
}

#[cfg(windows)]
pub fn promote_own_icons() {
    let own_exe = match std::env::current_exe() {
        Ok(path) => path.to_string_lossy().to_string(),
        Err(e) => {
            tracing::warn!("promote_own_icons: failed to get current_exe: {e}");
            return;
        }
    };

    let entries = read_entries();
    let to_promote = entries_to_promote(&entries, &own_exe, resolve_known_folder);

    if to_promote.is_empty() {
        tracing::debug!("promote_own_icons: no unpromoted entries found for '{own_exe}'");
        return;
    }

    for key in to_promote {
        match promote(&key) {
            Ok(()) => {
                tracing::info!("promote_own_icons: successfully promoted key '{key}'");
            }
            Err(e) => {
                tracing::warn!("promote_own_icons: failed to promote key '{key}': {e}");
            }
        }
    }
}

#[cfg(not(windows))]
pub fn promote_own_icons() {}

#[cfg(windows)]
pub fn watch_and_promote() {
    use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
        RegNotifyChangeKeyValue, RegOpenKeyExW,
    };

    promote_own_icons();

    let root_path = windows::core::w!("Control Panel\\NotifyIconSettings");
    let mut root_hkey = HKEY::default();
    let open_status =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, root_path, 0, KEY_READ, &mut root_hkey) };
    if open_status != ERROR_SUCCESS {
        tracing::warn!(
            "watch_and_promote: failed to open NotifyIconSettings for watching: {open_status:?}"
        );
        return;
    }
    let root_key = SafeHKey(root_hkey);

    loop {
        let notify_status = unsafe {
            RegNotifyChangeKeyValue(
                root_key.0,
                true,
                REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                HANDLE::default(),
                false,
            )
        };
        if notify_status != ERROR_SUCCESS {
            tracing::warn!("watch_and_promote: RegNotifyChangeKeyValue failed: {notify_status:?}");
            break;
        }
        promote_own_icons();
    }
}

#[cfg(not(windows))]
pub fn watch_and_promote() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_resolve(_guid: &str) -> Option<String> {
        None
    }

    #[test]
    fn test_plain_path_our_exe_missing_value_promoted() {
        let entries = vec![TrayEntry {
            key: "entry1".to_string(),
            exe_path: r"C:\app\earbuds-tray.exe".to_string(),
            is_promoted: None,
        }];
        let to_promote = entries_to_promote(&entries, r"C:\app\earbuds-tray.exe", dummy_resolve);
        assert_eq!(to_promote, vec!["entry1"]);
    }

    #[test]
    fn test_our_exe_with_explicit_value_not_promoted() {
        let entries = vec![
            TrayEntry {
                key: "entry_zero".to_string(),
                exe_path: r"C:\app\earbuds-tray.exe".to_string(),
                is_promoted: Some(0),
            },
            TrayEntry {
                key: "entry_one".to_string(),
                exe_path: r"C:\app\earbuds-tray.exe".to_string(),
                is_promoted: Some(1),
            },
        ];
        let to_promote = entries_to_promote(&entries, r"C:\app\earbuds-tray.exe", dummy_resolve);
        assert!(to_promote.is_empty());
    }

    #[test]
    fn test_another_exe_not_promoted() {
        let entries = vec![TrayEntry {
            key: "foreign_entry".to_string(),
            exe_path: r"C:\Program Files\OtherApp\other.exe".to_string(),
            is_promoted: None,
        }];
        let to_promote = entries_to_promote(&entries, r"C:\app\earbuds-tray.exe", dummy_resolve);
        assert!(to_promote.is_empty());
    }

    #[test]
    fn test_case_difference_and_slash_difference_still_matches() {
        let entries = vec![TrayEntry {
            key: "entry_mismatch_format".to_string(),
            exe_path: "c:/app/EARBUDS-TRAY.EXE".to_string(),
            is_promoted: None,
        }];
        let to_promote = entries_to_promote(&entries, r"C:\APP\earbuds-tray.exe", dummy_resolve);
        assert_eq!(to_promote, vec!["entry_mismatch_format"]);
    }

    #[test]
    fn test_known_folder_resolves_to_our_exe_promoted_unknown_guid_ignored() {
        let fake_resolve = |guid: &str| match guid {
            "6D809377-6AF0-444B-8957-A3773F02200E" => Some(r"C:\Program Files".to_string()),
            _ => None,
        };

        let entries = vec![
            TrayEntry {
                key: "known_resolved".to_string(),
                exe_path: r"{6D809377-6AF0-444B-8957-A3773F02200E}\earbuds-tray\earbuds-tray.exe"
                    .to_string(),
                is_promoted: None,
            },
            TrayEntry {
                key: "unknown_guid".to_string(),
                exe_path: r"{00000000-0000-0000-0000-000000000000}\earbuds-tray\earbuds-tray.exe"
                    .to_string(),
                is_promoted: None,
            },
        ];

        let to_promote = entries_to_promote(
            &entries,
            r"C:\Program Files\earbuds-tray\earbuds-tray.exe",
            fake_resolve,
        );
        assert_eq!(to_promote, vec!["known_resolved"]);
    }

    #[test]
    fn test_three_entries_of_ours_plus_foreign_ones() {
        let own = r"C:\app\earbuds-tray.exe";
        let entries = vec![
            TrayEntry {
                key: "ours_missing_1".to_string(),
                exe_path: own.to_string(),
                is_promoted: None,
            },
            TrayEntry {
                key: "ours_missing_2".to_string(),
                exe_path: own.to_string(),
                is_promoted: None,
            },
            TrayEntry {
                key: "ours_disabled".to_string(),
                exe_path: own.to_string(),
                is_promoted: Some(0),
            },
            TrayEntry {
                key: "foreign_missing".to_string(),
                exe_path: r"C:\other\tool.exe".to_string(),
                is_promoted: None,
            },
            TrayEntry {
                key: "foreign_enabled".to_string(),
                exe_path: r"C:\other\tool.exe".to_string(),
                is_promoted: Some(1),
            },
        ];

        let to_promote = entries_to_promote(&entries, own, dummy_resolve);
        assert_eq!(to_promote, vec!["ours_missing_1", "ours_missing_2"]);
    }

    #[test]
    fn test_expand_known_folder_malformed_no_closing_brace_or_slash() {
        assert_eq!(expand_known_folder("{unclosed", dummy_resolve), "{unclosed");
        assert_eq!(expand_known_folder("{abc}def", dummy_resolve), "{abc}def");
        assert_eq!(expand_known_folder("", dummy_resolve), "");
    }

    #[cfg(windows)]
    #[test]
    fn test_resolve_known_folder_program_files() {
        let res = resolve_known_folder("6D809377-6AF0-444B-8957-A3773F02200E");
        assert!(res.is_some(), "Expected ProgramFiles to resolve");
        let path = res.unwrap();
        assert!(
            path.to_ascii_lowercase().contains("program files"),
            "Expected 'Program Files' in resolved path, got: {path}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_resolve_known_folder_invalid_guid() {
        let res = resolve_known_folder("not-a-valid-guid");
        assert!(res.is_none());
        let res_zeros = resolve_known_folder("00000000-0000-0000-0000-000000000000");
        assert!(res_zeros.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn test_read_entries_does_not_panic() {
        // May be empty on a fresh machine or in CI, so only check the shape.
        let entries = read_entries();
        for entry in &entries {
            assert!(!entry.key.is_empty());
            assert!(!entry.exe_path.is_empty());
        }
    }
}
