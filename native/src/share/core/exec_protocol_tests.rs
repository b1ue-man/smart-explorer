use super::*;
use crate::share::exec_terminal::ExecTerminalRequest;
use crate::share::exec_types::{ExecCommand, ExecTerminalKind};
use std::collections::BTreeMap;

fn id() -> ExecId {
    ExecId::parse("ab".repeat(16)).unwrap()
}

fn start() -> ExecStart {
    ExecStart {
        exec_id: id(),
        command: ExecCommand::Shell {
            command: "printf x".into(),
        },
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: None,
    }
}

#[test]
fn binary_chunks_round_trip_without_base64_or_utf8_assumptions() {
    run(async {
        let frame = ServerFrame::Stdout {
            exec_id: id(),
            data: vec![0, 0xff, b'\n'],
        };
        let (mut tx, mut rx) = tokio::io::duplex(256);
        send_server_frame(&mut tx, &frame).await.unwrap();
        assert_eq!(recv_server_frame(&mut rx).await.unwrap(), frame);
    });
}

#[test]
fn authenticated_heartbeat_frames_round_trip_and_preserve_sequence() {
    run(async {
        let ping = ClientFrame::Ping {
            exec_id: id(),
            sequence: 7,
        };
        let pong = ServerFrame::Pong {
            exec_id: id(),
            sequence: 7,
        };
        let (mut client_tx, mut server_rx) = tokio::io::duplex(512);
        send_client_frame(&mut client_tx, &ping).await.unwrap();
        assert_eq!(recv_client_frame(&mut server_rx).await.unwrap(), ping);
        let (mut server_tx, mut client_rx) = tokio::io::duplex(512);
        send_server_frame(&mut server_tx, &pong).await.unwrap();
        assert_eq!(recv_server_frame(&mut client_rx).await.unwrap(), pong);
    });
}

#[test]
fn terminal_result_ack_round_trips_with_the_exact_exec_id() {
    run(async {
        let ack = ClientFrame::ResultAck { exec_id: id() };
        let (mut client_tx, mut server_rx) = tokio::io::duplex(256);
        send_client_frame(&mut client_tx, &ack).await.unwrap();
        assert_eq!(recv_client_frame(&mut server_rx).await.unwrap(), ack);

        let acknowledged = ServerFrame::ResultAcknowledged { exec_id: id() };
        let (mut server_tx, mut client_rx) = tokio::io::duplex(256);
        send_server_frame(&mut server_tx, &acknowledged)
            .await
            .unwrap();
        assert_eq!(
            recv_server_frame(&mut client_rx).await.unwrap(),
            acknowledged
        );
    });
}

#[test]
fn oversized_chunk_is_rejected_before_allocation_or_write() {
    run(async {
        let (mut tx, _rx) = tokio::io::duplex(8);
        let error = send_client_frame(
            &mut tx,
            &ClientFrame::Stdin {
                exec_id: id(),
                data: vec![0; MAX_EXEC_DATA_BYTES + 1],
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    });
}

#[test]
fn server_state_rejects_reordered_or_digest_mismatched_frames() {
    let mut state = ServerProtocolState::default();
    assert!(state
        .accept(&ClientFrame::Ping {
            exec_id: id(),
            sequence: 1,
        })
        .is_err());
    assert!(state
        .accept(&ClientFrame::Stdin {
            exec_id: id(),
            data: vec![1]
        })
        .is_err());
    let request = start();
    assert!(state
        .accept(&ClientFrame::Start {
            start: request.clone(),
            digest: "wrong".into()
        })
        .is_err());
    state
        .accept(&ClientFrame::Start {
            digest: request.digest().unwrap(),
            start: request,
        })
        .unwrap();
    state
        .accept(&ClientFrame::Ping {
            exec_id: id(),
            sequence: 1,
        })
        .unwrap();
    state
        .accept(&ClientFrame::StdinEof { exec_id: id() })
        .unwrap();
    state
        .accept(&ClientFrame::Ping {
            exec_id: id(),
            sequence: 2,
        })
        .unwrap();
    assert!(state
        .accept(&ClientFrame::ResultAck { exec_id: id() })
        .is_err());
    assert!(state
        .accept(&ClientFrame::Stdin {
            exec_id: id(),
            data: vec![1]
        })
        .is_err());
    state.terminal();
    state
        .accept(&ClientFrame::Stdin {
            exec_id: id(),
            data: vec![2],
        })
        .unwrap();
    state
        .accept(&ClientFrame::StdinEof { exec_id: id() })
        .unwrap();
    state
        .accept(&ClientFrame::Cancel { exec_id: id() })
        .unwrap();
    state
        .accept(&ClientFrame::Ping {
            exec_id: id(),
            sequence: 3,
        })
        .unwrap();
    state
        .accept(&ClientFrame::ResultAck { exec_id: id() })
        .unwrap();
}

#[test]
fn terminal_status_round_trips_distinctly() {
    run(async {
        let frame = ServerFrame::Terminal(ExecTerminal {
            exec_id: id(),
            kind: ExecTerminalKind::TimedOut,
            exit_code: None,
            signal: None,
            message: None,
            stdout_bytes: 1,
            stderr_bytes: 2,
            output_truncated: false,
        });
        let (mut tx, mut rx) = tokio::io::duplex(512);
        send_server_frame(&mut tx, &frame).await.unwrap();
        assert_eq!(recv_server_frame(&mut rx).await.unwrap(), frame);
    });
}

fn terminal_start() -> ExecStart {
    ExecStart {
        command: ExecCommand::Login,
        terminal: Some(ExecTerminalRequest {
            term: "xterm-256color".into(),
            size: ExecTerminalSize { cols: 80, rows: 24 },
        }),
        ..start()
    }
}

fn accept_start(state: &mut ServerProtocolState, request: ExecStart) {
    state
        .accept(&ClientFrame::Start {
            digest: request.digest().unwrap(),
            start: request,
        })
        .unwrap();
}

fn resize(cols: u16, rows: u16) -> ClientFrame {
    ClientFrame::Resize {
        exec_id: id(),
        size: ExecTerminalSize { cols, rows },
    }
}

#[test]
fn terminal_task_resize_frame_round_trips_with_tag_22_within_256_bytes() {
    run(async {
        let frame = resize(32767, 1);
        let mut bytes = Vec::new();
        send_client_frame(&mut bytes, &frame).await.unwrap();
        assert_eq!(bytes[0], 22);
        let length = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
        assert_eq!(length, bytes.len() - 5);
        assert!(length <= 256);
        assert_eq!(
            recv_client_frame(&mut bytes.as_slice()).await.unwrap(),
            frame
        );

        let mut unusable = Vec::new();
        let error = send_client_frame(&mut unusable, &resize(0, 24))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(unusable.is_empty());

        let padded = format!(
            "[\"{}\",{{\"cols\":80,\"rows\":24}}{}]",
            id(),
            " ".repeat(300)
        );
        let mut oversized = Vec::new();
        write_payload(&mut oversized, RESIZE, padded.as_bytes())
            .await
            .unwrap();
        let error = recv_client_frame(&mut oversized.as_slice())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    });
}

#[test]
fn terminal_task_resize_is_refused_without_a_terminal_start() {
    let mut state = ServerProtocolState::default();
    assert!(state.accept(&resize(80, 24)).is_err());
    accept_start(&mut state, start());
    assert!(state.accept(&resize(80, 24)).is_err());
    state
        .accept(&ClientFrame::StdinEof { exec_id: id() })
        .unwrap();
    assert!(state.accept(&resize(80, 24)).is_err());
    state.terminal();
    assert!(state.accept(&resize(80, 24)).is_err());
    state
        .accept(&ClientFrame::ResultAck { exec_id: id() })
        .unwrap();
}

#[test]
fn terminal_task_resize_is_accepted_for_a_terminal_start_until_the_result() {
    let mut state = ServerProtocolState::default();
    assert!(state.accept(&resize(80, 24)).is_err());
    accept_start(&mut state, terminal_start());
    state.accept(&resize(100, 30)).unwrap();
    assert!(state.accept(&resize(0, 30)).is_err());
    assert!(state.accept(&resize(100, 32768)).is_err());
    state.accept(&resize(1, 1)).unwrap();
    state
        .accept(&ClientFrame::StdinEof { exec_id: id() })
        .unwrap();
    state.accept(&resize(101, 31)).unwrap();
    state
        .accept(&ClientFrame::Cancel { exec_id: id() })
        .unwrap();
    state.accept(&resize(102, 32)).unwrap();
    // After the terminal result the frame is drained like queued input.
    state.terminal();
    state.accept(&resize(103, 33)).unwrap();
    assert!(state.accept(&resize(0, 33)).is_err());
    state
        .accept(&ClientFrame::ResultAck { exec_id: id() })
        .unwrap();
}

#[test]
fn terminal_task_server_state_refuses_a_login_start_without_terminal() {
    let mut state = ServerProtocolState::default();
    let mut request = terminal_start();
    request.terminal = None;
    assert!(state
        .accept(&ClientFrame::Start {
            start: request,
            digest: "unchecked".into()
        })
        .is_err());
    let mut state = ServerProtocolState::default();
    let mut request = start();
    request.terminal = terminal_start().terminal;
    assert!(state
        .accept(&ClientFrame::Start {
            start: request,
            digest: "unchecked".into()
        })
        .is_err());
}

#[test]
fn terminal_task_hello_ok_without_features_decodes_and_stays_byte_equal() {
    let old = r#"{"authorization":{"policy_revision":1,"authorization_epoch":2,"session_id":"s"},"provider":{"available":true,"provider":"p","detail":"d","elevated":false,"user_label":"u"}}"#;
    let decoded: ExecHelloOk = serde_json::from_str(old).unwrap();
    assert!(decoded.features.is_empty());
    assert_eq!(serde_json::to_string(&decoded).unwrap(), old);

    let announced = ExecHelloOk {
        features: vec!["exec_terminal_v1".into()],
        ..decoded
    };
    let encoded = serde_json::to_string(&announced).unwrap();
    assert!(encoded.ends_with(r#","features":["exec_terminal_v1"]}"#));
    assert_eq!(
        serde_json::from_str::<ExecHelloOk>(&encoded).unwrap(),
        announced
    );
}

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
