//! B5 fixtures for the single remote batch suite; no external relay or account.
use super::*;
use iroh::endpoint::{ApplicationPathToken, ApplicationPathUpdate, Connection};

const WAIT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(150);

pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// Both endpoints have exactly one numeric loopback transport and no discovery.
pub(crate) async fn pair() -> (iroh::Endpoint, iroh::Endpoint, Connection, Connection) {
    async fn endpoint() -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup()
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .alpns(vec![b"smart-explorer/batches-transport-fixture/1".to_vec()])
            .transport_config(
                iroh::endpoint::QuicTransportConfig::builder()
                    .application_path_policy(true)
                    .build(),
            )
            .bind()
            .await
            .unwrap()
    }
    let left = endpoint().await;
    let right = endpoint().await;
    let sockets = right.bound_sockets();
    assert_eq!(sockets.len(), 1);
    assert!(sockets[0].ip().is_loopback());
    let address = iroh::EndpointAddr::from_parts(
        right.id(),
        sockets.into_iter().map(iroh::TransportAddr::Ip),
    );
    let (outgoing, incoming) = tokio::time::timeout(WAIT, async {
        tokio::join!(
            left.connect(address, b"smart-explorer/batches-transport-fixture/1"),
            async { right.accept().await.unwrap().await }
        )
    })
    .await
    .expect("isolated QUIC handshake deadline");
    let outgoing = outgoing.unwrap();
    let incoming = incoming.unwrap();
    assert_eq!(outgoing.remote_id(), right.id());
    assert_eq!(incoming.remote_id(), left.id());
    (left, right, outgoing, incoming)
}

pub(crate) async fn allow(connection: &Connection) -> ApplicationPathToken {
    tokio::time::timeout(WAIT, async {
        loop {
            for path in connection.application_path_policy().paths {
                if path.validated
                    && connection
                        .update_application_path_policy(ApplicationPathUpdate::Allow(path.token))
                        .is_ok()
                {
                    return path.token;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fresh on-path validation and admission deadline")
}

async fn no_payload(sender: &Connection, receiver: &Connection, tx: u64, rx: u64) {
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        sender.application_path_policy().application_tx_bytes,
        tx,
        "blocked STREAM/DATAGRAM must never reach the payload serializer"
    );
    assert_eq!(
        receiver.application_path_policy().application_rx_bytes,
        rx,
        "isolated peer must receive no application payload during the blocked epoch"
    );
}

#[test]
fn batches_task_b5_actual_stream_and_datagram_gate_survives_network_epoch() {
    runtime().block_on(async {
        let (left, right, sender, receiver) = pair().await;
        assert!(!sender.application_data_allowed());
        assert!(!receiver.application_data_allowed());
        // Successful transport authentication must not authorize buffered data.
        let mut stream = sender.open_uni().await.unwrap();
        stream.write_all(b"before").await.unwrap();
        sender
            .send_datagram(b"initial-datagram".to_vec().into())
            .unwrap();
        no_payload(&sender, &receiver, 0, 0).await;
        assert!(tokio::time::timeout(QUIET, receiver.accept_uni())
            .await
            .is_err());
        assert!(tokio::time::timeout(QUIET, receiver.read_datagram())
            .await
            .is_err());
        allow(&sender).await;
        let mut incoming = tokio::time::timeout(WAIT, receiver.accept_uni())
            .await
            .unwrap()
            .unwrap();
        let mut first = [0; 6];
        tokio::time::timeout(WAIT, incoming.read_exact(&mut first))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&first, b"before");
        assert_eq!(
            &tokio::time::timeout(WAIT, receiver.read_datagram())
                .await
                .unwrap()
                .unwrap()[..],
            b"initial-datagram"
        );

        // Noq's real network-change path revokes the old grant before the app
        // obtains its next generation. No synthetic RouteStatus drives this test.
        let old = sender.application_path_policy();
        let stale = old
            .paths
            .iter()
            .find(|path| path.data_allowed)
            .unwrap()
            .token;
        left.network_change().await;
        tokio::time::timeout(WAIT, async {
            while sender.application_path_policy().network_change_count == old.network_change_count
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("network invalidation reaches Noq");
        assert!(sender.application_path_policy().requires_new_generation);
        assert!(sender
            .update_application_path_policy(ApplicationPathUpdate::Allow(stale))
            .is_err());
        let blocked_tx = sender.application_path_policy().application_tx_bytes;
        let blocked_rx = receiver.application_path_policy().application_rx_bytes;
        stream.write_all(b"after!").await.unwrap();
        sender
            .send_datagram(b"next-datagram".to_vec().into())
            .unwrap();
        no_payload(&sender, &receiver, blocked_tx, blocked_rx).await;
        let mut next = [0; 6];
        assert!(tokio::time::timeout(QUIET, incoming.read(&mut next))
            .await
            .is_err());
        assert!(tokio::time::timeout(QUIET, receiver.read_datagram())
            .await
            .is_err());
        sender
            .update_application_path_policy(ApplicationPathUpdate::BeginGeneration(
                old.generation + 1,
            ))
            .unwrap();
        assert!(sender
            .update_application_path_policy(ApplicationPathUpdate::Allow(stale))
            .is_err());
        no_payload(&sender, &receiver, blocked_tx, blocked_rx).await;
        allow(&sender).await;
        tokio::time::timeout(WAIT, incoming.read_exact(&mut next))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&next, b"after!");
        assert_eq!(
            &tokio::time::timeout(WAIT, receiver.read_datagram())
                .await
                .unwrap()
                .unwrap()[..],
            b"next-datagram"
        );
        stream.finish().unwrap();
        assert_eq!(
            tokio::time::timeout(WAIT, incoming.read(&mut next))
                .await
                .unwrap()
                .unwrap(),
            None
        );
        assert!(sender.application_path_policy().application_tx_bytes > blocked_tx);
        assert!(
            !receiver.application_data_allowed(),
            "local admission cannot admit the remote sender"
        );

        // The opposite sender has its own gate, despite having received data.
        receiver.send_datagram(b"reverse".to_vec().into()).unwrap();
        assert!(tokio::time::timeout(QUIET, sender.read_datagram())
            .await
            .is_err());
        assert_eq!(receiver.application_path_policy().application_tx_bytes, 0);
        allow(&receiver).await;
        assert_eq!(
            &tokio::time::timeout(WAIT, sender.read_datagram())
                .await
                .unwrap()
                .unwrap()[..],
            b"reverse"
        );
        sender.close(0u32.into(), b"fixture complete");
        left.close().await;
        right.close().await;
    });
}

#[test]
fn batches_task_b5_route_cancellation_closes_blocked_transport_without_payload() {
    runtime().block_on(async {
        let (left, right, sender, receiver) = pair().await;
        let cancellation = crate::share::route_cancellation::RouteCancellation::default();
        cancellation.bind(&sender).unwrap();
        let mut stream = sender.open_uni().await.unwrap();
        stream.write_all(b"must-not-escape").await.unwrap();
        cancellation.cancel();
        tokio::time::timeout(WAIT, cancellation.cancelled())
            .await
            .unwrap();
        assert_eq!(
            cancellation.check().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        tokio::time::timeout(WAIT, receiver.closed())
            .await
            .expect("cancel closes blocked peer");
        assert_eq!(sender.application_path_policy().application_tx_bytes, 0);
        assert_eq!(receiver.application_path_policy().application_rx_bytes, 0);
        assert!(!sender.application_data_allowed());
        left.close().await;
        right.close().await;
    });
}

fn candidate(key: &str, class: RouteClass, validated: bool) -> RouteCandidate {
    RouteCandidate {
        key: key.into(),
        snapshot_key: key.into(),
        class,
        validated,
        validation_pending: !validated,
        rtt: Some(Duration::from_millis(1)),
    }
}

#[test]
fn batches_task_b5_frozen_deadline_relay_fallback_and_late_direct_lan_upgrade() {
    let mut policy = RoutePolicy::new(7, Duration::from_millis(10), Duration::from_millis(10));
    let mut paths = vec![
        candidate("lan", RouteClass::Lan, false),
        candidate("direct", RouteClass::Direct, false),
        candidate("relay", RouteClass::Relay, true),
    ];
    let mut round = RoundFacts {
        round: "initial".into(),
        candidates: vec!["lan".into(), "direct".into()],
        exhausted: false,
        response_deadline_ms: None,
        pending_path_opens: 1,
        retry_intervals: vec![Duration::from_millis(20)],
        max_probe_attempts: 2,
    };
    assert_eq!(policy.observe(0, &paths, Some(&round)), None);
    assert_eq!(policy.observe(30, &paths, Some(&round)), None);
    let deadline = policy.budget().direct_deadline_ms;
    assert_eq!(deadline, 110);
    // New hints, a new round and long retry intervals cannot prolong the attempt.
    round.round = "late-upgrade".into();
    round.retry_intervals = vec![Duration::from_secs(600)];
    paths.push(candidate("late", RouteClass::Direct, false));
    assert_eq!(policy.observe(deadline - 1, &paths, Some(&round)), None);
    assert_eq!(
        policy.observe(deadline, &paths, Some(&round)),
        Some("relay".into())
    );
    assert_eq!(policy.budget().direct_deadline_ms, deadline);
    assert_eq!(policy.budget().frozen_candidates, 2);
    paths[1].validated = true;
    assert_eq!(
        policy.observe(deadline + 1, &paths, Some(&round)),
        Some("direct".into())
    );
    paths[0].validated = true;
    paths[0].rtt = Some(Duration::from_secs(1));
    assert_eq!(
        policy.observe(deadline + 2, &paths, Some(&round)),
        Some("lan".into())
    );
    assert_eq!(policy.status().generation, 7);
    assert_eq!(policy.observe(deadline + 3, &[], None), None);
    assert_eq!(policy.status().phase, RoutePhase::Offline);
    assert_eq!(policy.status().pause_epoch, 1);
    assert_eq!(
        policy.observe(deadline + 4, &paths, None),
        Some("lan".into())
    );
}

#[test]
fn batches_task_b5_round_completion_waits_for_responses_validation_and_open_paths() {
    let mut policy = RoutePolicy::new(8, Duration::from_millis(10), Duration::from_millis(10));
    let paths = vec![candidate("relay", RouteClass::Relay, true)];
    let mut round = RoundFacts {
        round: "initial".into(),
        candidates: vec!["direct".into()],
        exhausted: true,
        response_deadline_ms: Some(50),
        pending_path_opens: 1,
        retry_intervals: vec![Duration::from_secs(1)],
        max_probe_attempts: 2,
    };
    policy.observe(0, &paths, Some(&round));
    assert_eq!(policy.observe(30, &paths, Some(&round)), None);
    assert_eq!(policy.observe(78, &paths, Some(&round)), None);
    round.pending_path_opens = 0;
    assert_eq!(policy.observe(79, &paths, Some(&round)), None);
    assert_eq!(
        policy.observe(80, &paths, Some(&round)),
        Some("relay".into())
    );
}
