//! Exec streams of access rights (D.2.2): clients of 0.5.180 to 0.5.182
//! never see `APPROVAL_PENDING` and stay connected through a long
//! confirmation, and a client of this version works with an old host.
use super::*;

fn endpoint(host: &ShareIdentity) -> PeerEndpoint {
    PeerEndpoint {
        label: "Share Direkt: Target".into(),
        scope: ShareScope::Direct {
            contact_id: "contact".into(),
        },
        presence: PeerPresence {
            kind: "direct".into(),
            relation_id: host.direct_lookup_id.clone(),
            device_id: host.device_id.clone(),
            device_name: host.device_name.clone(),
            public_key: host.public_key.clone(),
            fingerprint: host.fingerprint.clone(),
            node_id: host.node_id.clone(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: i64::MAX,
            nonce: String::new(),
            proof: String::new(),
        },
        relation_secret: SECRET.to_vec(),
        expected_node_id: Some(host.node_id.clone()),
    }
}

/// The host side after it accepted `start`.
fn accepted(start: &ExecStart) -> ServerProtocolState {
    let mut protocol = ServerProtocolState::default();
    protocol
        .accept(&ClientFrame::Start {
            start: start.clone(),
            digest: start.digest().unwrap(),
        })
        .unwrap();
    protocol
}

fn exited(exec_id: &ExecId) -> ExecTerminal {
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

/// Ends a start the way a host does after `Terminal`: frames queued ahead
/// of `ResultAck` are drained, then the result is acknowledged.
async fn finish<W: tokio::io::AsyncWrite + Unpin>(
    send: &mut W,
    stream: &mut ClientStream,
    exec_id: &ExecId,
) {
    send_server_frame(send, &ServerFrame::Terminal(exited(exec_id)))
        .await
        .unwrap();
    loop {
        match stream.frames.next().await.unwrap().unwrap() {
            ClientFrame::ResultAck { .. } => break,
            ClientFrame::Ping { .. } => {}
            frame => panic!("unexpected frame after the result: {frame:?}"),
        }
    }
    send_server_frame(
        send,
        &ServerFrame::ResultAcknowledged {
            exec_id: exec_id.clone(),
        },
    )
    .await
    .unwrap();
}

#[test]
fn perm_task_old_client_no_pending_frame() {
    let policy = ExecHeartbeatPolicy {
        interval: Duration::from_millis(50),
        peer_timeout: Duration::from_millis(800),
        write_timeout: Duration::from_millis(400),
    };
    let request = start("ab", ExecCommand::Login);
    let exec_id = request.exec_id.clone();
    let pending = ExecApprovalPending::new(
        exec_id.clone(),
        Capability::ExecTerminal,
        ApprovalSurface::Window,
    );
    assert_eq!(
        serde_json::to_string(&pending).unwrap(),
        format!(r#"{{"exec_id":"{exec_id}","capability":"exec_terminal","surface":"window"}}"#)
    );
    for features in [Vec::new(), client_features()] {
        let frames = run(async {
            let (client, server) = tokio::io::duplex(64 * 1024);
            let (mut client_read, mut client_write) = tokio::io::split(client);
            let (server_read, mut server_write) = tokio::io::split(server);
            let mut protocol = accepted(&request);
            let notice = wants_approval_frames(&features).then(|| pending.clone());
            let server_id = exec_id.clone();
            let host = tokio::spawn(async move {
                let mut stream = ClientStream::new(client_frames(server_read));
                let decision = tokio::time::sleep(Duration::from_millis(300));
                let waited = wait_for_approval(
                    &mut server_write,
                    &mut stream,
                    &mut protocol,
                    &server_id,
                    notice,
                    decision,
                    policy,
                )
                .await
                .unwrap();
                assert!(matches!(waited, ApprovalWait::Decided(())));
            });
            for sequence in 1..=3u64 {
                let ping = ClientFrame::Ping {
                    exec_id: exec_id.clone(),
                    sequence,
                };
                send_client_frame(&mut client_write, &ping).await.unwrap();
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            host.await.unwrap();
            let mut frames = Vec::new();
            while let Ok(frame) = recv_server_frame(&mut client_read).await {
                frames.push(frame);
            }
            frames
        });
        let pongs: Vec<ServerFrame> = (1..=3u64)
            .map(|sequence| ServerFrame::Pong {
                exec_id: exec_id.clone(),
                sequence,
            })
            .collect();
        if features.is_empty() {
            // A client of 0.5.180 to 0.5.182 would fail on frame 39.
            assert_eq!(frames, pongs);
        } else {
            assert_eq!(frames[0], ServerFrame::ApprovalPending(pending.clone()));
            assert_eq!(frames[1..], pongs[..]);
        }
    }
}

#[test]
fn perm_task_old_client_survives_long_approval() {
    // Production heartbeats (ping every 5 s, 20 s without an answer ends the
    // session) and a decision after 45 s.
    let request = shell("ce");
    let exec_id = request.exec_id.clone();
    let (events, result) = run(async {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut protocol = accepted(&request);
        let server_id = exec_id.clone();
        let host = tokio::spawn(async move {
            let mut stream = ClientStream::new(client_frames(server_read));
            let decision = async {
                tokio::time::sleep(Duration::from_secs(45)).await;
                "once"
            };
            let waited = wait_for_approval(
                &mut server_write,
                &mut stream,
                &mut protocol,
                &server_id,
                None,
                decision,
                EXEC_HEARTBEAT_POLICY,
            )
            .await
            .unwrap();
            assert!(matches!(waited, ApprovalWait::Decided("once")));
            assert!(stream.last_sequence >= 8, "the client kept pinging");
            send_server_frame(
                &mut server_write,
                &ServerFrame::Started {
                    exec_id: server_id.clone(),
                },
            )
            .await
            .unwrap();
            protocol.terminal();
            finish(&mut server_write, &mut stream, &server_id).await;
        });
        // The client side: the active phase after `Start`, as 0.5.182 runs it.
        let (_input, input_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let result = exec_client_active::run(
            client_write,
            client_read,
            input_rx,
            event_tx,
            exec_id.clone(),
            false,
            EXEC_HEARTBEAT_POLICY,
            Box::pin(std::future::pending()),
        )
        .await;
        host.await.unwrap();
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }
        (events, result)
    });
    let terminal = result.unwrap();
    assert_eq!(terminal.kind, ExecTerminalKind::Exited);
    assert_eq!(events.first(), Some(&ExecClientEvent::Started));
    assert!(matches!(events.last(), Some(ExecClientEvent::Terminal(_))));
}

#[test]
fn perm_task_new_client_old_host() {
    let host = identity("target", "Target", 7);
    let client = identity("local", "Local", 9);
    let request = shell("cd");
    let exec_id = request.exec_id.clone();
    let server = server_hello(&host);
    let (events, result, notice) = run(async {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let (mut server_read, mut server_write) = tokio::io::split(server_stream);
        let host_hello = server.clone();
        let host_id = exec_id.clone();
        let old_host = tokio::spawn(async move {
            send_server_hello(&mut server_write, &host_hello)
                .await
                .unwrap();
            let hello = recv_client_hello(&mut server_read).await.unwrap();
            // The features stay outside the proof that a host of 0.5.182 checks.
            assert_eq!(hello.client_features, client_features());
            assert!(verify_hmac(
                SECRET,
                &transcript(&host_hello, &hello),
                &hello.proof
            ));
            // The `HELLO_OK` of 0.5.182: no access view, no access features.
            let old: ExecHelloOk = serde_json::from_str(
                r#"{"authorization":{"policy_revision":3,"authorization_epoch":4,"session_id":"session-0123456789"},"provider":{"available":true,"provider":"test","detail":"ready","elevated":false,"user_label":"tester"},"features":["exec_terminal_v1"]}"#,
            )
            .unwrap();
            send_hello_ok(&mut server_write, &old).await.unwrap();
            let start = match recv_client_frame(&mut server_read).await.unwrap() {
                ClientFrame::Start { start, .. } => start,
                frame => panic!("first operation was not Start: {frame:?}"),
            };
            assert_eq!(start.exec_id, host_id);
            let mut stream = ClientStream::new(client_frames(server_read));
            send_server_frame(
                &mut server_write,
                &ServerFrame::Started {
                    exec_id: host_id.clone(),
                },
            )
            .await
            .unwrap();
            finish(&mut server_write, &mut stream, &host_id).await;
        });
        let (_input, input_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (notice_tx, notice_rx) = watch::channel(ExecClientNotice::default());
        let result = run_stream_with_transport(
            client_write,
            client_read,
            endpoint(&host),
            client,
            request,
            input_rx,
            event_tx,
            EXEC_HEARTBEAT_POLICY,
            Box::pin(std::future::pending()),
            &notice_tx,
        )
        .await;
        old_host.await.unwrap();
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }
        let notice = notice_rx.borrow().clone();
        (events, result, notice)
    });
    assert_eq!(result.unwrap().kind, ExecTerminalKind::Exited);
    assert!(matches!(
        events.first(),
        Some(ExecClientEvent::Authorized { .. })
    ));
    assert!(events.contains(&ExecClientEvent::Started));
    assert!(notice.access.is_none() && notice.approval.is_none());
    assert_eq!(
        account_line("Target", notice.access.as_ref(), "tester"),
        "Target: account tester (older host, rights not reported)"
    );
}
