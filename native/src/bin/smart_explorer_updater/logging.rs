use std::path::{Path, PathBuf};

pub(crate) fn appdata_dir() -> PathBuf {
    #[cfg(windows)]
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(target_os = "linux")]
    let dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("share")
        });
    #[cfg(not(any(windows, target_os = "linux")))]
    let dir = std::env::temp_dir();
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app
}

pub(crate) fn default_error_file() -> PathBuf {
    appdata_dir().join("last_updater_error.txt")
}

pub(crate) fn record_failure(error_file: &Path, msg: &str) {
    if let Some(parent) = error_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(error_file, msg);
    append_log(&format!("error: {}", msg));
}

pub(crate) fn append_log(msg: &str) {
    let path = appdata_dir().join("updater.log");
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 512 * 1024 {
        let _ = std::fs::write(&path, "");
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}
