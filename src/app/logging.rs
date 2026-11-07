use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

pub const MAX_LOG_SIZE: u64 = 1_048_576; // 1 MiB

pub fn log_path(local_app_data: &Path) -> PathBuf {
    local_app_data.join("earbuds-tray").join("earbuds-tray.log")
}

pub fn needs_rotation(size: u64) -> bool {
    size > MAX_LOG_SIZE
}

pub fn open_log_file() -> Option<File> {
    let app_data = std::env::var_os("LOCALAPPDATA")?;
    let path = log_path(Path::new(&app_data));
    let dir = path.parent()?;
    fs::create_dir_all(dir).ok()?;

    if let Ok(metadata) = fs::metadata(&path)
        && needs_rotation(metadata.len())
    {
        let old_path = dir.join("earbuds-tray.log.old");
        let _ = fs::remove_file(&old_path);
        let _ = fs::rename(&path, &old_path);
    }

    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_path() {
        let base = Path::new(r"C:\Users\Marc\AppData\Local");
        let expected = PathBuf::from(r"C:\Users\Marc\AppData\Local\earbuds-tray\earbuds-tray.log");
        assert_eq!(log_path(base), expected);
    }

    #[test]
    fn test_needs_rotation_boundaries() {
        assert!(!needs_rotation(0));
        assert!(!needs_rotation(1_048_576), "Exactly 1 MiB must not rotate");
        assert!(needs_rotation(1_048_577), "1 MiB + 1 byte must rotate");
        assert!(needs_rotation(2_000_000));
    }
}
