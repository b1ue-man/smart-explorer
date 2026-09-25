use std::path::PathBuf;
use std::sync::OnceLock;

/// Directories and device facts an embedding host (the Android app) hands in,
/// because its process has no usable environment for them. The desktop builds
/// never set them and keep deriving everything from the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostConfig {
    /// Parent of the app data directory (`<data_home>/smart_explorer`).
    pub data_home: PathBuf,
    /// Host cache directory; temporary files live in `<cache_dir>/tmp`.
    pub cache_dir: PathBuf,
    /// Default home location (desktop: `$HOME`), e.g. the default Share export.
    pub home_dir: PathBuf,
    /// Device name offered to peers until the user picks another one.
    pub device_name: String,
    /// Opaque value that changes with every device boot.
    pub boot_marker: String,
}

const HOST_TEMP_SUBDIR: &str = "tmp";

static HOST: OnceLock<HostConfig> = OnceLock::new();

/// Installs the host values. Only the first call takes effect; later calls are
/// ignored so every thread keeps seeing one consistent set of directories.
pub fn set_host(config: HostConfig) {
    let _ = HOST.set(config);
}

/// The host values, once `set_host` ran.
pub fn host() -> Option<&'static HostConfig> {
    HOST.get()
}

/// Root for temporary files: `<cache_dir>/tmp` (created on demand) once the
/// host values are set, otherwise the process temp directory.
pub fn temp_dir() -> PathBuf {
    let dir = temp_dir_for(host());
    if host().is_some() {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir
}

fn temp_dir_for(host: Option<&HostConfig>) -> PathBuf {
    match host {
        Some(host) => host.cache_dir.join(HOST_TEMP_SUBDIR),
        None => std::env::temp_dir(),
    }
}

fn data_home() -> PathBuf {
    data_home_for(host())
}

fn data_home_for(host: Option<&HostConfig>) -> PathBuf {
    match host {
        Some(host) => host.data_home.clone(),
        None => platform_data_home(),
    }
}

fn platform_data_home() -> PathBuf {
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
        std::env::temp_dir()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_host() -> HostConfig {
        HostConfig {
            data_home: PathBuf::from("/data/user/0/app/files"),
            cache_dir: PathBuf::from("/data/user/0/app/cache"),
            home_dir: PathBuf::from("/storage/emulated/0"),
            device_name: "Telefon".into(),
            boot_marker: "17".into(),
        }
    }

    #[test]
    fn android_task_host_values_redirect_data_and_temp_roots() {
        let host = sample_host();
        assert_eq!(
            data_home_for(Some(&host)),
            PathBuf::from("/data/user/0/app/files")
        );
        assert_eq!(
            temp_dir_for(Some(&host)),
            PathBuf::from("/data/user/0/app/cache/tmp")
        );
    }

    #[test]
    fn android_task_without_host_values_desktop_roots_stay_unchanged() {
        assert_eq!(data_home_for(None), platform_data_home());
        assert_eq!(temp_dir_for(None), std::env::temp_dir());
    }
}
