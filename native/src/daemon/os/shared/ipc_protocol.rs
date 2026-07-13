use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::line::{read_line_limited_from_stream, MAX_IPC_LINE};

const MAX_EVENT_BYTES: usize = 384 * 1024;
const MAX_LEGACY_REQUEST_BYTES: usize = 256 * 1024;
const MAX_CANDIDATE_BYTES: usize = 128 * 1024;
const MAX_STATUS_TEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShareWorkerSnapshot {
    pub events: Vec<crate::share::ShareEvent>,
    /// Canonical daemon-owned profile state. Consumers must replace stale
    /// cached copies instead of replaying worker events as profile writes.
    #[serde(default)]
    pub profiles: crate::share::ShareProfiles,
    #[serde(default)]
    pub(crate) profile_revision: crate::share::ProfileRevision,
    #[serde(default)]
    pub(crate) exec_grant_retry:
        Option<super::ipc_host::exec_grant_journal::ExecGrantPersistResult>,
    #[serde(default)]
    pub pending_direct_requests: Vec<crate::share::PeerPresence>,
    pub running: bool,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    pub relay_url: String,
    pub candidates: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(super) enum IpcRequest {
    Ping {
        token: String,
    },
    RefreshShare {
        token: String,
    },
    ShareCommand {
        token: String,
        cmd: crate::share::ShareCmd,
    },
    MutateExecGrant {
        token: String,
        target: crate::share::ExecGrantTarget,
        enabled: bool,
    },
    DrainShareEvents {
        token: String,
    },
    OpenShare {
        token: String,
        target: crate::share::PeerOpenTarget,
    },
    ExecShare {
        token: String,
        target: crate::share::PeerOpenTarget,
        req: crate::share::ExecRequest,
    },
    ExecStream {
        token: String,
        target: crate::share::PeerOpenTarget,
        start: crate::share::ExecStart,
    },
    ExecJobs {
        token: String,
    },
    CancelExec {
        token: String,
        target: super::exec_state::ExecCancelTarget,
    },
}

impl IpcRequest {
    pub(super) fn token(&self) -> &str {
        match self {
            Self::Ping { token }
            | Self::RefreshShare { token }
            | Self::ShareCommand { token, .. }
            | Self::MutateExecGrant { token, .. }
            | Self::DrainShareEvents { token }
            | Self::OpenShare { token, .. }
            | Self::ExecShare { token, .. }
            | Self::ExecStream { token, .. }
            | Self::ExecJobs { token }
            | Self::CancelExec { token, .. } => token,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum IpcResponse {
    Pong {
        #[serde(default)]
        version: String,
        #[serde(default)]
        generation: String,
        #[serde(default)]
        initialized: bool,
    },
    Ok,
    RefreshOk {
        running: bool,
    },
    OpenOk {
        label: String,
        status: crate::share::ShareStatus,
    },
    ShareEvents {
        snapshot: Box<ShareWorkerSnapshot>,
    },
    ExecResult {
        result: crate::share::ExecResult,
    },
    ExecGrantMutation {
        result: super::ipc_host::exec_grant_journal::ExecGrantPersistResult,
    },
    ExecReady {
        exec_id: crate::share::ExecId,
    },
    ExecJobs {
        snapshot: super::exec_state::ExecJobsSnapshot,
    },
    ExecCancelled {
        found: bool,
    },
    Err {
        msg: String,
    },
}

pub(super) fn write_response(stream: &mut TcpStream, response: &IpcResponse) -> io::Result<()> {
    let mut line = serde_json::to_string(response).map_err(eio)?;
    line.push('\n');
    if line.len() > MAX_IPC_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ipc response exceeds line budget",
        ));
    }
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

pub(super) fn bound_snapshot_for_ipc(mut snapshot: ShareWorkerSnapshot) -> ShareWorkerSnapshot {
    let dropped_events = retain_newest_with_budget(&mut snapshot.events, MAX_EVENT_BYTES);
    let dropped_legacy = retain_newest_with_budget(
        &mut snapshot.pending_direct_requests,
        MAX_LEGACY_REQUEST_BYTES,
    );
    let dropped_candidates =
        retain_newest_with_budget(&mut snapshot.candidates, MAX_CANDIDATE_BYTES);
    if let Some(error) = &mut snapshot.last_error {
        truncate_utf8(error, MAX_STATUS_TEXT_BYTES);
    }
    truncate_utf8(&mut snapshot.relay_url, MAX_STATUS_TEXT_BYTES);

    if dropped_events + dropped_legacy + dropped_candidates > 0 {
        snapshot.events.push(crate::share::ShareEvent::Error(format!(
            "Share status truncated transient backlog: events={dropped_events}, legacy_requests={dropped_legacy}, candidates={dropped_candidates}; durable request/profile state is complete"
        )));
    }

    while encoded_response_len(&snapshot) > MAX_IPC_LINE {
        if !snapshot.events.is_empty() {
            snapshot.events.remove(0);
            continue;
        }
        if snapshot.candidates.pop().is_some() {
            continue;
        }
        if snapshot.pending_direct_requests.pop().is_some() {
            continue;
        }
        break;
    }
    snapshot
}

fn retain_newest_with_budget<T: Serialize>(values: &mut Vec<T>, budget: usize) -> usize {
    let original_len = values.len();
    let mut used = 2usize;
    let mut kept = Vec::new();
    for value in std::mem::take(values).into_iter().rev() {
        let bytes = serde_json::to_vec(&value)
            .map(|encoded| encoded.len().saturating_add(1))
            .unwrap_or(budget.saturating_add(1));
        if used.saturating_add(bytes) <= budget {
            used += bytes;
            kept.push(value);
        }
    }
    kept.reverse();
    *values = kept;
    original_len.saturating_sub(values.len())
}

fn encoded_response_len(snapshot: &ShareWorkerSnapshot) -> usize {
    serde_json::to_vec(&IpcResponse::ShareEvents {
        snapshot: Box::new(snapshot.clone()),
    })
    .map(|encoded| encoded.len().saturating_add(1))
    .unwrap_or(usize::MAX)
}

fn truncate_utf8(value: &mut String, max: usize) {
    if value.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

pub(super) fn set_stream_timeout(stream: &TcpStream, timeout: Option<Duration>) {
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);
}

pub(super) fn write_request(stream: &mut TcpStream, req: &IpcRequest) -> io::Result<()> {
    let mut line = serde_json::to_string(req).map_err(eio)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

pub(super) fn read_response(stream: &mut TcpStream) -> io::Result<IpcResponse> {
    let mut line = String::new();
    read_line_limited_from_stream(stream, &mut line, MAX_IPC_LINE)?;
    serde_json::from_str(line.trim()).map_err(eio)
}

fn eio<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        bound_snapshot_for_ipc, encoded_response_len, read_response, IpcRequest, IpcResponse,
        ShareWorkerSnapshot, MAX_IPC_LINE,
    };
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn response_read_preserves_following_stream_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.write_all(br#"{"t":"ok"}"#).unwrap();
            socket.write_all(b"\nAGENT").unwrap();
            socket.flush().unwrap();
        });
        let mut client = TcpStream::connect(addr).unwrap();
        match read_response(&mut client).unwrap() {
            IpcResponse::Ok => {}
            other => panic!("unexpected response: {other:?}"),
        }
        let mut rest = [0u8; 5];
        client.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"AGENT");
        server.join().unwrap();
    }

    #[test]
    fn share_snapshot_carries_the_profile_cas_revision() {
        let snapshot = ShareWorkerSnapshot {
            profile_revision: crate::share::ProfileRevision::Digest([7; 32]),
            ..ShareWorkerSnapshot::default()
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: ShareWorkerSnapshot = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            decoded.profile_revision,
            crate::share::ProfileRevision::Digest([7; 32])
        );
    }

    #[test]
    fn maximum_profile_and_event_backlog_fit_one_ipc_response() {
        let mut snapshot = ShareWorkerSnapshot::default();
        for index in 0..210 {
            snapshot
                .profiles
                .default_direct_exports
                .roots
                .push(crate::share::SharedRoot {
                    label: format!("root-{index}"),
                    path: format!("/{}", "x".repeat(4096)),
                });
        }
        snapshot.events = (0..512)
            .map(|index| {
                crate::share::ShareEvent::Status(format!("event-{index}-{}", "y".repeat(4096)))
            })
            .collect();
        assert!(encoded_response_len(&snapshot) > MAX_IPC_LINE);

        let bounded = bound_snapshot_for_ipc(snapshot);

        assert!(encoded_response_len(&bounded) <= MAX_IPC_LINE);
        assert_eq!(bounded.profiles.default_direct_exports.roots.len(), 210);
        assert!(bounded.events.iter().any(|event| matches!(
            event,
            crate::share::ShareEvent::Error(message) if message.contains("truncated transient backlog")
        )));
    }

    #[test]
    fn old_unit_pong_deserializes_as_stale_version() {
        match serde_json::from_str::<IpcResponse>(r#"{"t":"pong"}"#).unwrap() {
            IpcResponse::Pong {
                version,
                generation,
                initialized,
            } => {
                assert!(version.is_empty());
                assert!(generation.is_empty());
                assert!(!initialized);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn exec_share_ipc_roundtrips_request_and_response() {
        let (target, _) =
            crate::share::PeerOpenTarget::from_endpoint("share://direct/contact-a/bin").unwrap();
        let req = crate::share::ExecRequest {
            argv: vec!["echo".into(), "hi".into()],
            cwd: Some("/tmp".into()),
            timeout_ms: 5_000,
            max_output_bytes: 128,
            shell: false,
        };
        let json = serde_json::to_string(&IpcRequest::ExecShare {
            token: "token".into(),
            target: target.clone(),
            req: req.clone(),
        })
        .unwrap();
        match serde_json::from_str::<IpcRequest>(&json).unwrap() {
            IpcRequest::ExecShare {
                token,
                target: got_target,
                req: got_req,
            } => {
                assert_eq!(token, "token");
                assert_eq!(got_target, target);
                assert_eq!(got_req.argv, req.argv);
                assert_eq!(got_req.cwd, req.cwd);
                assert_eq!(got_req.timeout_ms, req.timeout_ms);
                assert_eq!(got_req.max_output_bytes, req.max_output_bytes);
                assert_eq!(got_req.shell, req.shell);
            }
            _ => panic!("wrong request"),
        }

        let response = IpcResponse::ExecResult {
            result: crate::share::ExecResult {
                stdout: b"hi\n".to_vec(),
                stderr: Vec::new(),
                exit_code: Some(0),
                timed_out: false,
                stdout_truncated: false,
                stderr_truncated: false,
            },
        };
        let json = serde_json::to_string(&response).unwrap();
        match serde_json::from_str::<IpcResponse>(&json).unwrap() {
            IpcResponse::ExecResult { result } => {
                assert_eq!(result.stdout, b"hi\n");
                assert_eq!(result.exit_code, Some(0));
                assert!(!result.timed_out);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn exec_grant_ipc_reports_durable_and_runtime_state() {
        use super::super::ipc_host::exec_grant_journal::{
            ExecGrantPersistResult, ExecGrantRetryState,
        };

        let target = crate::share::ExecGrantTarget::Direct {
            device_id: "device-a".into(),
            public_key: "key-a".into(),
            fingerprint: "fingerprint-a".into(),
            node_id: "key-a".into(),
        };
        let request = IpcRequest::MutateExecGrant {
            token: "token".into(),
            target: target.clone(),
            enabled: true,
        };
        let json = serde_json::to_string(&request).unwrap();
        match serde_json::from_str::<IpcRequest>(&json).unwrap() {
            IpcRequest::MutateExecGrant {
                token,
                target: decoded,
                enabled,
            } => {
                assert_eq!(token, "token");
                assert_eq!(decoded, target);
                assert!(enabled);
            }
            _ => panic!("wrong request"),
        }

        let response = IpcResponse::ExecGrantMutation {
            result: ExecGrantPersistResult {
                operation_id: "01".repeat(16),
                target,
                requested_enabled: true,
                persisted: true,
                applied: false,
                revision: 7,
                retry_state: ExecGrantRetryState::PendingApply,
                error: Some("retry".into()),
            },
        };
        let json = serde_json::to_string(&response).unwrap();
        match serde_json::from_str::<IpcResponse>(&json).unwrap() {
            IpcResponse::ExecGrantMutation { result } => {
                assert!(result.persisted);
                assert!(!result.applied);
                assert_eq!(result.revision, 7);
                assert_eq!(result.retry_state, ExecGrantRetryState::PendingApply);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }
}
