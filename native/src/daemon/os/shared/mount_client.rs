//! Daemon control and offline recovery requests for managed drives.
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use crate::mount::{MountConfig, MountId, MountSnapshot};

use super::drive_modules::ipc_protocol_drives::DriveIpcRequest;
use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use super::ipc_storage::{read_ipc_addr, read_token};

pub use super::ipc_protocol::MountHostConfig;

// DRV-M7: `mount_path_for`, `ensure_mounted_for` (contract V2, S2-V8).
#[path = "mount_paths.rs"]
mod paths;
pub(in crate::daemon) use paths::mounts_for_location;
pub(crate) use paths::{ensure_mounted_for, mount_path_for};

// DRV-M6: the requests of the device-tree host (`child_backend`,
// `child_roots`, `device_watch`).
#[path = "mount_client_devices.rs"]
mod devices;

#[path = "mount_client_host.rs"]
mod host;
pub use host::{connect_mount_host, MountHostSession};
// The device-tree client keeps its existing internal helper paths.
use host::{host_stream, MountProxyIdentity};

// Stop/Retry may legitimately wait for one five-minute Dokany callback plus
// manager cleanup. Keep a margin above the manager's fail-safe stop grace.
const CONTROL_TIMEOUT: Duration =
    Duration::from_secs(super::mount_manager::STOP_GRACE.as_secs() + 30);

pub fn start_mount(config: MountConfig) -> Result<MountSnapshot, String> {
    mount_request(|token| IpcRequest::StartMount { token, config })
}

pub fn stop_mount(id: MountId) -> Result<MountSnapshot, String> {
    mount_request(|token| IpcRequest::StopMount { token, id })
}

pub fn retry_mount(id: MountId) -> Result<MountSnapshot, String> {
    mount_request(|token| IpcRequest::RetryMount { token, id })
}

pub fn save_remove_mount(
    id: MountId,
    destination: PathBuf,
) -> Result<crate::mount::InactiveMountRecovery, String> {
    let req = DriveIpcRequest::RecoverySaveRemove { id, destination };
    match recovery_request(req)? {
        IpcResponse::MountRecovery { recovery } => Ok(recovery),
        _ => Err("Unerwartete Recovery-IPC-Antwort".into()),
    }
}

pub fn list_mount_recoveries() -> Result<Vec<crate::mount::InactiveMountRecovery>, String> {
    match recovery_request(DriveIpcRequest::RecoveryList)? {
        IpcResponse::MountRecoveries { recoveries } => Ok(recoveries),
        _ => Err("Unerwartete Recoverybestands-IPC-Antwort".into()),
    }
}

pub fn import_mount_recovery(
    package: PathBuf,
) -> Result<crate::mount::InactiveMountRecovery, String> {
    match recovery_request(DriveIpcRequest::RecoveryImport { package })? {
        IpcResponse::MountRecovery { recovery } => Ok(recovery),
        _ => Err("Unerwartete Recoveryimport-IPC-Antwort".into()),
    }
}

pub fn reactivate_mount_recovery(id: String) -> Result<MountSnapshot, String> {
    match recovery_request(DriveIpcRequest::RecoveryReactivate { id })? {
        IpcResponse::Mount { mount } => Ok(mount),
        _ => Err("Unerwartete Recoveryreaktivierungs-IPC-Antwort".into()),
    }
}

/// Deletes an inactive recovery and the cache it owns (the saved package stays).
pub fn discard_mount_recovery(id: String) -> Result<(), String> {
    match recovery_request(DriveIpcRequest::RecoveryDiscard { id })? {
        IpcResponse::Ok => Ok(()),
        _ => Err("Unerwartete Recoverylösch-IPC-Antwort".into()),
    }
}

fn recovery_request(req: DriveIpcRequest) -> Result<IpcResponse, String> {
    super::ipc_client::ensure_worker_ready()?;
    let (mut stream, token) = authenticated_stream(CONTROL_TIMEOUT)?;
    // A complete local snapshot/import can legitimately exceed the host's
    // five-minute callback grace. EOF still reports a daemon exit; a fixed
    // control timeout must not report failure while its ownership commit runs.
    set_stream_timeout(&stream, None);
    write_request(&mut stream, &IpcRequest::Drive { token, req })
        .map_err(|error| format!("Recovery-IPC senden: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Recovery-IPC lesen: {error}"))? {
        IpcResponse::Err { msg, .. } => Err(msg),
        response => Ok(response),
    }
}

pub fn list_mounts() -> Result<Vec<MountSnapshot>, String> {
    super::ipc_client::ensure_worker_ready()?;
    let (mut stream, token) = authenticated_stream(CONTROL_TIMEOUT)?;
    write_request(&mut stream, &IpcRequest::ListMounts { token })
        .map_err(|error| format!("Laufwerk-IPC senden: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Laufwerk-IPC lesen: {error}"))? {
        IpcResponse::Mounts { mounts } => Ok(mounts),
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err("Unerwartete Laufwerk-IPC-Antwort".into()),
    }
}

fn mount_request(request: impl FnOnce(String) -> IpcRequest) -> Result<MountSnapshot, String> {
    super::ipc_client::ensure_worker_ready()?;
    let (mut stream, token) = authenticated_stream(CONTROL_TIMEOUT)?;
    write_request(&mut stream, &request(token))
        .map_err(|error| format!("Laufwerk-IPC senden: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Laufwerk-IPC lesen: {error}"))? {
        IpcResponse::Mount { mount } => Ok(mount),
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err("Unerwartete Laufwerk-IPC-Antwort".into()),
    }
}

fn authenticated_stream(timeout: Duration) -> Result<(TcpStream, String), String> {
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(timeout));
    Ok((stream, token))
}
