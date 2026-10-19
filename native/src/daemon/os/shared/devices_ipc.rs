//! The answer about the device tree „Verbundene Geräte als Ordner“ (DRV-M6,
//! contract V4) and the client side of its requests: `se drive devices`, the
//! settings of the app and se Desktop read the same object.
//!
//! `DevicesStatus` is exactly the V4 object (`enabled`, `state`, `mounted`,
//! `root_path`, `letter`, `mount_point`, `revision`, `devices`) plus
//! `pending`, the number of devices with changes that wait for a
//! connection. Absent values are not written.
use std::net::TcpStream;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::mount::{layout, DeviceKind, DeviceRow, MountSnapshot, MountStatus, PathDialect};

use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use super::super::ipc_storage::{read_ipc_addr, read_token};
use super::ipc_protocol_drives::DriveIpcRequest;

/// The status a drive reports while it waits for Smart Explorer Share
/// (`mount_restore.rs`, `DEFERRED_DETAIL`).
const WAITING_PREFIX: &str = "Wartet auf Smart Explorer Share";
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
const SWITCH_TIMEOUT: Duration =
    Duration::from_secs(super::super::mount_manager::STOP_GRACE.as_secs() + 30);
/// A watch answers after 30 s at the latest (V4).
const WATCH_TIMEOUT: Duration = Duration::from_secs(35);

/// One device in the answer. `name` is the name of its folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStatusRow {
    pub key: String,
    pub name: String,
    /// `direct` or `room`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    pub online: bool,
    /// The folder of the device while the tree is mounted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicesStatus {
    pub enabled: bool,
    /// `off`, `mounting`, `mounted`, `unmounting`, `waiting`, `failed` or
    /// `stopped`.
    pub state: String,
    pub mounted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub letter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_point: Option<String>,
    pub revision: u64,
    pub devices: Vec<DeviceStatusRow>,
    /// Devices with changes that wait for a connection.
    #[serde(default)]
    pub pending: u32,
}

/// Beside the registry: exists once the one-time hint about the device
/// tree was shown or the user switched the tree on himself.
const HINT_FILE: &str = "devices-hint-shown";

fn hint_path() -> std::path::PathBuf {
    crate::support_dirs::app_data_dir()
        .join("mounts")
        .join(HINT_FILE)
}

/// Whether the one-time hint „Verbundene Geräte erscheinen jetzt …“ has not
/// been shown yet (decision C5).
pub fn devices_hint_due() -> bool {
    !hint_path().exists()
}

/// Records that the hint was shown, or is not needed because the user
/// switched the tree on himself.
pub fn devices_hint_done() {
    let _ = crate::support_dirs::write_private_atomic(&hint_path(), b"");
}

/// The name rules of the host of this platform.
pub(super) fn host_dialect() -> PathDialect {
    if cfg!(windows) {
        PathDialect::Windows
    } else {
        PathDialect::Posix
    }
}

/// `state` and `mounted` of a drive status.
fn state_of(status: &MountStatus) -> (&'static str, bool) {
    match status {
        MountStatus::Unmounted => ("stopped", false),
        MountStatus::Mounting => ("mounting", false),
        MountStatus::Mounted { .. } | MountStatus::MountedAt { .. } => ("mounted", true),
        MountStatus::Unmounting => ("unmounting", false),
        MountStatus::Failed { detail } if detail.starts_with(WAITING_PREFIX) => ("waiting", false),
        MountStatus::Conflict { .. } => ("failed", true),
        MountStatus::Failed { .. } | MountStatus::RuntimeUnavailable { .. } => ("failed", false),
    }
}

/// Builds the answer from the drive entry (if any) and the devices. The
/// revision is set by the board that compares answers.
pub(super) fn build_status(
    entry: Option<&MountSnapshot>,
    rows: &[DeviceRow],
    pending: u32,
    dialect: PathDialect,
) -> DevicesStatus {
    let (state, mounted) = entry.map_or(("off", false), |entry| state_of(&entry.status));
    let (letter, root_path) = match entry.map(|entry| &entry.status) {
        Some(MountStatus::Mounted { drive }) | Some(MountStatus::Conflict { drive, .. }) => {
            (Some(drive.to_string()), Some(format!("{drive}\\")))
        }
        Some(MountStatus::MountedAt { path }) => (None, Some(path.to_string_lossy().into_owned())),
        _ => (None, None),
    };
    let mount_point = entry
        .filter(|_| cfg!(target_os = "linux"))
        .and_then(|entry| entry.config.location_directory())
        .map(|path| path.to_string_lossy().into_owned());
    let tree = layout(rows, dialect);
    let separator = if dialect == PathDialect::Windows {
        "\\"
    } else {
        "/"
    };
    let devices = rows
        .iter()
        .map(|row| {
            let folder = tree.folder_of(&row.key);
            let path = match (&root_path, &folder) {
                (Some(root), Some(folder)) if mounted => Some(format!(
                    "{}{}",
                    if root.ends_with(separator) {
                        root.clone()
                    } else {
                        format!("{root}{separator}")
                    },
                    folder.join(separator)
                )),
                _ => None,
            };
            DeviceStatusRow {
                key: row.key.clone(),
                name: folder
                    .as_ref()
                    .and_then(|folder| folder.last().cloned())
                    .unwrap_or_else(|| row.name.clone()),
                kind: match row.kind {
                    DeviceKind::Direct => "direct".into(),
                    DeviceKind::Room => "room".into(),
                },
                room: row.room.as_ref().map(|room| room.name.clone()),
                online: row.online,
                path,
            }
        })
        .collect();
    DevicesStatus {
        enabled: entry.is_some(),
        state: state.into(),
        mounted,
        root_path,
        letter,
        mount_point,
        revision: 0,
        devices,
        pending,
    }
}

/// The device tree as the worker sees it; starts the worker when it does
/// not run (like `list_mounts`).
pub fn devices_status() -> Result<DevicesStatus, String> {
    request(DriveIpcRequest::DevicesStatus, STATUS_TIMEOUT)
}

/// The answer after a change of the revision `since`, or after 30 s
/// unchanged.
pub fn devices_watch(since: u64) -> Result<DevicesStatus, String> {
    request(DriveIpcRequest::DevicesWatch { since }, WATCH_TIMEOUT)
}

/// Switches the device tree on. `letter` (Windows) or `mount_point` (Linux)
/// choose where it appears; without either the default applies.
pub fn devices_enable(
    letter: Option<String>,
    mount_point: Option<String>,
) -> Result<DevicesStatus, String> {
    let status = request(
        DriveIpcRequest::DevicesEnable {
            letter,
            mount_point,
        },
        SWITCH_TIMEOUT,
    )?;
    // The user switched it on himself: no hint about a tree that came by
    // itself (decision C5).
    devices_hint_done();
    Ok(status)
}

/// Switches it off: the drive is unmounted, the place in the file manager
/// goes, the unsaved changes of devices stay for the next switch-on.
pub fn devices_disable() -> Result<DevicesStatus, String> {
    request(DriveIpcRequest::DevicesDisable, SWITCH_TIMEOUT)
}

fn request(req: DriveIpcRequest, read_timeout: Duration) -> Result<DevicesStatus, String> {
    crate::daemon::ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(read_timeout));
    write_request(&mut stream, &IpcRequest::Drive { token, req })
        .map_err(|error| format!("Geräte-IPC senden: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Geräte-IPC lesen: {error}"))? {
        IpcResponse::Devices { status } => Ok(status),
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err("Unerwartete Geräte-IPC-Antwort".into()),
    }
}

#[cfg(test)]
#[path = "devices_tests.rs"]
mod tests;
