use std::path::{Path, PathBuf};

pub(super) fn appdata_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
}

fn override_path() -> PathBuf {
    appdata_dir().join("update_source.txt")
}

pub(super) fn last_applied_path() -> PathBuf {
    appdata_dir().join("last_applied_update.txt")
}

pub(super) fn updater_error_path() -> PathBuf {
    appdata_dir().join("last_updater_error.txt")
}

pub fn take_updater_error() -> Option<String> {
    let p = updater_error_path();
    let raw = std::fs::read_to_string(&p).ok()?;
    let _ = std::fs::remove_file(&p);
    let msg = raw.trim().to_string();
    if msg.is_empty() {
        None
    } else {
        Some(msg)
    }
}

/// The raw configured update source string (folder path OR http(s) URL),
/// first hit wins. Used by the UI text field and the transport classifier.
pub fn update_source_str() -> Option<String> {
    let read = |p: &Path| -> Option<String> {
        let s = std::fs::read_to_string(p).ok()?;
        let line = s.lines().next()?.trim().to_string();
        if line.is_empty() {
            None
        } else {
            Some(line)
        }
    };
    if let Some(s) = read(&override_path()) {
        return Some(s);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(s) = read(&dir.join("update_source.txt")) {
                return Some(s);
            }
        }
    }
    None
}

/// Persist a user-chosen feed folder (empty string removes the override).
pub fn set_update_source(path: &str) -> std::io::Result<()> {
    let path = path.trim();
    if path.is_empty() {
        let _ = std::fs::remove_file(override_path());
        Ok(())
    } else {
        std::fs::write(override_path(), path)
    }
}
