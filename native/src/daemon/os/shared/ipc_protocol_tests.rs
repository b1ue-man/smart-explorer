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
    use super::super::ipc_host::exec_grant_journal::{ExecGrantPersistResult, ExecGrantRetryState};

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
