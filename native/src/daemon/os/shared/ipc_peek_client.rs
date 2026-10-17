//! Reading the background service without starting it (DSK-M3, V4, V11).
//!
//! se Desktop reads Share (and later drive and vault) state as a second
//! reader next to the GUI. It must never start the background service: a
//! start would make this device reachable for paired devices without a user
//! decision (V11, S5 H1). It must also never take the UI events that belong to
//! the GUI (F8). Both functions here therefore probe the running worker with
//! `Ping` (`probe_worker`, the probe `ensure_worker_ready` starts with) and
//! send one request only to a ready worker of this version. A missing worker
//! and a worker of another version are `None`; `ensure_worker_ready` is never
//! called. A worker that is still starting or being replaced is reported as
//! an error with its reason, so the caller can name it.
use std::net::TcpStream;
use std::time::Duration;

use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse, ShareWorkerSnapshot,
};
use super::super::ipc_storage::{read_ipc_addr, read_token};
use super::{probe_worker, WorkerProbe};

/// Probe deadline, the first probe of `ensure_worker_ready`.
const PROBE_TIMEOUT: Duration = Duration::from_millis(700);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// Read deadline of a snapshot, as `share_worker_snapshot` allows it.
const SNAPSHOT_READ: Duration = Duration::from_secs(8);

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
        Some(IpcResponse::Err { msg }) => Err(msg),
        Some(_) => Err("Unerwartete Worker-Antwort".into()),
    }
}

/// Sends one request built from the IPC token to a ready worker and returns
/// its answer; `Ok(None)` when no worker of this version runs. `read_timeout`
/// bounds the answer (35 s for the drive watch of DSK-M4, V4).
pub(in crate::daemon) fn request_if_running(
    request: impl FnOnce(String) -> IpcRequest,
    read_timeout: Duration,
) -> Result<Option<IpcResponse>, String> {
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
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else {
        return Ok(None);
    };
    set_stream_timeout(&stream, Some(read_timeout));
    write_request(&mut stream, &request(token)).map_err(|error| error.to_string())?;
    read_response(&mut stream)
        .map(Some)
        .map_err(|error| error.to_string())
}
