//! Host configuration handed to `init` (api.md §1) and host conditions from
//! `sys.hostState`.
use super::error::ApiError;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One storage volume the host reports (`StorageVolume.getDirectory()`).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VolumeInfo {
    pub path: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub removable: bool,
}

/// The `init` configuration of the first `init` call. `noBackupDir` and
/// `versionCode` are accepted but not needed by the core.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostSettings {
    pub files_dir: PathBuf,
    pub cache_dir: PathBuf,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub home_dir: Option<PathBuf>,
    #[serde(default)]
    pub boot_marker: String,
    #[serde(default)]
    pub update_feed_url: Option<String>,
    #[serde(default = "default_true")]
    pub start_daemon: bool,
    #[serde(default)]
    pub volumes: Vec<VolumeInfo>,
}

fn default_true() -> bool {
    true
}

impl HostSettings {
    pub(crate) fn parse(text: &str) -> Result<Self, ApiError> {
        let settings: HostSettings = serde_json::from_str(text)
            .map_err(|error| ApiError::invalid(format!("Init-Konfiguration: {error}")))?;
        let home = settings.home_dir.as_ref().map(|home| ("homeDir", home));
        let paths = [
            ("filesDir", &settings.files_dir),
            ("cacheDir", &settings.cache_dir),
        ];
        for (name, path) in paths.iter().copied().chain(home) {
            if !path.is_absolute() || path.to_string_lossy().contains('\0') {
                return Err(ApiError::invalid(format!(
                    "Init-Konfiguration: {name} muss ein absoluter Pfad sein"
                )));
            }
        }
        for volume in &settings.volumes {
            validate_volume(volume)?;
        }
        Ok(settings)
    }

    /// `<filesDir>/smart_explorer`, the core's data directory.
    pub(crate) fn data_dir(&self) -> PathBuf {
        self.files_dir.join("smart_explorer")
    }

    /// `<data>/mobile`, the facade's own files.
    pub(crate) fn mobile_dir(&self) -> PathBuf {
        self.data_dir().join("mobile")
    }

    /// One of the declared cache subfolders (`open`, `share`, `update`, `tmp`).
    pub(crate) fn cache_subdir(&self, name: &str) -> PathBuf {
        self.cache_dir.join(name)
    }

    /// The default home location: `homeDir`, else the primary volume. It is
    /// the default Share export, so without either it is an empty folder of
    /// its own, never `filesDir` (credentials, tokens and keys live there).
    pub(crate) fn home(&self) -> PathBuf {
        self.home_dir
            .clone()
            .or_else(|| {
                self.volumes
                    .iter()
                    .find(|volume| volume.primary)
                    .or(self.volumes.first())
                    .map(|volume| PathBuf::from(&volume.path))
            })
            .unwrap_or_else(|| self.files_dir.join("home"))
    }
}

pub(crate) fn validate_volume(volume: &VolumeInfo) -> Result<(), ApiError> {
    if !Path::new(&volume.path).is_absolute() || volume.path.contains('\0') {
        return Err(ApiError::invalid(format!(
            "Ungültiger Speicherort: {}",
            volume.path
        )));
    }
    Ok(())
}
