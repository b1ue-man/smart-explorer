use std::collections::BTreeMap;

use super::*;
use crate::share::exec_heartbeat::ExecHeartbeatPolicy;
use crate::share::exec_protocol::{
    recv_server_frame, send_client_frame, send_server_frame, ClientFrame, ServerFrame,
};
use crate::share::exec_types::{ExecCommand, ExecId, ExecTerminal, ExecTerminalKind};

fn id(byte: &str) -> ExecId {
    ExecId::parse(byte.repeat(16)).unwrap()
}

#[test]
fn terminal_ack_drains_in_flight_frames_and_rejects_other_exec_ids() {
    let exec_id = id("11");
    let start = ExecStart {
        exec_id: exec_id.clone(),
        command: ExecCommand::Shell {
            command: "printf x".into(),
        },
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
    };
    let mut protocol = ServerProtocolState::default();
    protocol
        .accept(&ClientFrame::Start {
            digest: start.digest().unwrap(),
            start,
        })
        .unwrap();
    protocol.terminal();

    assert!(!validate_result_ack_frame(
        &mut protocol,
        &exec_id,
        ClientFrame::Stdin {
            exec_id: exec_id.clone(),
            data: vec![1],
        },
    )
    .unwrap());
    assert!(!validate_result_ack_frame(
        &mut protocol,
        &exec_id,
        ClientFrame::Cancel {
            exec_id: exec_id.clone(),
        },
    )
    .unwrap());
    assert!(validate_result_ack_frame(
        &mut protocol,
        &exec_id,
        ClientFrame::ResultAck {
            exec_id: exec_id.clone(),
        },
    )
    .unwrap());
    assert!(validate_result_ack_frame(
        &mut protocol,
        &exec_id,
        ClientFrame::ResultAck { exec_id: id("22") },
    )
    .is_err());
}

#[test]
fn handshake_io_enforces_one_absolute_deadline_for_a_stalled_peer() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
        let error = io_deadline::run_until(
            deadline,
            "stalled handshake",
            std::future::pending::<io::Result<()>>(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "stalled handshake");
    });
}

#[test]
fn delayed_terminal_ack_uses_the_server_budget_and_strict_slack() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let exec_id = id("33");
        let policy = ExecHeartbeatPolicy {
            interval: Duration::from_millis(200),
            peer_timeout: Duration::from_millis(800),
            write_timeout: Duration::from_millis(200),
        };
        let old_server_boundary = policy.peer_timeout.saturating_add(policy.write_timeout);
        let delayed_ack = old_server_boundary.saturating_add(policy.interval / 2);
        assert!(delayed_ack > old_server_boundary);
        assert!(delayed_ack < policy.server_result_ack_timeout());

        let (client, server) = tokio::io::duplex(4096);
        let (mut client_read, mut client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let server_id = exec_id.clone();
        let server_task = tokio::spawn(async move {
            let mut protocol = ServerProtocolState::default();
            protocol.terminal();
            send_server_frame(
                &mut server_write,
                &ServerFrame::Terminal(ExecTerminal {
                    exec_id: server_id.clone(),
                    kind: ExecTerminalKind::Exited,
                    exit_code: Some(0),
                    signal: None,
                    message: None,
                    stdout_bytes: 0,
                    stderr_bytes: 0,
                    output_truncated: false,
                }),
            )
            .await?;
            wait_result_ack_with_policy(&mut server_read, &mut protocol, &server_id, policy)
                .await?;
            send_server_frame(
                &mut server_write,
                &ServerFrame::ResultAcknowledged { exec_id: server_id },
            )
            .await
        });

        assert!(matches!(
            recv_server_frame(&mut client_read).await.unwrap(),
            ServerFrame::Terminal(_)
        ));
        tokio::time::sleep(delayed_ack).await;
        send_client_frame(
            &mut client_write,
            &ClientFrame::ResultAck {
                exec_id: exec_id.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            recv_server_frame(&mut client_read).await.unwrap(),
            ServerFrame::ResultAcknowledged { exec_id }
        );
        server_task.await.unwrap().unwrap();
    });
}
