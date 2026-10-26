//! `IpcRequest::Drive { token, req }`: requests of the drive strand that
//! carry the daemon token (DRV-M3; plan 2026-10-17 C1 V2, V4 and C2 S1
//! Punkt 3). DRV-M3 adds the drive options and the restore; DRV-M6 adds the
//! device tree (`DevicesStatus`, `DevicesEnable`, `DevicesDisable`,
//! `DevicesWatch`) and DRV-M7 `MountPathFor` (`mount_paths.rs`) here. The answers reuse
//! `IpcResponse::Mount` and `IpcResponse::Mounts`, so the hub enum of
//! answers does not grow.
use serde::{Deserialize, Serialize};
use std::io;
use std::net::TcpStream;
use std::time::Duration;

use crate::mount::{MountId, MountSnapshot};

use super::super::ipc_host::ShareHost;
use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, write_response, IpcRequest, IpcResponse,
};
use super::super::ipc_storage::{read_ipc_addr, read_token};

/// A change of options waits behind a running start or stop of the same
/// worker (lifecycle lock); the margin matches `mount_client.rs`.
const OPTIONS_TIMEOUT: Duration =
    Duration::from_secs(super::super::mount_manager::STOP_GRACE.as_secs() + 30);
const RESTORE_CONNECT: Duration = Duration::from_millis(500);
const RESTORE_REPLY: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum DriveIpcRequest {
    /// P22: identical offline transaction for desktop, CLI/JSON and MCP.
    RecoverySaveRemove {
        id: MountId,
        destination: std::path::PathBuf,
    },
    RecoveryList,
    RecoveryImport {
        package: std::path::PathBuf,
    },
    RecoveryReactivate {
        id: String,
    },
    /// `se drive set`, the drive window: the options of a managed drive.
    SetOptions {
        id: MountId,
        auto_remount: bool,
    },
    /// The login entry: mount every registered drive that restores at login
    /// and is not mounted.
    Restore,
    /// DRV-M7 (S2-V8): the mounted drive that serves `location`, answered
    /// as `IpcResponse::Mounts` with at most one drive.
    MountPathFor {
        location: String,
    },
    /// DRV-M6 (V4): the device tree „Verbundene Geräte als Ordner“, answered
    /// as `IpcResponse::Devices`.
    DevicesStatus,
    /// Switches it on; `letter` (Windows) or `mount_point` (Linux) choose
    /// where it appears, `null` takes the default.
    DevicesEnable {
        letter: Option<String>,
        mount_point: Option<String>,
    },
    DevicesDisable,
    /// The status after a change of the revision `since`, at the latest
    /// after 30 s.
    DevicesWatch {
        since: u64,
    },
}

impl DriveIpcRequest {
    /// Whether the request lifts the restriction of a worker started for the
    /// drive restore (S2 K7, `run_start_reason.rs`): requests of the app and
    /// of `se` do, the login entry does not. DRV-M6 adds the read-only views
    /// of se Desktop (`DevicesStatus`, `DevicesWatch`) as non-promoting.
    /// `MountPathFor` comes only from opening a file in the app, `se` or
    /// `se mcp`, so it promotes like every other request of theirs; so do
    /// switching the device tree on and off, which are acts of the user.
    pub(crate) fn promotes_daemon(&self) -> bool {
        matches!(
            self,
            Self::SetOptions { .. }
                | Self::RecoverySaveRemove { .. }
                | Self::RecoveryImport { .. }
                | Self::RecoveryReactivate { .. }
                | Self::MountPathFor { .. }
                | Self::DevicesEnable { .. }
                | Self::DevicesDisable
        )
    }
}

/// Serves one `IpcRequest::Drive` (arm in `ipc_host_ext.rs`).
pub(crate) fn serve(
    host: &ShareHost,
    stream: &mut TcpStream,
    req: DriveIpcRequest,
) -> io::Result<()> {
    let response = match req {
        DriveIpcRequest::RecoverySaveRemove { id, destination } => {
            recovery_answer(host.mounts.save_remove(&id, &destination))
        }
        DriveIpcRequest::RecoveryList => match host.mounts.recovery_list() {
            Ok(recoveries) => IpcResponse::MountRecoveries { recoveries },
            Err(msg) => IpcResponse::Err { msg, code: None },
        },
        DriveIpcRequest::RecoveryImport { package } => {
            recovery_answer(host.mounts.recovery_import(&package))
        }
        DriveIpcRequest::RecoveryReactivate { id } => match host.mounts.recovery_reactivate(&id) {
            Ok(mount) => IpcResponse::Mount { mount },
            Err(msg) => IpcResponse::Err { msg, code: None },
        },
        DriveIpcRequest::SetOptions { id, auto_remount } => {
            match host.mounts.set_options(&id, auto_remount) {
                Ok(mount) => IpcResponse::Mount { mount },
                Err(msg) => IpcResponse::Err { msg, code: None },
            }
        }
        DriveIpcRequest::Restore => {
            super::run_start_reason::restore_drives(host);
            match host.mounts.list() {
                Ok(mounts) => IpcResponse::Mounts { mounts },
                Err(msg) => IpcResponse::Err { msg, code: None },
            }
        }
        DriveIpcRequest::MountPathFor { location } => {
            match host.mounts.list().and_then(|mounts| {
                super::super::mount_client::mounts_for_location(mounts, &location)
            }) {
                Ok(mounts) => IpcResponse::Mounts { mounts },
                Err(msg) => IpcResponse::Err { msg, code: None },
            }
        }
        DriveIpcRequest::DevicesStatus => IpcResponse::Devices {
            status: super::devices_mount::status(host),
        },
        DriveIpcRequest::DevicesWatch { since } => IpcResponse::Devices {
            status: super::devices_host::watch_status(host, since),
        },
        DriveIpcRequest::DevicesEnable {
            letter,
            mount_point,
        } => devices_answer(super::devices_mount::enable(host, letter, mount_point)),
        DriveIpcRequest::DevicesDisable => devices_answer(super::devices_mount::disable(host)),
    };
    write_response(stream, &response)
}

fn recovery_answer(result: Result<crate::mount::InactiveMountRecovery, String>) -> IpcResponse {
    match result {
        Ok(recovery) => IpcResponse::MountRecovery { recovery },
        Err(msg) => IpcResponse::Err { msg, code: None },
    }
}

fn devices_answer(result: Result<super::devices_ipc::DevicesStatus, String>) -> IpcResponse {
    match result {
        Ok(status) => IpcResponse::Devices { status },
        Err(msg) => IpcResponse::Err { msg, code: None },
    }
}

/// `se drive set` and the drive window: switches „Beim Anmelden automatisch
/// einbinden“ of a managed drive; starts the worker when it does not run.
pub(crate) fn set_mount_options(id: MountId, auto_remount: bool) -> Result<MountSnapshot, String> {
    crate::daemon::ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(OPTIONS_TIMEOUT));
    let request = IpcRequest::Drive {
        token,
        req: DriveIpcRequest::SetOptions { id, auto_remount },
    };
    write_request(&mut stream, &request)
        .map_err(|error| format!("Laufwerk-IPC senden: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Laufwerk-IPC lesen: {error}"))? {
        IpcResponse::Mount { mount } => Ok(mount),
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err("Unerwartete Laufwerk-IPC-Antwort".into()),
    }
}

/// The login entry: asks a worker that already runs to restore its drives.
/// Never starts a worker; every failure means there is none to ask.
pub(crate) fn request_restore_if_running() {
    let Some(addr) = read_ipc_addr().filter(|addr| addr.ip().is_loopback()) else {
        return;
    };
    let Ok(token) = read_token() else {
        return;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, RESTORE_CONNECT) else {
        return;
    };
    set_stream_timeout(&stream, Some(RESTORE_REPLY));
    let request = IpcRequest::Drive {
        token,
        req: DriveIpcRequest::Restore,
    };
    if write_request(&mut stream, &request).is_ok() {
        let _ = read_response(&mut stream);
    }
}

#[cfg(test)]
mod tests {
    use super::DriveIpcRequest;
    use crate::mount::MountId;

    #[test]
    fn drive_requests_keep_their_wire_form() {
        let options = DriveIpcRequest::SetOptions {
            id: MountId::parse("drive-1").unwrap(),
            auto_remount: true,
        };
        let encoded = serde_json::to_string(&options).unwrap();
        assert_eq!(
            encoded,
            r#"{"op":"set_options","id":"drive-1","auto_remount":true}"#
        );
        assert_eq!(
            serde_json::from_str::<DriveIpcRequest>(&encoded).unwrap(),
            options
        );
        assert_eq!(
            serde_json::to_string(&DriveIpcRequest::Restore).unwrap(),
            r#"{"op":"restore"}"#
        );
        let path_for = DriveIpcRequest::MountPathFor {
            location: "vault://Privat/a.txt".into(),
        };
        assert_eq!(
            serde_json::to_string(&path_for).unwrap(),
            r#"{"op":"mount_path_for","location":"vault://Privat/a.txt"}"#
        );
        assert!(path_for.promotes_daemon());
        assert!(!DriveIpcRequest::Restore.promotes_daemon());
    }

    #[test]
    fn devices_wire_requests_keep_their_form() {
        let lines = [
            (DriveIpcRequest::DevicesStatus, r#"{"op":"devices_status"}"#),
            (
                DriveIpcRequest::DevicesEnable {
                    letter: Some("X".into()),
                    mount_point: None,
                },
                r#"{"op":"devices_enable","letter":"X","mount_point":null}"#,
            ),
            (
                DriveIpcRequest::DevicesEnable {
                    letter: None,
                    mount_point: Some("/home/u/Geräte".into()),
                },
                r#"{"op":"devices_enable","letter":null,"mount_point":"/home/u/Geräte"}"#,
            ),
            (
                DriveIpcRequest::DevicesDisable,
                r#"{"op":"devices_disable"}"#,
            ),
            (
                DriveIpcRequest::DevicesWatch { since: 12 },
                r#"{"op":"devices_watch","since":12}"#,
            ),
        ];
        for (request, line) in lines {
            assert_eq!(serde_json::to_string(&request).unwrap(), line);
            assert_eq!(
                serde_json::from_str::<DriveIpcRequest>(line).unwrap(),
                request
            );
        }
    }

    /// The read-only views of se Desktop never lift the restriction of a
    /// worker started for the drive restore (V11); switching the tree does.
    #[test]
    fn drive_task_devices_watch_not_promoting() {
        assert!(!DriveIpcRequest::DevicesStatus.promotes_daemon());
        assert!(!DriveIpcRequest::DevicesWatch { since: 0 }.promotes_daemon());
        assert!(DriveIpcRequest::DevicesDisable.promotes_daemon());
        assert!(DriveIpcRequest::DevicesEnable {
            letter: None,
            mount_point: None
        }
        .promotes_daemon());
    }
}
