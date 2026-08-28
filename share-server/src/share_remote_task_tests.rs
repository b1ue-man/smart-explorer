use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::discovery::{self, CAPABILITY};
use super::discovery_state::{unix_seconds, MAX_SERVER_LEASE};
use super::limits::SourceKey;
use super::protocol::{
    DiscoveryAdvertisement, DiscoveryKind, DiscoveryOfferRequest, DiscoveryOperation,
    DiscoveryRejectionClass, PairingCloseReason, PairingPacketKind,
};
use super::state::{register_client, State};
use super::{Out, Writer};

#[test]
fn share_remote_task_server_discovery_lease_renewal_and_complete_lifecycle() {
    let state = Arc::new(Mutex::new(State::default()));
    let (publisher_id, publisher, publisher_rx) = client(&state, 1, true);
    let (connector_id, connector, connector_rx) = client(&state, 2, true);

    let before_publish = Instant::now();
    discovery::publish(
        publisher_id,
        &publisher,
        offer("offer-complete", 600),
        &state,
    );
    let advertisement = published(recv(&publisher_rx));
    assert_eq!(advertisement.offer_id, "offer-complete");
    assert!(advertisement.expires_at <= unix_seconds().saturating_add(300));
    {
        let locked = state.lock().unwrap();
        let retained = locked
            .discovery_offers
            .get(&advertisement.discovery_id)
            .unwrap();
        assert!(retained.deadline > before_publish);
        assert!(retained.deadline <= Instant::now() + MAX_SERVER_LEASE);
    }

    discovery::publish(
        publisher_id,
        &publisher,
        offer("offer-complete", 30),
        &state,
    );
    let renewed = published(recv(&publisher_rx));
    assert_eq!(renewed.discovery_id, advertisement.discovery_id);
    assert_eq!(state.lock().unwrap().discovery_offers.len(), 1);

    discovery::list(connector_id, &connector, &state);
    match recv(&connector_rx) {
        Out::DiscoveryList { advertisements } => {
            assert_eq!(advertisements, vec![renewed.clone()]);
        }
        _ => panic!("connector did not receive the discovery list"),
    }

    discovery::start_pairing(
        connector_id,
        &connector,
        &renewed.discovery_id,
        "exchange-complete",
        "ke1".into(),
        &state,
    );
    assert_opened(recv(&connector_rx), "exchange-complete", &renewed.discovery_id);
    assert_started(
        recv(&publisher_rx),
        "exchange-complete",
        &renewed.discovery_id,
        "ke1",
    );

    route_packet(
        publisher_id,
        &publisher,
        &state,
        "exchange-complete",
        PairingPacketKind::OpaqueKe2,
        "ke2",
    );
    assert_packet(recv(&connector_rx), PairingPacketKind::OpaqueKe2, "ke2");
    route_packet(
        connector_id,
        &connector,
        &state,
        "exchange-complete",
        PairingPacketKind::OpaqueKe3Bundle,
        "ke3-bundle",
    );
    assert_packet(
        recv(&publisher_rx),
        PairingPacketKind::OpaqueKe3Bundle,
        "ke3-bundle",
    );
    route_packet(
        publisher_id,
        &publisher,
        &state,
        "exchange-complete",
        PairingPacketKind::PublisherBundle,
        "publisher-bundle",
    );
    assert_packet(
        recv(&connector_rx),
        PairingPacketKind::PublisherBundle,
        "publisher-bundle",
    );
    route_packet(
        connector_id,
        &connector,
        &state,
        "exchange-complete",
        PairingPacketKind::ConnectorCommit,
        "connector-commit",
    );
    assert_packet(
        recv(&publisher_rx),
        PairingPacketKind::ConnectorCommit,
        "connector-commit",
    );
    route_packet(
        publisher_id,
        &publisher,
        &state,
        "exchange-complete",
        PairingPacketKind::PublisherCommit,
        "publisher-commit",
    );
    assert_packet(
        recv(&connector_rx),
        PairingPacketKind::PublisherCommit,
        "publisher-commit",
    );
    assert_finished(
        recv(&connector_rx),
        "exchange-complete",
        PairingCloseReason::Completed,
    );
    assert_finished(
        recv(&publisher_rx),
        "exchange-complete",
        PairingCloseReason::Completed,
    );
    assert!(state.lock().unwrap().discovery_exchanges.is_empty());
}

#[test]
fn share_remote_task_server_discovery_rejections_cancel_and_expiry() {
    let state = Arc::new(Mutex::new(State::default()));
    let (unsupported_id, unsupported, unsupported_rx) = client(&state, 1, false);
    discovery::list(unsupported_id, &unsupported, &state);
    assert_rejected(
        recv(&unsupported_rx),
        DiscoveryOperation::ListDiscoveries,
        DiscoveryRejectionClass::Unsupported,
        None,
    );

    let (publisher_id, publisher, publisher_rx) = client(&state, 2, true);
    let (connector_id, connector, connector_rx) = client(&state, 3, true);
    let (outsider_id, outsider, outsider_rx) = client(&state, 4, true);
    discovery::publish(
        publisher_id,
        &publisher,
        offer("invalid-lease", 0),
        &state,
    );
    assert_rejected(
        recv(&publisher_rx),
        DiscoveryOperation::PublishDiscovery,
        DiscoveryRejectionClass::InvalidRequest,
        Some("invalid-lease"),
    );

    discovery::publish(
        publisher_id,
        &publisher,
        offer("offer-rejections", 300),
        &state,
    );
    let advertisement = published(recv(&publisher_rx));
    discovery::start_pairing(
        connector_id,
        &connector,
        &advertisement.discovery_id,
        "exchange-protocol",
        "ke1".into(),
        &state,
    );
    assert_opened(
        recv(&connector_rx),
        "exchange-protocol",
        &advertisement.discovery_id,
    );
    assert_started(
        recv(&publisher_rx),
        "exchange-protocol",
        &advertisement.discovery_id,
        "ke1",
    );
    route_packet(
        connector_id,
        &connector,
        &state,
        "exchange-protocol",
        PairingPacketKind::OpaqueKe2,
        "wrong-role",
    );
    assert_rejected(
        recv(&connector_rx),
        DiscoveryOperation::PairingPacket,
        DiscoveryRejectionClass::Protocol,
        Some("exchange-protocol"),
    );
    assert_finished(
        recv(&connector_rx),
        "exchange-protocol",
        PairingCloseReason::ProtocolError,
    );
    assert_finished(
        recv(&publisher_rx),
        "exchange-protocol",
        PairingCloseReason::ProtocolError,
    );

    start_exchange(
        &state,
        connector_id,
        &connector,
        &connector_rx,
        &publisher_rx,
        &advertisement,
        "exchange-cancel",
    );
    discovery::cancel_pairing(outsider_id, &outsider, "exchange-cancel", &state);
    assert_rejected(
        recv(&outsider_rx),
        DiscoveryOperation::CancelPairing,
        DiscoveryRejectionClass::Forbidden,
        Some("exchange-cancel"),
    );
    assert!(state
        .lock()
        .unwrap()
        .discovery_exchanges
        .contains_key("exchange-cancel"));
    discovery::cancel_pairing(connector_id, &connector, "exchange-cancel", &state);
    assert_finished(
        recv(&connector_rx),
        "exchange-cancel",
        PairingCloseReason::Cancelled,
    );
    assert_finished(
        recv(&publisher_rx),
        "exchange-cancel",
        PairingCloseReason::Cancelled,
    );

    start_exchange(
        &state,
        connector_id,
        &connector,
        &connector_rx,
        &publisher_rx,
        &advertisement,
        "exchange-expiry",
    );
    {
        let mut locked = state.lock().unwrap();
        locked
            .discovery_offers
            .get_mut(&advertisement.discovery_id)
            .unwrap()
            .deadline = Instant::now() - Duration::from_secs(1);
    }
    discovery::prune_expired(&state);
    assert_finished(
        recv(&connector_rx),
        "exchange-expiry",
        PairingCloseReason::OfferExpired,
    );
    assert_finished(
        recv(&publisher_rx),
        "exchange-expiry",
        PairingCloseReason::OfferExpired,
    );
    let locked = state.lock().unwrap();
    assert!(locked.discovery_offers.is_empty());
    assert!(locked.discovery_exchanges.is_empty());
}

fn client(
    state: &Arc<Mutex<State>>,
    address: u8,
    capability: bool,
) -> (u64, Writer, Receiver<Out>) {
    let (writer, receiver) = Writer::test_channel(64);
    let capabilities = if capability {
        HashSet::from([CAPABILITY.to_string()])
    } else {
        HashSet::new()
    };
    let source = SourceKey::from_socket(SocketAddr::from(([127, 0, 0, address], 5000)));
    let id = register_client(
        state,
        writer.clone(),
        source,
        format!("client-{address}"),
        capabilities,
    )
    .unwrap();
    (id, writer, receiver)
}

fn offer(offer_id: &str, lease_secs: u32) -> DiscoveryOfferRequest {
    DiscoveryOfferRequest {
        offer_id: offer_id.into(),
        kind: DiscoveryKind::Direct,
        display_alias: "Fixture Device".into(),
        suite: "se-discovery-opaque-r255-sha512-argon2id-chacha20poly1305".into(),
        version: 1,
        lease_secs,
    }
}

fn recv(receiver: &Receiver<Out>) -> Out {
    receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|error| panic!("expected server message: {error}"))
}

fn published(message: Out) -> DiscoveryAdvertisement {
    match message {
        Out::DiscoveryPublished { advertisement } => advertisement,
        _ => panic!("expected DiscoveryPublished"),
    }
}

fn start_exchange(
    state: &Arc<Mutex<State>>,
    connector_id: u64,
    connector: &Writer,
    connector_rx: &Receiver<Out>,
    publisher_rx: &Receiver<Out>,
    advertisement: &DiscoveryAdvertisement,
    exchange_id: &str,
) {
    discovery::start_pairing(
        connector_id,
        connector,
        &advertisement.discovery_id,
        exchange_id,
        "ke1".into(),
        state,
    );
    assert_opened(recv(connector_rx), exchange_id, &advertisement.discovery_id);
    assert_started(
        recv(publisher_rx),
        exchange_id,
        &advertisement.discovery_id,
        "ke1",
    );
}

fn route_packet(
    id: u64,
    writer: &Writer,
    state: &Arc<Mutex<State>>,
    exchange_id: &str,
    kind: PairingPacketKind,
    payload: &str,
) {
    discovery::pairing_packet(id, writer, exchange_id, kind, payload.into(), state);
}

fn assert_opened(message: Out, exchange_id: &str, discovery_id: &str) {
    match message {
        Out::PairingOpened {
            exchange_id: actual_exchange,
            discovery_id: actual_discovery,
        } => {
            assert_eq!(actual_exchange, exchange_id);
            assert_eq!(actual_discovery, discovery_id);
        }
        _ => panic!("expected PairingOpened"),
    }
}

fn assert_started(message: Out, exchange_id: &str, discovery_id: &str, payload: &str) {
    match message {
        Out::PairingStarted {
            exchange_id: actual_exchange,
            discovery_id: actual_discovery,
            payload: actual_payload,
        } => {
            assert_eq!(actual_exchange, exchange_id);
            assert_eq!(actual_discovery, discovery_id);
            assert_eq!(actual_payload, payload);
        }
        _ => panic!("expected PairingStarted"),
    }
}

fn assert_packet(message: Out, kind: PairingPacketKind, payload: &str) {
    match message {
        Out::PairingPacket {
            kind: actual_kind,
            payload: actual_payload,
            ..
        } => {
            assert_eq!(actual_kind, kind);
            assert_eq!(actual_payload, payload);
        }
        _ => panic!("expected PairingPacket"),
    }
}

fn assert_finished(message: Out, exchange_id: &str, reason: PairingCloseReason) {
    match message {
        Out::PairingFinished {
            exchange_id: actual_exchange,
            reason: actual_reason,
        } => {
            assert_eq!(actual_exchange, exchange_id);
            assert_eq!(actual_reason, reason);
        }
        _ => panic!("expected PairingFinished"),
    }
}

fn assert_rejected(
    message: Out,
    operation: DiscoveryOperation,
    classification: DiscoveryRejectionClass,
    correlation: Option<&str>,
) {
    match message {
        Out::DiscoveryRejected {
            operation: actual_operation,
            offer_id,
            exchange_id,
            classification: actual_classification,
            ..
        } => {
            assert_eq!(actual_operation, operation);
            assert_eq!(actual_classification, classification);
            let actual = offer_id.as_deref().or(exchange_id.as_deref());
            assert_eq!(actual, correlation);
        }
        _ => panic!("expected DiscoveryRejected"),
    }
}
