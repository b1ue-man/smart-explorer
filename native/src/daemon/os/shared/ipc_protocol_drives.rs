//! `IpcRequest::Drive { token, req }`: requests of the drive strand that
//! carry the daemon token (DRV-M3; plan 2026-10-17 C1 V2, V4 and C2 S1
//! Punkt 3). DRV-M3 adds the drive options and the restore; DRV-M6 adds the
//! device tree (`DevicesStatus`, `DevicesEnable`, `DevicesDisable`,
//! `DevicesWatch`) and DRV-M7 `MountPathFor` here. The answers reuse
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
    /// `se drive set`, the drive window: the options of a managed drive.
    SetOptions { id: MountId, auto_remount: bool },
    /// The login entry: mount every registered drive that restores at login
    /// and is not mounted.
    Restore,
}

impl DriveIpcRequest {
    /// Whether the request lifts the restriction of a worker started for the
    /// drive restore (S2 K7, `run_start_reason.rs`): requests of the app and
    /// of `se` do, the login entry does not. DRV-M6 adds the read-only views
    /// of se Desktop (`DevicesStatus`, `DevicesWatch`) as non-promoting.
    pub(crate) fn promotes_daemon(&self) -> bool {
        matches!(self, Self::SetOptions { .. })
    }
}

/// Serves one `IpcRequest::Drive` (arm in `ipc_host_ext.rs`).
pub(crate) fn serve(
    host: &ShareHost,
    stream: &mut TcpStream,
    req: DriveIpcRequest,
) -> io::Result<()> {
    let response = match req {
        DriveIpcRequest::SetOptions { id, auto_remount } => {
            match host.mounts.set_options(&id, auto_remount) {
                Ok(mount) => IpcResponse::Mount { mount },
                Err(msg) => IpcResponse::Err { msg },
            }
        }
        DriveIpcRequest::Restore => {
            super::run_start_reason::restore_drives(host);
            match host.mounts.list() {
                Ok(mounts) => IpcResponse::Mounts { mounts },
                Err(msg) => IpcResponse::Err { msg },
            }
        }
    };
    write_response(stream, &response)
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
        IpcResponse::Err { msg } => Err(msg),
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
    }
}
