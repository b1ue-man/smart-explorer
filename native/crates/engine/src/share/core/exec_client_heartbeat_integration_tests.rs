use super::*;
use crate::share::exec_terminal::{ExecTerminalRequest, EXEC_TERMINAL_FEATURE};

#[test]
fn silent_authenticated_peer_hits_the_application_liveness_deadline() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let request = start("66");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let policy = ExecHeartbeatPolicy {
            interval: Duration::from_millis(20),
            peer_timeout: Duration::from_millis(80),
            write_timeout: Duration::from_millis(20),
        };
        let task = tokio::spawn(run_stream_with_policy(
            client_write,
            client_read,
            endpoint,
            identity,
            request,
            input_rx,
            event_tx,
            policy,
        ));
        send_server_hello(&mut server_write, &server_hello)
            .await
            .unwrap();
        recv_client_hello(&mut server_read).await.unwrap();
        send_hello_ok(&mut server_write, &hello_ok(true))
            .await
            .unwrap();
        let exec_id = match recv_client_frame(&mut server_read).await.unwrap() {
            ClientFrame::Start { start, .. } => start.exec_id,
            frame => panic!("first operation was not Start: {frame:?}"),
        };
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
        let error = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind, ExecClientFailureKind::Disconnected);
        assert_eq!(error.code, "peer_unresponsive");
        assert!(error.start_may_have_been_sent);
    });
}

#[test]
fn matching_pongs_keep_an_unlimited_silent_command_alive() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let request = start("77");
        let exec_id = request.exec_id.clone();
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(16);
        let policy = ExecHeartbeatPolicy {
            interval: Duration::from_millis(20),
            peer_timeout: Duration::from_millis(80),
            write_timeout: Duration::from_millis(20),
        };
        let task = tokio::spawn(run_stream_with_policy(
            client_write,
            client_read,
            endpoint,
            identity,
            request,
            input_rx,
            event_tx,
            policy,
        ));
        send_server_hello(&mut server_write, &server_hello)
            .await
            .unwrap();
        recv_client_hello(&mut server_read).await.unwrap();
        send_hello_ok(&mut server_write, &hello_ok(true))
            .await
            .unwrap();
        assert!(matches!(
            recv_client_frame(&mut server_read).await.unwrap(),
            ClientFrame::Start { .. }
        ));
        send_server_frame(
            &mut server_write,
            &ServerFrame::Started {
                exec_id: exec_id.clone(),
            },
        )
        .await
        .unwrap();
        for expected in 1..=6 {
            let sequence = match recv_client_frame(&mut server_read).await.unwrap() {
                ClientFrame::Ping {
                    exec_id: ping_id,
                    sequence,
                } => {
                    assert_eq!(ping_id, exec_id);
                    sequence
                }
                frame => panic!("expected heartbeat Ping, got {frame:?}"),
            };
            assert_eq!(sequence, expected);
            send_server_frame(
                &mut server_write,
                &ServerFrame::Pong {
                    exec_id: exec_id.clone(),
                    sequence,
                },
            )
            .await
            .unwrap();
        }
        send_server_frame(
            &mut server_write,
            &ServerFrame::Terminal(terminal(&exec_id)),
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
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.exit_code, Some(7));
    });
}

#[test]
fn delayed_terminal_acknowledgement_fits_the_shared_server_and_client_budgets() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let request = start("88");
        let exec_id = request.exec_id.clone();
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let policy = ExecHeartbeatPolicy {
            interval: Duration::from_millis(200),
            peer_timeout: Duration::from_millis(800),
            write_timeout: Duration::from_millis(200),
        };
        let legacy_client_boundary = policy
            .peer_timeout
            .saturating_add(policy.write_timeout.saturating_mul(2));
        let delayed_confirmation = policy
            .server_result_ack_timeout()
            .saturating_add(policy.write_timeout)
            .saturating_add(policy.interval / 2);
        assert!(delayed_confirmation > legacy_client_boundary);
        assert!(delayed_confirmation < policy.client_result_acknowledged_timeout());

        let task = tokio::spawn(run_stream_with_policy(
            client_write,
            client_read,
            endpoint,
            identity,
            request,
            input_rx,
            event_tx,
            policy,
        ));
        send_server_hello(&mut server_write, &server_hello)
            .await
            .unwrap();
        recv_client_hello(&mut server_read).await.unwrap();
        send_hello_ok(&mut server_write, &hello_ok(true))
            .await
            .unwrap();
        assert!(matches!(
            recv_client_frame(&mut server_read).await.unwrap(),
            ClientFrame::Start { .. }
        ));
        send_server_frame(
            &mut server_write,
            &ServerFrame::Terminal(terminal(&exec_id)),
        )
        .await
        .unwrap();
        assert_eq!(
            recv_client_frame(&mut server_read).await.unwrap(),
            ClientFrame::ResultAck {
                exec_id: exec_id.clone()
            }
        );

        tokio::time::sleep(delayed_confirmation).await;
        send_server_frame(
            &mut server_write,
            &ServerFrame::ResultAcknowledged {
                exec_id: exec_id.clone(),
            },
        )
        .await
        .unwrap();

        let result = tokio::time::timeout(Duration::from_millis(300), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.exec_id, exec_id);
    });
}

fn terminal_start(byte: &str) -> ExecStart {
    ExecStart {
        command: ExecCommand::Login,
        terminal: Some(ExecTerminalRequest {
            term: "xterm-256color".into(),
            size: ExecTerminalSize { cols: 80, rows: 24 },
        }),
        ..start(byte)
    }
}

#[test]
fn terminal_task_terminal_start_is_never_sent_to_a_host_without_the_feature() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_stream(
            client_write,
            client_read,
            endpoint,
            identity,
            terminal_start("66"),
            input_rx,
            event_tx,
        ));
        send_server_hello(&mut server_write, &server_hello)
            .await
            .unwrap();
        recv_client_hello(&mut server_read).await.unwrap();
        // A host that predates terminals sends no `features` at all.
        send_hello_ok(&mut server_write, &hello_ok(true))
            .await
            .unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code, "terminal_unsupported");
        assert_eq!(error.kind, ExecClientFailureKind::Remote);
        assert_eq!(
            error.message,
            "the peer's Smart Explorer has no terminal support; update it"
        );
        assert!(!error.start_may_have_been_sent);
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecClientEvent::Authorized { .. })
        ));
        let observed = tokio::time::timeout(
            Duration::from_millis(50),
            recv_client_frame(&mut server_read),
        )
        .await;
        assert!(matches!(observed, Err(_) | Ok(Err(_))));
    });
}

#[test]
fn terminal_task_terminal_start_and_resizes_reach_a_host_with_the_feature() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let request = terminal_start("77");
        let exec_id = request.exec_id.clone();
        let sent_request = request.clone();
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (input_tx, input_rx) = mpsc::channel(4);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let client_task = tokio::spawn(run_stream(
            client_write,
            client_read,
            endpoint,
            identity,
            request,
            input_rx,
            event_tx,
        ));
        let server_task = tokio::spawn(async move {
            send_server_hello(&mut server_write, &server_hello)
                .await
                .unwrap();
            recv_client_hello(&mut server_read).await.unwrap();
            let features = vec![EXEC_TERMINAL_FEATURE.to_string()];
            send_hello_ok(&mut server_write, &hello_with_features(true, features))
                .await
                .unwrap();
            match recv_client_frame(&mut server_read).await.unwrap() {
                ClientFrame::Start { start, digest } => {
                    assert_eq!(start, sent_request);
                    assert_eq!(start.digest().unwrap(), digest);
                }
                frame => panic!("first operation was not Start: {frame:?}"),
            }
            // The unusable size before it was dropped by the client.
            assert_eq!(
                recv_client_frame(&mut server_read).await.unwrap(),
                ClientFrame::Resize {
                    exec_id: exec_id.clone(),
                    size: ExecTerminalSize {
                        cols: 100,
                        rows: 30
                    },
                }
            );
            send_server_frame(
                &mut server_write,
                &ServerFrame::Terminal(terminal(&exec_id)),
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
                &ServerFrame::ResultAcknowledged { exec_id },
            )
            .await
            .unwrap();
        });
        let unusable = ExecTerminalSize { cols: 0, rows: 24 };
        input_tx
            .send(ExecClientInput::Resize(unusable))
            .await
            .unwrap();
        let size = ExecTerminalSize {
            cols: 100,
            rows: 30,
        };
        input_tx.send(ExecClientInput::Resize(size)).await.unwrap();
        let result = client_task.await.unwrap().unwrap();
        server_task.await.unwrap();
        assert_eq!(result.exit_code, Some(7));
    });
}

#[test]
fn terminal_task_resize_without_a_terminal_start_has_no_effect() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let request = start("88");
        let exec_id = request.exec_id.clone();
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (input_tx, input_rx) = mpsc::channel(4);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let client_task = tokio::spawn(run_stream(
            client_write,
            client_read,
            endpoint,
            identity,
            request,
            input_rx,
            event_tx,
        ));
        let server_task = tokio::spawn(async move {
            send_server_hello(&mut server_write, &server_hello)
                .await
                .unwrap();
            recv_client_hello(&mut server_read).await.unwrap();
            // Even a host with the feature gets no Resize for a plain start.
            let features = vec![EXEC_TERMINAL_FEATURE.to_string()];
            send_hello_ok(&mut server_write, &hello_with_features(true, features))
                .await
                .unwrap();
            match recv_client_frame(&mut server_read).await.unwrap() {
                ClientFrame::Start { start, .. } => assert_eq!(start.terminal, None),
                frame => panic!("first operation was not Start: {frame:?}"),
            }
            assert!(matches!(
                recv_client_frame(&mut server_read).await.unwrap(),
                ClientFrame::StdinEof { .. }
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
                &ServerFrame::ResultAcknowledged { exec_id },
            )
            .await
            .unwrap();
        });
        let size = ExecTerminalSize { cols: 80, rows: 24 };
        input_tx.send(ExecClientInput::Resize(size)).await.unwrap();
        input_tx.send(ExecClientInput::StdinEof).await.unwrap();
        client_task.await.unwrap().unwrap();
        server_task.await.unwrap();
    });
}
