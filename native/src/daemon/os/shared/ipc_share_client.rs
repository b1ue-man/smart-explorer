use std::net::TcpStream;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse, ShareCommandReply,
};
use super::super::ipc_storage::{read_ipc_addr, read_token};
use super::{ensure_worker_ready, probe_worker, restart_worker_for_client, WorkerProbe};

/// Sends one Share command and returns the data the worker produced for it,
/// such as a new discovery offer with its end time.
pub fn share_command(cmd: crate::share::ShareCmd) -> Result<ShareCommandReply, String> {
    ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(Duration::from_secs(8)));
    write_request(&mut stream, &IpcRequest::ShareCommand { token, cmd })
        .map_err(|error| error.to_string())?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::Ok => Ok(ShareCommandReply::Applied),
        IpcResponse::ShareCommand { reply } => Ok(reply),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}

/// What a freshly installed executable did about the running worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerHandoff {
    /// No worker was running; none was started.
    NotRunning,
    /// The running worker already is this version.
    Current,
    /// A worker of another version was handed off to this executable.
    Replaced,
}

#[derive(Debug, PartialEq, Eq)]
enum HandoffAction {
    Report(WorkerHandoff),
    Replace,
    AwaitCurrent,
}

/// Runs in the newly installed executable: a worker of another version is
/// replaced by the version-bound handoff, a missing worker stays missing.
pub fn hand_off_running_worker() -> Result<WorkerHandoff, String> {
    if crate::autostart::DAEMON_IN_PROCESS {
        return Ok(WorkerHandoff::Current);
    }
    match handoff_action(&probe_worker(Duration::from_millis(700))) {
        HandoffAction::Report(handoff) => Ok(handoff),
        HandoffAction::Replace => restart_worker_for_client().map(|()| WorkerHandoff::Replaced),
        HandoffAction::AwaitCurrent => ensure_worker_ready().map(|()| WorkerHandoff::Current),
    }
}

fn handoff_action(probe: &WorkerProbe) -> HandoffAction {
    match probe {
        WorkerProbe::Missing => HandoffAction::Report(WorkerHandoff::NotRunning),
        WorkerProbe::Ready { .. } => HandoffAction::Report(WorkerHandoff::Current),
        WorkerProbe::Stale => HandoffAction::Replace,
        // Both answer with this version; wait until one serves requests.
        WorkerProbe::Starting { .. } | WorkerProbe::Retiring => HandoffAction::AwaitCurrent,
    }
}

#[cfg(test)]
mod tests {
    use super::{handoff_action, HandoffAction, WorkerHandoff, WorkerProbe};

    #[test]
    fn cli_task_handoff_never_starts_a_missing_worker() {
        let generation = "0".repeat(32);
        assert_eq!(
            handoff_action(&WorkerProbe::Missing),
            HandoffAction::Report(WorkerHandoff::NotRunning)
        );
        assert_eq!(
            handoff_action(&WorkerProbe::Ready {
                generation: generation.clone(),
            }),
            HandoffAction::Report(WorkerHandoff::Current)
        );
        assert_eq!(handoff_action(&WorkerProbe::Stale), HandoffAction::Replace);
        assert_eq!(
            handoff_action(&WorkerProbe::Starting { generation }),
            HandoffAction::AwaitCurrent
        );
        assert_eq!(
            handoff_action(&WorkerProbe::Retiring),
            HandoffAction::AwaitCurrent
        );
        assert_eq!(
            serde_json::to_string(&WorkerHandoff::NotRunning).unwrap(),
            "\"not_running\""
        );
    }
}
