use super::DiscoveryExchangePortImpl;
use crate::share::direct_protocol::DirectPeerIdentity;
use crate::share::direct_reciprocal::{DirectReciprocalPeer, DirectRelationMaterial};
use crate::share::discovery_relation_store::{DiscoveryRelationOutcome, InMemoryRelationStore};
use crate::share::discovery_signal_port::{
    DiscoveryExchangePort, DiscoveryPortAction, DiscoveryPortError,
};
use crate::share::discovery_signal_types::{
    DiscoveryAdvertisement, DiscoveryKind, DiscoveryPublishTarget, PairingCloseReason,
    PairingPacketKind, DISCOVERY_PAIRING_SUITE, DISCOVERY_PAIRING_VERSION,
};

fn port(seed: u8, device: &'static str) -> DiscoveryExchangePortImpl {
    let source = move || {
        let key = iroh::SecretKey::from_bytes(&[seed; 32]);
        let identity = DirectPeerIdentity::from_secret(device, device, &key);
        let material =
            DirectRelationMaterial::new(format!("lookup-{device}"), vec![seed; 32]).unwrap();
        DirectReciprocalPeer::authenticated(identity, material).map_err(|error| error.to_string())
    };
    DiscoveryExchangePortImpl::new(Box::new(source), Box::new(InMemoryRelationStore::default()))
}

fn advertisement() -> DiscoveryAdvertisement {
    DiscoveryAdvertisement {
        discovery_id: "discovery".into(),
        offer_id: "offer".into(),
        kind: DiscoveryKind::Direct,
        display_alias: "Publisher".into(),
        suite: DISCOVERY_PAIRING_SUITE.into(),
        version: DISCOVERY_PAIRING_VERSION,
        expires_at: i64::MAX,
    }
}

fn packet(action: DiscoveryPortAction, expected: PairingPacketKind) -> Vec<u8> {
    match action {
        DiscoveryPortAction::StartPairing { .. } => panic!("unexpected repeated pairing start"),
        DiscoveryPortAction::SendPacket(packet) => {
            assert_eq!(packet.kind, expected);
            packet.payload
        }
        DiscoveryPortAction::PersistedAndSend(persisted) => {
            let (_, packet) = persisted.into_parts();
            assert_eq!(packet.kind, expected);
            packet.payload
        }
        DiscoveryPortAction::ExchangeReady { .. } => panic!("exchange completed too early"),
    }
}

fn persisted_pair() -> (
    DiscoveryExchangePortImpl,
    DiscoveryExchangePortImpl,
    Vec<u8>,
) {
    let mut publisher = port(91, "publisher");
    let mut connector = port(92, "connector");
    publisher
        .prepare_offer("offer", DiscoveryPublishTarget::Direct, b"481902")
        .unwrap();
    let start = connector
        .start_connector("exchange", &advertisement(), b"481902", false)
        .unwrap();
    let ke1 = match start {
        DiscoveryPortAction::StartPairing { payload } => payload,
        _ => panic!("missing KE1"),
    };
    let ke2 = packet(
        publisher
            .start_publisher("exchange", "discovery", "offer", ke1)
            .unwrap(),
        PairingPacketKind::OpaqueKe2,
    );
    let ke3 = packet(
        connector
            .handle_packet("exchange", PairingPacketKind::OpaqueKe2, ke2)
            .unwrap()
            .unwrap(),
        PairingPacketKind::OpaqueKe3Bundle,
    );
    let publisher_bundle = packet(
        publisher
            .handle_packet("exchange", PairingPacketKind::OpaqueKe3Bundle, ke3)
            .unwrap()
            .unwrap(),
        PairingPacketKind::PublisherBundle,
    );
    let commit = packet(
        connector
            .handle_packet(
                "exchange",
                PairingPacketKind::PublisherBundle,
                publisher_bundle,
            )
            .unwrap()
            .unwrap(),
        PairingPacketKind::ConnectorCommit,
    );
    (publisher, connector, commit)
}

fn installed(outcome: Option<DiscoveryRelationOutcome>) {
    assert!(matches!(
        outcome,
        Some(DiscoveryRelationOutcome::DirectInstalled { .. })
    ));
}

#[test]
fn review_task_persisted_pairing_stays_reportable_after_malformed_commit() {
    let (mut publisher, mut connector, commit) = persisted_pair();
    let publisher_commit = packet(
        publisher
            .handle_packet("exchange", PairingPacketKind::ConnectorCommit, commit)
            .unwrap()
            .unwrap(),
        PairingPacketKind::PublisherCommit,
    );
    assert!(connector
        .handle_packet("exchange", PairingPacketKind::PublisherCommit, vec![0])
        .is_err());
    installed(connector.take_persisted_outcome("exchange"));
    assert!(connector.take_persisted_outcome("exchange").is_none());
    assert!(!publisher_commit.is_empty());
    installed(publisher.take_persisted_outcome("exchange"));

    let (mut publisher, _, _) = persisted_pair();
    assert!(publisher
        .handle_packet("exchange", PairingPacketKind::ConnectorCommit, vec![0])
        .is_err());
    installed(publisher.take_persisted_outcome("exchange"));
}

#[test]
fn review_task_premature_completion_preserves_the_installed_pairing_outcome() {
    let (mut publisher, mut connector, _) = persisted_pair();
    assert!(publisher
        .finish_exchange("exchange", PairingCloseReason::Completed)
        .is_err());
    assert!(connector
        .finish_exchange("exchange", PairingCloseReason::Completed)
        .is_err());
    installed(publisher.take_persisted_outcome("exchange"));
    installed(connector.take_persisted_outcome("exchange"));
}

#[test]
fn review_task_empty_pin_is_rejected_by_publishing_and_connecting_ports() {
    let mut publisher = port(91, "publisher");
    let mut connector = port(92, "connector");
    assert!(matches!(
        publisher.prepare_offer("offer", DiscoveryPublishTarget::Direct, b""),
        Err(DiscoveryPortError::InvalidRequest(_))
    ));
    assert!(matches!(
        connector.start_connector("exchange", &advertisement(), b"", false),
        Err(DiscoveryPortError::InvalidRequest(_))
    ));
}
