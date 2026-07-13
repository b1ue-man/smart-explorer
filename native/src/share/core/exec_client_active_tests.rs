use std::future::pending;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::AsyncWrite;
use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::share::exec_protocol::{recv_client_frame, send_server_frame};
use crate::share::exec_types::ExecTerminalKind;

fn id(byte: &str) -> ExecId {
    ExecId::parse(byte.repeat(16)).unwrap()
}

fn policy() -> ExecHeartbeatPolicy {
    ExecHeartbeatPolicy {
        interval: Duration::from_millis(20),
        peer_timeout: Duration::from_millis(80),
        write_timeout: Duration::from_millis(20),
    }
}

fn terminal(exec_id: &ExecId) -> ExecTerminal {
    ExecTerminal {
        exec_id: exec_id.clone(),
        kind: ExecTerminalKind::Exited,
        exit_code: Some(0),
        signal: None,
        message: None,
        stdout_bytes: 0,
        stderr_bytes: 0,
        output_truncated: false,
    }
}

#[test]
fn result_handshake_has_slack_for_both_bounded_writes() {
    assert_eq!(
        policy().server_result_ack_timeout(),
        Duration::from_millis(120)
    );
    assert_eq!(
        policy().client_result_acknowledged_timeout(),
        Duration::from_millis(160)
    );
}

#[test]
fn wrong_exec_id_and_sequence_pongs_are_rejected() {
    runtime().block_on(async {
        for (pong_id, pong_sequence, expected_code) in [
            (id("bb"), 1, "heartbeat_id"),
            (id("aa"), 2, "heartbeat_sequence"),
        ] {
            let exec_id = id("aa");
            let (client, server) = tokio::io::duplex(4096);
            let (client_read, client_write) = tokio::io::split(client);
            let (mut server_read, mut server_write) = tokio::io::split(server);
            let (_input_tx, input_rx) = mpsc::channel(1);
            let (event_tx, _event_rx) = mpsc::channel(8);
            let task = tokio::spawn(run(
                client_write,
                client_read,
                input_rx,
                event_tx,
                exec_id.clone(),
                policy(),
                Box::pin(pending()),
            ));

            send_server_frame(
                &mut server_write,
                &ServerFrame::Started {
                    exec_id: exec_id.clone(),
                },
            )
            .await
            .unwrap();
            assert!(matches!(
                recv_client_frame(&mut server_read).await.unwrap(),
                ClientFrame::Ping { sequence: 1, .. }
            ));
            send_server_frame(
                &mut server_write,
                &ServerFrame::Pong {
                    exec_id: pong_id,
                    sequence: pong_sequence,
                },
            )
            .await
            .unwrap();

            let error = tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(error.kind, ExecClientFailureKind::Protocol);
            assert_eq!(error.code, expected_code);
        }
    });
}

#[test]
fn remote_terminal_error_is_acknowledged_before_it_is_returned() {
    runtime().block_on(async {
        let exec_id = id("cc");
        let (client, server) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let task = tokio::spawn(run(
            client_write,
            client_read,
            input_rx,
            event_tx,
            exec_id.clone(),
            policy(),
            Box::pin(pending()),
        ));

        send_server_frame(
            &mut server_write,
            &ServerFrame::Error(ExecWireError {
                code: "already_running".into(),
                message: "duplicate".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            recv_client_frame(&mut server_read).await.unwrap(),
            ClientFrame::ResultAck {
                exec_id: exec_id.clone()
            }
        );
        send_server_frame(
            &mut server_write,
            &ServerFrame::ResultAcknowledged {
                exec_id: exec_id.clone(),
            },
        )
        .await
        .unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.kind, ExecClientFailureKind::Remote);
        assert_eq!(error.code, "already_running");
    });
}

#[test]
fn missing_or_wrong_terminal_acknowledgement_never_reports_success() {
    runtime().block_on(async {
        let exec_id = id("12");
        let (client, server) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let task = tokio::spawn(run(
            client_write,
            client_read,
            input_rx,
            event_tx,
            exec_id.clone(),
            policy(),
            Box::pin(pending()),
        ));
        send_server_frame(
            &mut server_write,
            &ServerFrame::Terminal(terminal(&exec_id)),
        )
        .await
        .unwrap();
        assert!(matches!(
            recv_client_frame(&mut server_read).await.unwrap(),
            ClientFrame::ResultAck { .. }
        ));
        send_server_frame(
            &mut server_write,
            &ServerFrame::ResultAcknowledged { exec_id: id("34") },
        )
        .await
        .unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.kind, ExecClientFailureKind::Protocol);
        assert_eq!(error.code, "result_acknowledged_id");

        let exec_id = id("56");
        let (client, server) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let task = tokio::spawn(run(
            client_write,
            client_read,
            input_rx,
            event_tx,
            exec_id.clone(),
            policy(),
            Box::pin(pending()),
        ));
        send_server_frame(
            &mut server_write,
            &ServerFrame::Terminal(terminal(&exec_id)),
        )
        .await
        .unwrap();
        assert!(matches!(
            recv_client_frame(&mut server_read).await.unwrap(),
            ClientFrame::ResultAck { .. }
        ));
        let error = tokio::time::timeout(Duration::from_millis(300), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind, ExecClientFailureKind::Disconnected);
        assert_eq!(error.code, "result_acknowledged_timeout");
    });
}

#[test]
fn transport_close_requires_ack_and_a_buffered_ack_wins_the_close_race() {
    runtime().block_on(async {
        let exec_id = id("78");
        let (mut server_write, client_read) = tokio::io::duplex(4096);
        send_server_frame(
            &mut server_write,
            &ServerFrame::Terminal(terminal(&exec_id)),
        )
        .await
        .unwrap();
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let (close_tx, close_rx) = oneshot::channel();
        close_tx.send(()).unwrap();
        let error = run(
            tokio::io::sink(),
            client_read,
            input_rx,
            event_tx,
            exec_id,
            policy(),
            Box::pin(async move {
                let _ = close_rx.await;
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, ExecClientFailureKind::Disconnected);

        let exec_id = id("9a");
        let (client, server) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let (close_tx, close_rx) = oneshot::channel();
        let task = tokio::spawn(run(
            client_write,
            client_read,
            input_rx,
            event_tx,
            exec_id.clone(),
            policy(),
            Box::pin(async move {
                let _ = close_rx.await;
            }),
        ));
        send_server_frame(
            &mut server_write,
            &ServerFrame::Terminal(terminal(&exec_id)),
        )
        .await
        .unwrap();
        assert!(matches!(
            recv_client_frame(&mut server_read).await.unwrap(),
            ClientFrame::ResultAck { .. }
        ));
        send_server_frame(
            &mut server_write,
            &ServerFrame::ResultAcknowledged {
                exec_id: exec_id.clone(),
            },
        )
        .await
        .unwrap();
        close_tx.send(()).unwrap();
        let result = task.await.unwrap().unwrap();
        assert_eq!(result.exec_id, exec_id);
    });
}

#[test]
fn output_frames_do_not_extend_the_authenticated_peer_deadline() {
    runtime().block_on(async {
        let exec_id = id("dd");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (_server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(128);
        let started = tokio::time::Instant::now();
        let task = tokio::spawn(run(
            client_write,
            client_read,
            input_rx,
            event_tx,
            exec_id.clone(),
            policy(),
            Box::pin(pending()),
        ));
        send_server_frame(
            &mut server_write,
            &ServerFrame::Started {
                exec_id: exec_id.clone(),
            },
        )
        .await
        .unwrap();
        let flood = tokio::spawn(async move {
            for _ in 0..40 {
                if send_server_frame(
                    &mut server_write,
                    &ServerFrame::Stdout {
                        exec_id: exec_id.clone(),
                        data: vec![1],
                    },
                )
                .await
                .is_err()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(4)).await;
            }
        });

        let error = tokio::time::timeout(Duration::from_millis(300), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "peer_unresponsive");
        assert!(started.elapsed() < Duration::from_millis(200));
        flood.abort();
    });
}

#[test]
fn transport_close_and_blocked_writes_are_bounded() {
    runtime().block_on(async {
        let exec_id = id("ee");
        let (mut server_write, client_read) = tokio::io::duplex(4096);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let (close_tx, close_rx) = oneshot::channel();
        let task = tokio::spawn(run(
            tokio::io::sink(),
            client_read,
            input_rx,
            event_tx,
            exec_id.clone(),
            policy(),
            Box::pin(async move {
                let _ = close_rx.await;
            }),
        ));
        send_server_frame(&mut server_write, &ServerFrame::Started { exec_id })
            .await
            .unwrap();
        close_tx.send(()).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind, ExecClientFailureKind::Disconnected);
        assert_eq!(error.code, "disconnected");

        let exec_id = id("ff");
        let (mut server_write, client_read) = tokio::io::duplex(4096);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let started = tokio::time::Instant::now();
        let task = tokio::spawn(run(
            PendingWriter,
            client_read,
            input_rx,
            event_tx,
            exec_id.clone(),
            policy(),
            Box::pin(pending()),
        ));
        send_server_frame(&mut server_write, &ServerFrame::Started { exec_id })
            .await
            .unwrap();
        let error = tokio::time::timeout(Duration::from_millis(300), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind, ExecClientFailureKind::Disconnected);
        assert_eq!(error.code, "write_timeout");
        assert!(started.elapsed() < Duration::from_millis(150));
    });
}

struct PendingWriter;

impl AsyncWrite for PendingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        _buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Pending
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
