use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::line::{read_line_limited_from_stream, MAX_IPC_LINE};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShareWorkerSnapshot {
    pub events: Vec<crate::share::ShareEvent>,
    /// Canonical daemon-owned profile state. Consumers must replace stale
    /// cached copies instead of replaying worker events as profile writes.
    #[serde(default)]
    pub profiles: crate::share::ShareProfiles,
    #[serde(default)]
    pub(crate) profile_revision: crate::share::ProfileRevision,
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
}

impl IpcRequest {
    pub(super) fn token(&self) -> &str {
        match self {
            Self::Ping { token }
            | Self::RefreshShare { token }
            | Self::ShareCommand { token, .. }
            | Self::DrainShareEvents { token }
            | Self::OpenShare { token, .. }
            | Self::ExecShare { token, .. } => token,
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
    Err {
        msg: String,
    },
}

pub(super) fn write_response(stream: &mut TcpStream, response: &IpcResponse) -> io::Result<()> {
    let mut line = serde_json::to_string(response).map_err(eio)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
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
    use super::{read_response, IpcRequest, IpcResponse, ShareWorkerSnapshot};
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
}
