use super::*;
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

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
