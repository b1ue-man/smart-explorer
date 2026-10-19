//! Reading the background service without starting it (DSK-M3, DSK-M4, V4,
//! V10, V11).
//!
//! se Desktop reads Share, drive and vault state as a second reader next to
//! the GUI. It must never start the background service: a start would make
//! this device reachable for paired devices without a user decision (V11,
//! S5 H1). It must also never take the UI events that belong to the GUI
//! (F8). Every function here therefore probes the running worker with `Ping`
//! (`probe_worker`, the probe `ensure_worker_ready` starts with) and sends one
//! request only to a ready worker of this version. A missing worker and a
//! worker of another version are `None`; `ensure_worker_ready` is never
//! called. A worker that is still starting or being replaced is reported as
//! an error with its reason, so the caller can name it.
//!
//! The vault read of DSK-M4 is `VaultIpcRequest::Status` of VLT-M4 (U11),
//! which never starts the worker and never lifts the restriction of a
//! drive-restore start. The drive reads are typed since DRV-M6
//! (`DriveIpcRequest::DevicesStatus`, `DevicesWatch { since }`, answered as
//! `IpcResponse::Devices`); both count as not lifting the restriction of a
//! drive-restore start (V11, `promotes_daemon`). A worker that does not know
//! a request closes the connection without an answer ([`PEEK_UNSUPPORTED`]);
//! the desktop then shows the state as unknown.
use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;

use super::super::drive_modules::ipc_protocol_drives::DriveIpcRequest;
use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse, ShareWorkerSnapshot,
    VaultIpcReply, VaultIpcRequest, VaultStatusView,
};
use super::super::ipc_storage::{read_ipc_addr, read_token};
use super::super::line::{read_line_limited_from_stream, MAX_IPC_LINE};
use super::{probe_worker, WorkerProbe};

/// Probe deadline, the first probe of `ensure_worker_ready`.
const PROBE_TIMEOUT: Duration = Duration::from_millis(700);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// Read deadline of a snapshot, as `share_worker_snapshot` allows it.
const SNAPSHOT_READ: Duration = Duration::from_secs(8);
/// Read deadline of a device-tree or vault status.
const STATUS_READ: Duration = Duration::from_secs(8);
/// `DevicesWatch` answers after a change or after 30 s at the latest (V4).
const WATCH_READ: Duration = Duration::from_secs(35);

/// A worker that does not know a read: it closes without an answer.
pub const PEEK_UNSUPPORTED: &str = "Hintergrunddienst kennt diese Abfrage nicht";

/// The Share state of a running worker without starting one and without
/// consuming UI events (`IpcRequest::ShareSnapshotPeek`: the answer of
/// `ShareSnapshot`, but it never starts the Share host of a worker that was
/// started for the drive restore). `Ok(None)`: no worker of this version runs.
pub fn share_worker_peek() -> Result<Option<ShareWorkerSnapshot>, String> {
    match request_if_running(
        |token| IpcRequest::ShareSnapshotPeek { token },
        SNAPSHOT_READ,
    )? {
        None => Ok(None),
        Some(IpcResponse::ShareEvents { snapshot }) => Ok(Some(*snapshot)),
        Some(IpcResponse::Err { msg, .. }) => Err(msg),
        Some(_) => Err("Unerwartete Worker-Antwort".into()),
    }
}

/// The S2 device tree of a running worker (V4): the answer of
/// `DriveIpcRequest::DevicesStatus`. `Ok(None)`: no worker of this version.
pub fn drive_devices_peek() -> Result<Option<Value>, String> {
    devices_request_if_running(DriveIpcRequest::DevicesStatus, STATUS_READ)
}

/// `DriveIpcRequest::DevicesWatch { since }` of a running worker: the device
/// tree after a change of `since`, or after 30 s unchanged (V4).
pub fn drive_devices_watch_peek(since: u64) -> Result<Option<Value>, String> {
    devices_request_if_running(DriveIpcRequest::DevicesWatch { since }, WATCH_READ)
}

/// One typed drive request to a ready worker; the status comes back as JSON
/// (`feed_sources.rs` reads it as such). A worker without the request closes
/// the connection: [`PEEK_UNSUPPORTED`].
fn devices_request_if_running(
    req: DriveIpcRequest,
    read_timeout: Duration,
) -> Result<Option<Value>, String> {
    let Some((mut stream, token)) = connect_if_running(read_timeout)? else {
        return Ok(None);
    };
    write_request(&mut stream, &IpcRequest::Drive { token, req })
        .map_err(|error| error.to_string())?;
    match read_response(&mut stream) {
        Ok(IpcResponse::Devices { status }) => serde_json::to_value(&status)
            .map(Some)
            .map_err(|error| error.to_string()),
        Ok(IpcResponse::Err { msg, .. }) => Err(msg),
        Ok(_) => Err("Unerwartete Worker-Antwort".into()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            Err(error.to_string())
        }
        Err(_) => Err(PEEK_UNSUPPORTED.into()),
    }
}

/// The vault sites of a running worker (`VaultIpcRequest::Status`, V10,
/// H5). `Ok(None)`: no worker of this version runs (then every vault is
/// locked); a worker without vault sessions refuses the request.
pub(crate) fn vault_status_peek() -> Result<Option<VaultStatusView>, String> {
    match request_if_running(
        |token| IpcRequest::Vault {
            token,
            req: VaultIpcRequest::Status,
        },
        STATUS_READ,
    )? {
        None => Ok(None),
        Some(IpcResponse::Vault(VaultIpcReply::Status(status))) => Ok(Some(status)),
        Some(IpcResponse::Err { msg, .. }) => Err(msg),
        Some(_) => Err("Unerwartete Worker-Antwort".into()),
    }
}

/// Sends one request built from the IPC token to a ready worker and returns
/// its answer; `Ok(None)` when no worker of this version runs. `read_timeout`
/// bounds the answer.
pub(in crate::daemon) fn request_if_running(
    request: impl FnOnce(String) -> IpcRequest,
    read_timeout: Duration,
) -> Result<Option<IpcResponse>, String> {
    let Some((mut stream, token)) = connect_if_running(read_timeout)? else {
        return Ok(None);
    };
    write_request(&mut stream, &request(token)).map_err(|error| error.to_string())?;
    read_response(&mut stream)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Like [`request_if_running`] for a request in its wire form; the answer
/// comes back as JSON. An error answer (`{"t":"err","msg"}`) is `Err(msg)`,
/// no answer at all is [`PEEK_UNSUPPORTED`]. Kept for a read whose variant
/// a worker of another version may lack; no caller needs it at the moment.
#[allow(dead_code)]
fn raw_request_if_running(
    request: impl FnOnce(&str) -> Value,
    read_timeout: Duration,
) -> Result<Option<Value>, String> {
    let Some((mut stream, token)) = connect_if_running(read_timeout)? else {
        return Ok(None);
    };
    let mut line = serde_json::to_string(&request(&token)).map_err(|error| error.to_string())?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|error| error.to_string())?;
    let mut answer = String::new();
    read_line_limited_from_stream(&mut stream, &mut answer, MAX_IPC_LINE)
        .map_err(|error| error.to_string())?;
    let answer = answer.trim();
    if answer.is_empty() {
        return Err(PEEK_UNSUPPORTED.into());
    }
    let value: Value = serde_json::from_str(answer).map_err(|error| error.to_string())?;
    if value.get("t").and_then(Value::as_str) == Some("err") {
        let message = value.get("msg").and_then(Value::as_str);
        return Err(message.unwrap_or(PEEK_UNSUPPORTED).to_string());
    }
    Ok(Some(value))
}

/// A connection to a ready worker of this version with its IPC token;
/// `Ok(None)` without one.
fn connect_if_running(read_timeout: Duration) -> Result<Option<(TcpStream, String)>, String> {
    match probe_worker(PROBE_TIMEOUT) {
        WorkerProbe::Ready { .. } => {}
        WorkerProbe::Stale | WorkerProbe::Missing => return Ok(None),
        WorkerProbe::Starting { .. } => return Err("Hintergrunddienst startet noch".into()),
        WorkerProbe::Retiring => return Err("Hintergrunddienst wird gerade ersetzt".into()),
    }
    let Ok(token) = read_token() else {
        return Ok(None);
    };
    let Some(addr) = read_ipc_addr() else {
        return Ok(None);
    };
    let Ok(stream) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else {
        return Ok(None);
    };
    set_stream_timeout(&stream, Some(read_timeout));
    Ok(Some((stream, token)))
}

#[cfg(test)]
#[path = "ipc_peek_client_tests.rs"]
mod tests;
