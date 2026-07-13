use std::collections::BTreeMap;

use super::*;
use crate::share::core::public_fingerprint;
use crate::share::exec_client_active::ClientServerState;
use crate::share::exec_heartbeat::ExecHeartbeatPolicy;
use crate::share::exec_protocol::{
    recv_client_frame, recv_client_hello, send_hello_ok, send_server_frame, send_server_hello,
    ExecServerHello, ExecWireError, ServerFrame,
};
use crate::share::exec_types::{ExecCommand, ExecTerminalKind};
use crate::share::types::{PeerPresence, ShareScope};

fn identity(device_id: &str, name: &str, seed: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[seed; 32]);
    let node_id = secret.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: name.into(),
        direct_lookup_id: format!("lookup-{device_id}"),
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: secret,
        direct_secret: [seed; 32],
    }
}

fn fixture() -> (PeerEndpoint, ShareIdentity, ExecServerHello) {
    let target = identity("target", "Target", 7);
    let local = identity("local", "Local", 9);
    let endpoint = PeerEndpoint {
        label: "Target".into(),
        scope: ShareScope::Direct {
            contact_id: "contact".into(),
        },
        presence: PeerPresence {
            kind: "direct".into(),
            relation_id: "target-lookup".into(),
            device_id: target.device_id.clone(),
            device_name: target.device_name.clone(),
            public_key: target.public_key.clone(),
            fingerprint: target.fingerprint.clone(),
            node_id: target.node_id.clone(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: i64::MAX,
            nonce: String::new(),
            proof: String::new(),
        },
        relation_secret: b"relation-secret".to_vec(),
        expected_node_id: Some(target.node_id.clone()),
    };
    let server = ExecServerHello::new(
        "fresh-server-challenge-0123456789".into(),
        target.device_id,
        target.public_key,
        target.fingerprint,
        target.node_id,
    );
    (endpoint, local, server)
}

fn start(byte: &str) -> ExecStart {
    ExecStart {
        exec_id: ExecId::parse(byte.repeat(16)).unwrap(),
        command: ExecCommand::Argv {
            program: "program".into(),
            args: vec!["literal $ value".into()],
        },
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
    }
}

fn hello_ok(available: bool) -> ExecHelloOk {
    ExecHelloOk {
        authorization: ExecAuthorization {
            policy_revision: 3,
            authorization_epoch: 4,
            session_id: "session-012345678901234567890123".into(),
        },
        provider: ExecProviderStatus {
            available,
            provider: "test".into(),
            detail: if available { "ready" } else { "missing" }.into(),
            elevated: false,
            user_label: "tester".into(),
        },
    }
}

fn terminal(id: &ExecId) -> ExecTerminal {
    ExecTerminal {
        exec_id: id.clone(),
        kind: ExecTerminalKind::Exited,
        exit_code: Some(7),
        signal: None,
        message: None,
        stdout_bytes: 3,
        stderr_bytes: 2,
        output_truncated: false,
    }
}

#[test]
fn one_start_streams_raw_io_and_distinct_lifecycle_events() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let request = start("11");
        let exec_id = request.exec_id.clone();
        let (client, server) = tokio::io::duplex(256 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (input_tx, input_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(8);
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
            let _hello = recv_client_hello(&mut server_read).await.unwrap();
            send_hello_ok(&mut server_write, &hello_ok(true))
                .await
                .unwrap();
            match recv_client_frame(&mut server_read).await.unwrap() {
                ClientFrame::Start { start, digest } => {
                    assert_eq!(start.exec_id, exec_id);
                    assert_eq!(start.digest().unwrap(), digest);
                }
                frame => panic!("first operation was not Start: {frame:?}"),
            }
            send_server_frame(
                &mut server_write,
                &ServerFrame::Started {
                    exec_id: exec_id.clone(),
                },
            )
            .await
            .unwrap();
            match recv_client_frame(&mut server_read).await.unwrap() {
                ClientFrame::Stdin { data, .. } => assert_eq!(data, [0, 0xff, b'\n']),
                frame => panic!("missing raw stdin: {frame:?}"),
            }
            assert!(matches!(
                recv_client_frame(&mut server_read).await.unwrap(),
                ClientFrame::StdinEof { .. }
            ));
            send_server_frame(
                &mut server_write,
                &ServerFrame::Stdout {
                    exec_id: exec_id.clone(),
                    data: vec![0, 0xff, b'\n'],
                },
            )
            .await
            .unwrap();
            send_server_frame(
                &mut server_write,
                &ServerFrame::Stderr {
                    exec_id: exec_id.clone(),
                    data: b"e\0".to_vec(),
                },
            )
            .await
            .unwrap();
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
        input_tx
            .send(ExecClientInput::Stdin(vec![0, 0xff, b'\n']))
            .await
            .unwrap();
        input_tx.send(ExecClientInput::StdinEof).await.unwrap();
        let result = client_task.await.unwrap().unwrap();
        server_task.await.unwrap();
        assert_eq!(result.exit_code, Some(7));
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecClientEvent::Authorized { .. })
        ));
        assert_eq!(event_rx.recv().await, Some(ExecClientEvent::Started));
        assert_eq!(
            event_rx.recv().await,
            Some(ExecClientEvent::Stdout(vec![0, 0xff, b'\n']))
        );
        assert_eq!(
            event_rx.recv().await,
            Some(ExecClientEvent::Stderr(b"e\0".to_vec()))
        );
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecClientEvent::Terminal(_))
        ));
    });
}

#[test]
fn dropping_input_sends_cancel_after_the_single_start() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let request = start("22");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(4);
        drop(input_tx);
        let client_task = tokio::spawn(run_stream(
            client_write,
            client_read,
            endpoint,
            identity,
            request,
            input_rx,
            event_tx,
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
        assert!(matches!(
            recv_client_frame(&mut server_read).await.unwrap(),
            ClientFrame::Cancel { .. }
        ));
        let error = client_task.await.unwrap().unwrap_err();
        assert_eq!(error.kind, ExecClientFailureKind::Local);
        assert!(error.start_may_have_been_sent);
    });
}

#[test]
fn state_rejects_wrong_ids_order_and_preserves_remote_error_codes() {
    let id = ExecId::parse("33".repeat(16)).unwrap();
    let other = ExecId::parse("44".repeat(16)).unwrap();
    let mut state = ClientServerState::default();
    let error = state
        .accept(
            ServerFrame::Stdout {
                exec_id: id.clone(),
                data: vec![1],
            },
            &id,
            true,
        )
        .unwrap_err();
    assert_eq!(error.kind, ExecClientFailureKind::Protocol);
    assert!(state
        .accept(ServerFrame::Started { exec_id: other }, &id, true)
        .is_err());
    let remote = state
        .accept(
            ServerFrame::Error(ExecWireError {
                code: "exec_not_authorized".into(),
                message: "enable it on the target".into(),
            }),
            &id,
            true,
        )
        .unwrap_err();
    assert_eq!(remote.kind, ExecClientFailureKind::Remote);
    assert_eq!(remote.code, "exec_not_authorized");
    assert!(remote.start_may_have_been_sent);
}

#[test]
fn matching_cached_terminal_is_valid_without_another_started_frame() {
    let id = ExecId::parse("45".repeat(16)).unwrap();
    let mut state = ClientServerState::default();
    let event = state
        .accept(ServerFrame::Terminal(terminal(&id)), &id, true)
        .unwrap();
    assert!(matches!(event, ExecClientEvent::Terminal(_)));
    assert!(state
        .accept(ServerFrame::Terminal(terminal(&id)), &id, true)
        .is_err());
}

#[test]
fn unavailable_provider_fails_before_start_is_sent() {
    run(async {
        let (endpoint, identity, server_hello) = fixture();
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let (_input_tx, input_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_stream(
            client_write,
            client_read,
            endpoint,
            identity,
            start("55"),
            input_rx,
            event_tx,
        ));
        send_server_hello(&mut server_write, &server_hello)
            .await
            .unwrap();
        recv_client_hello(&mut server_read).await.unwrap();
        send_hello_ok(&mut server_write, &hello_ok(false))
            .await
            .unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code, "provider_unavailable");
        assert!(!error.start_may_have_been_sent);
        let observed = tokio::time::timeout(
            Duration::from_millis(50),
            recv_client_frame(&mut server_read),
        )
        .await;
        assert!(matches!(observed, Err(_) | Ok(Err(_))));
    });
}

#[path = "exec_client_heartbeat_integration_tests.rs"]
mod heartbeat_integration;

fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
