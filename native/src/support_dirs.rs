use std::path::PathBuf;
use std::sync::OnceLock;

pub(crate) fn ensure_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    crate::creds::private_storage::ensure_directory(path)
}

pub(crate) fn create_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent() { ensure_private_dir(parent)?; }
    crate::creds::private_storage::create_file(path)
}

pub(crate) fn open_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent() { ensure_private_dir(parent)?; }
    crate::creds::private_storage::open_file(path, false)
}

pub(crate) fn open_private_lock(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    match create_private_file(path) {
        Ok(file) => {
            drop(file);
            crate::creds::private_storage::open_file(path, true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::creds::private_storage::open_file(path, true)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn secure_private_file(file: &std::fs::File) -> std::io::Result<()> {
    crate::creds::private_storage::secure(file, false)
}

pub(crate) fn read_private_text(path: &std::path::Path, max_bytes: u64) -> std::io::Result<String> {
    use std::io::Read;
    let file = open_private_file(path)?;
    if file.metadata()?.len() > max_bytes {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
            "private application file exceeds its byte limit"));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
            "private application file exceeds its byte limit"));
    }
    String::from_utf8(bytes).map_err(|_| std::io::Error::new(
        std::io::ErrorKind::InvalidData, "private application file is not UTF-8"))
}

/// Atomic, private, durable application metadata. An existing unsafe target
/// is rejected before staging; failed promotion preserves its contents.
pub(crate) fn write_private_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| std::io::Error::new(
        std::io::ErrorKind::InvalidInput, "private file needs a parent"))?;
    ensure_private_dir(parent)?;
    match open_private_file(path) {
        Ok(_) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|error| std::io::Error::other(error.to_string()))?;
    let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let stage = parent.join(format!(".se-private-{nonce}.tmp"));
    let result = (|| {
        let mut file = create_private_file(&stage)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        crate::vfs::replace_local_file(&stage, path)?;
        crate::creds::private_storage::sync_directory(parent)
    })();
    if result.is_err() { let _ = std::fs::remove_file(&stage); }
    result
}

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

/// An actual host/OS home fact; migration must never invent a temporary home.
pub(crate) fn home_dir() -> Option<PathBuf> {
    if let Some(host) = host() { return Some(host.home_dir.clone()); }
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.filter(|path| !path.is_empty()).map(PathBuf::from)
}

/// Root for temporary files: `<cache_dir>/tmp` (created on demand) once the
/// host values are set, otherwise the process temp directory.
pub fn temp_dir() -> PathBuf {
    let dir = temp_dir_for(host());
    if host().is_some() {
        let _ = ensure_private_dir(&dir);
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
    let _ = ensure_private_dir(&dir);
    dir
}

pub(crate) fn app_data_file(name: &str) -> PathBuf {
    app_data_dir().join(name)
}

pub(crate) fn sync_data_dir() -> PathBuf {
    let dir = app_data_dir().join("sync");
    let _ = ensure_private_dir(&dir);
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
