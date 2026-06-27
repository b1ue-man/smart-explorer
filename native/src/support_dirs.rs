use std::path::PathBuf;

fn data_home() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(dir);
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".local").join("share");
        }
        return std::env::temp_dir();
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        std::env::temp_dir()
    }
}

pub(crate) fn app_data_dir() -> PathBuf {
    let dir = data_home().join("smart_explorer");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub(crate) fn app_data_file(name: &str) -> PathBuf {
    app_data_dir().join(name)
}

pub(crate) fn sync_data_dir() -> PathBuf {
    let dir = app_data_dir().join("sync");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
