use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExecJobsSnapshot {
    pub outgoing_active: Vec<crate::share::ExecJobView>,
    pub outgoing_history: Vec<crate::share::ExecJobView>,
    pub incoming_active: Vec<crate::share::ExecJobView>,
    pub incoming_history: Vec<crate::share::ExecJobView>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecJobDirection {
    Outgoing,
    Incoming,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecCancelTarget {
    pub direction: ExecJobDirection,
    pub exec_id: crate::share::ExecId,
    pub peer_device_id: String,
}

pub(super) struct ExecState {
    inner: Mutex<State>,
}

#[derive(Default)]
struct State {
    active: HashMap<crate::share::ExecId, Outgoing>,
    history: VecDeque<crate::share::ExecJobView>,
}

struct Outgoing {
    view: crate::share::ExecJobView,
    input: crate::share::ShareExecInput,
}

impl ExecState {
    pub(super) fn new() -> Self {
        Self {
            inner: Mutex::new(State::default()),
        }
    }

    pub(super) fn begin(
        &self,
        view: crate::share::ExecJobView,
        input: crate::share::ShareExecInput,
    ) {
        if let Ok(mut state) = self.inner.lock() {
            state
                .active
                .insert(view.exec_id.clone(), Outgoing { view, input });
        }
    }

    pub(super) fn authorized(&self, id: &crate::share::ExecId, revision: u64) {
        self.update(id, |view| {
            view.state = crate::share::ExecLifecycleState::Authorized;
            view.policy_revision = revision;
        });
    }

    pub(super) fn started(&self, id: &crate::share::ExecId) {
        self.update(id, |view| {
            view.state = crate::share::ExecLifecycleState::Running;
            view.started_at = Some(crate::share::core_now_secs());
        });
    }

    pub(super) fn terminal(&self, terminal: &crate::share::ExecTerminal) {
        if let Ok(mut state) = self.inner.lock() {
            let Some(mut active) = state.active.remove(&terminal.exec_id) else {
                return;
            };
            active.view.state = lifecycle(&terminal.kind);
            active.view.finished_at = Some(crate::share::core_now_secs());
            active.view.terminal = Some(terminal.clone());
            push_history(&mut state.history, active.view);
        }
    }

    pub(super) fn failed(
        &self,
        id: &crate::share::ExecId,
        failure: &crate::daemon::ExecIpcFailure,
    ) {
        if let Ok(mut state) = self.inner.lock() {
            let Some(mut active) = state.active.remove(id) else {
                return;
            };
            let kind = if failure.kind == "disconnected" {
                crate::share::ExecTerminalKind::Disconnected
            } else {
                crate::share::ExecTerminalKind::Failed
            };
            active.view.state = lifecycle(&kind);
            active.view.finished_at = Some(crate::share::core_now_secs());
            active.view.terminal = Some(crate::share::ExecTerminal {
                exec_id: id.clone(),
                kind,
                exit_code: None,
                signal: None,
                message: Some(format!("{}: {}", failure.code, failure.message)),
                stdout_bytes: 0,
                stderr_bytes: 0,
                output_truncated: false,
            });
            push_history(&mut state.history, active.view);
        }
    }

    pub(super) fn snapshot(
        &self,
    ) -> (
        Vec<crate::share::ExecJobView>,
        Vec<crate::share::ExecJobView>,
    ) {
        self.inner
            .lock()
            .map(|state| {
                (
                    state
                        .active
                        .values()
                        .map(|entry| entry.view.clone())
                        .collect(),
                    state.history.iter().cloned().collect(),
                )
            })
            .unwrap_or_default()
    }

    pub(super) fn cancel(&self, id: &crate::share::ExecId, peer_device_id: &str) -> bool {
        let input = self.inner.lock().ok().and_then(|state| {
            state.active.get(id).and_then(|entry| {
                (entry.view.peer_device_id == peer_device_id).then(|| entry.input.clone())
            })
        });
        input.is_some_and(|input| input.send(crate::share::ExecClientInput::Cancel).is_ok())
    }

    fn update(
        &self,
        id: &crate::share::ExecId,
        update: impl FnOnce(&mut crate::share::ExecJobView),
    ) {
        if let Ok(mut state) = self.inner.lock() {
            if let Some(entry) = state.active.get_mut(id) {
                update(&mut entry.view);
            }
        }
    }
}

fn push_history(
    history: &mut VecDeque<crate::share::ExecJobView>,
    view: crate::share::ExecJobView,
) {
    history.push_back(view);
    while history.len() > 128 {
        history.pop_front();
    }
}

fn lifecycle(kind: &crate::share::ExecTerminalKind) -> crate::share::ExecLifecycleState {
    match kind {
        crate::share::ExecTerminalKind::Exited => crate::share::ExecLifecycleState::Exited,
        crate::share::ExecTerminalKind::Failed => crate::share::ExecLifecycleState::Failed,
        crate::share::ExecTerminalKind::TimedOut => crate::share::ExecLifecycleState::TimedOut,
        crate::share::ExecTerminalKind::Cancelled => crate::share::ExecLifecycleState::Cancelled,
        crate::share::ExecTerminalKind::Revoked => crate::share::ExecLifecycleState::Revoked,
        crate::share::ExecTerminalKind::Disconnected => {
            crate::share::ExecLifecycleState::Disconnected
        }
    }
}

pub(super) fn snapshot(host: &super::ipc_host::ShareHost) -> ExecJobsSnapshot {
    let (outgoing_active, outgoing_history) = host.exec_state.snapshot();
    let service = host
        .state
        .lock()
        .ok()
        .and_then(|state| state.service.clone());
    let (incoming_active, incoming_history) = service
        .map(|service| (service.exec_active_views(), service.exec_history()))
        .unwrap_or_default();
    ExecJobsSnapshot {
        outgoing_active,
        outgoing_history,
        incoming_active,
        incoming_history,
    }
}

pub(super) fn cancel(host: &super::ipc_host::ShareHost, target: &ExecCancelTarget) -> bool {
    cancel_routed(
        target,
        |id, peer| host.exec_state.cancel(id, peer),
        |id, peer| {
            host.state
                .lock()
                .ok()
                .and_then(|state| state.service.clone())
                .is_some_and(|service| service.cancel_exec(id, peer))
        },
    )
}

fn cancel_routed(
    target: &ExecCancelTarget,
    cancel_outgoing: impl FnOnce(&crate::share::ExecId, &str) -> bool,
    cancel_incoming: impl FnOnce(&crate::share::ExecId, &str) -> bool,
) -> bool {
    match target.direction {
        ExecJobDirection::Outgoing => cancel_outgoing(&target.exec_id, &target.peer_device_id),
        ExecJobDirection::Incoming => cancel_incoming(&target.exec_id, &target.peer_device_id),
    }
}

pub fn load() -> Result<ExecJobsSnapshot, String> {
    request(super::ipc_protocol::IpcRequest::ExecJobs { token: token()? }).and_then(|response| {
        match response {
            super::ipc_protocol::IpcResponse::ExecJobs { snapshot } => Ok(snapshot),
            super::ipc_protocol::IpcResponse::Err { msg } => Err(msg),
            _ => Err("Unerwartete Worker-Antwort auf Exec-Status".into()),
        }
    })
}

pub fn cancel_remote(target: ExecCancelTarget) -> Result<bool, String> {
    request(super::ipc_protocol::IpcRequest::CancelExec {
        token: token()?,
        target,
    })
    .and_then(|response| match response {
        super::ipc_protocol::IpcResponse::ExecCancelled { found } => Ok(found),
        super::ipc_protocol::IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort auf Exec-Abbruch".into()),
    })
}

fn token() -> Result<String, String> {
    super::ipc_client::ensure_worker_ready()?;
    super::ipc_storage::read_token().map_err(|error| format!("Background-Worker Token: {error}"))
}

fn request(
    request: super::ipc_protocol::IpcRequest,
) -> Result<super::ipc_protocol::IpcResponse, String> {
    let addr = super::ipc_storage::read_ipc_addr()
        .ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    super::ipc_protocol::set_stream_timeout(&stream, Some(Duration::from_secs(8)));
    super::ipc_protocol::write_request(&mut stream, &request).map_err(|error| error.to_string())?;
    super::ipc_protocol::read_response(&mut stream).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn same_exec_id_is_cancelled_only_in_requested_direction() {
        let outgoing_calls = Cell::new(0);
        let incoming_calls = Cell::new(0);
        let target = ExecCancelTarget {
            direction: ExecJobDirection::Incoming,
            exec_id: crate::share::ExecId::parse("22".repeat(16)).unwrap(),
            peer_device_id: "incoming-peer".into(),
        };
        let cancelled = cancel_routed(
            &target,
            |_, _| {
                outgoing_calls.set(outgoing_calls.get() + 1);
                true
            },
            |id, peer| {
                incoming_calls.set(incoming_calls.get() + 1);
                assert_eq!(id, &target.exec_id);
                assert_eq!(peer, "incoming-peer");
                true
            },
        );
        assert!(cancelled);
        assert_eq!(outgoing_calls.get(), 0);
        assert_eq!(incoming_calls.get(), 1);
    }

    #[test]
    fn wrong_peer_identity_cancels_nothing() {
        let target = ExecCancelTarget {
            direction: ExecJobDirection::Outgoing,
            exec_id: crate::share::ExecId::parse("33".repeat(16)).unwrap(),
            peer_device_id: "attacker-selected-peer".into(),
        };
        assert!(!cancel_routed(
            &target,
            |_, peer| peer == "real-peer",
            |_, _| panic!("incoming registry must not be queried"),
        ));
    }
}
