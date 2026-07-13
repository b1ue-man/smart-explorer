use super::*;

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
