use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::direct_protocol::DirectPeerIdentity;
use super::direct_reciprocal::{DirectReciprocalPeer, DirectRelationMaterial};
use super::discovery_bundle::{ConnectorApplicationBundle, PublisherApplicationBundle};
use super::discovery_domain::{
    DiscoveryCryptoError, DiscoveryId, DiscoveryOfferBinding, ExchangeId, OfferId, PairingBundle,
};
use super::discovery_pake::{ConnectorAwaitingKe2, PublisherOffer};
use super::discovery_relation_store::{
    DiscoveryRelationOutcome, InMemoryRelationStore, RelationStore,
};
use super::discovery_signal_state::{ActiveDiscoveryExchange, ActiveDiscoveryOffer};
use super::discovery_signal_types::{
    DiscoveryKind, DiscoveryOperation, DiscoveryPublishTarget, DiscoveryRejectionClass,
    PairingPacketKind, DISCOVERY_PAIRING_SUITE, DISCOVERY_PAIRING_VERSION,
};
use super::discovery_signal_validation::validate_rejection;
use super::discovery_signal_wire::{DiscoveryClientMsg, DiscoveryServerMsg};
use super::discovery_wire::{OpaqueKe3ConnectorBundle, PublisherBundle};
use super::room_relation::{RoomRelationMaterial, RoomRelationOffer};
use super::types::DirectAccessState;

const WIRE_FIXTURE: &str =
    include_str!("../../../../testdata/share-discovery-wire-v1.jsonl");

#[derive(Deserialize)]
struct FixtureLine {
    direction: String,
    message: serde_json::Value,
}

#[test]
fn share_remote_task_discovery_wire_fixture_roundtrips() {
    assert!(WIRE_FIXTURE.len() < 16 * 1024);
    let mut client_messages = 0;
    let mut server_messages = 0;
    for (index, line) in WIRE_FIXTURE.lines().enumerate() {
        assert!(!line.is_empty(), "blank fixture line {}", index + 1);
        assert!(line.len() < 2 * 1024, "oversized fixture line {}", index + 1);
        let fixture: FixtureLine = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("fixture line {}: {error}", index + 1));
        match fixture.direction.as_str() {
            "client" => {
                let message: DiscoveryClientMsg = serde_json::from_value(fixture.message.clone())
                    .unwrap_or_else(|error| panic!("client fixture line {}: {error}", index + 1));
                assert_eq!(serde_json::to_value(message).unwrap(), fixture.message);
                client_messages += 1;
            }
            "server" => {
                let message: DiscoveryServerMsg = serde_json::from_value(fixture.message.clone())
                    .unwrap_or_else(|error| panic!("server fixture line {}: {error}", index + 1));
                let encoded = serde_json::to_value(&message).unwrap();
                let reparsed: DiscoveryServerMsg = serde_json::from_value(encoded).unwrap();
                assert_eq!(reparsed, message);
                server_messages += 1;
            }
            other => panic!("unknown fixture direction {other}"),
        }
    }
    assert_eq!((client_messages, server_messages), (6, 7));
}

#[test]
fn share_remote_task_discovery_accepts_empty_and_zero_pin() {
    complete_pairing(b"", "empty");
    complete_pairing(b"0", "zero");
}

#[test]
fn share_remote_task_discovery_rejects_wrong_pin_tamper_replay_and_binding() {
    let (offer_binding, binding) = bindings(DiscoveryKind::Direct, "wrong-pin");
    let publisher = PublisherOffer::register(offer_binding, b"correct").unwrap();
    let (connector, ke1) = ConnectorAwaitingKe2::start(binding.clone(), b"incorrect").unwrap();
    let (publisher, ke2) = publisher.start_exchange(binding, ke1).unwrap();
    let result = connector.finish(ke2, bundle(DiscoveryKind::Direct, b"connector"));
    assert_crypto_error(result, DiscoveryCryptoError::AuthenticationFailed);
    drop(publisher);

    let (offer_binding, binding) = bindings(DiscoveryKind::Direct, "tamper");
    let publisher = PublisherOffer::register(offer_binding, b"pin").unwrap();
    let (connector, ke1) = ConnectorAwaitingKe2::start(binding.clone(), b"pin").unwrap();
    let (publisher, ke2) = publisher.start_exchange(binding, ke1).unwrap();
    let (connector, ke3) = connector
        .finish(ke2, bundle(DiscoveryKind::Direct, b"connector"))
        .unwrap();
    let (publisher, _) = publisher.finish(ke3).unwrap();
    let (_, publisher_bundle) = publisher
        .publisher_bundle(bundle(DiscoveryKind::Direct, b"publisher"))
        .unwrap();
    let mut tampered = publisher_bundle.into_bytes();
    let last = tampered.last_mut().unwrap();
    *last ^= 1;
    let result = connector.accept_publisher_bundle(PublisherBundle::from_bytes(tampered).unwrap());
    assert_crypto_error(result, DiscoveryCryptoError::AuthenticationFailed);

    let (offer_binding, binding) = bindings(DiscoveryKind::Direct, "replay");
    let publisher = PublisherOffer::register(offer_binding, b"pin").unwrap();
    let (connector_one, ke1_one) = ConnectorAwaitingKe2::start(binding.clone(), b"pin").unwrap();
    let (publisher_one, ke2_one) = publisher.start_exchange(binding.clone(), ke1_one).unwrap();
    let (_, replayed) = connector_one
        .finish(ke2_one, bundle(DiscoveryKind::Direct, b"first"))
        .unwrap();
    let replayed = replayed.to_bytes();
    let (connector_two, ke1_two) = ConnectorAwaitingKe2::start(binding.clone(), b"pin").unwrap();
    let (publisher_two, ke2_two) = publisher.start_exchange(binding, ke1_two).unwrap();
    let _ = connector_two
        .finish(ke2_two, bundle(DiscoveryKind::Direct, b"second"))
        .unwrap();
    let result = publisher_two.finish(OpaqueKe3ConnectorBundle::from_bytes(replayed).unwrap());
    assert_crypto_error(result, DiscoveryCryptoError::AuthenticationFailed);
    drop(publisher_one);

    let offer_binding = DiscoveryOfferBinding::new(
        DiscoveryKind::Direct,
        OfferId::new("offer-binding").unwrap(),
    );
    let publisher_binding = offer_binding.for_exchange(
        DiscoveryId::new("discovery-a").unwrap(),
        ExchangeId::new("exchange-a").unwrap(),
    );
    let connector_binding = offer_binding.for_exchange(
        DiscoveryId::new("discovery-b").unwrap(),
        ExchangeId::new("exchange-b").unwrap(),
    );
    let publisher = PublisherOffer::register(offer_binding, b"pin").unwrap();
    let (connector, ke1) = ConnectorAwaitingKe2::start(connector_binding, b"pin").unwrap();
    let (_, ke2) = publisher.start_exchange(publisher_binding, ke1).unwrap();
    let result = connector.finish(ke2, bundle(DiscoveryKind::Direct, b"binding"));
    assert_crypto_error(result, DiscoveryCryptoError::AuthenticationFailed);
}

#[test]
fn share_remote_task_discovery_lease_renewal_exchange_order_and_rejections() {
    let now = Instant::now();
    let mut offer = ActiveDiscoveryOffer {
        offer_id: "offer-lease".into(),
        target: DiscoveryPublishTarget::Direct,
        kind: DiscoveryKind::Direct,
        display_alias: "Device".into(),
        suite: DISCOVERY_PAIRING_SUITE.into(),
        version: DISCOVERY_PAIRING_VERSION,
        deadline: now + Duration::from_secs(601),
        discoverable_until: 601,
        next_publish_at: now,
        last_publish_lease_secs: 0,
        last_publish_sent_at: None,
        published_until: None,
        discovery_id: None,
        pairing_starts: VecDeque::new(),
    };
    let request = offer.request(now).unwrap();
    assert_eq!(request.lease_secs, 300);
    offer.mark_publish_sent(now, request.lease_secs);
    assert_eq!(offer.next_publish_at, now + Duration::from_secs(200));
    offer.last_publish_sent_at = None;
    assert!(offer.publish_due(now + Duration::from_secs(200)));
    assert_eq!(offer.request(now + Duration::from_secs(301)).unwrap().lease_secs, 300);
    assert_eq!(
        offer
            .request(now + Duration::from_millis(600_500))
            .unwrap()
            .lease_secs,
        1
    );
    assert!(offer.request(now + Duration::from_secs(601)).is_none());

    let mut exchange = ActiveDiscoveryExchange::connector("exchange".into(), "discovery".into());
    exchange.accept_opened("discovery").unwrap();
    exchange.accept_packet(PairingPacketKind::OpaqueKe2).unwrap();
    exchange
        .accept_port_packet(PairingPacketKind::OpaqueKe3Bundle)
        .unwrap();
    exchange
        .accept_packet(PairingPacketKind::PublisherBundle)
        .unwrap();
    exchange
        .accept_port_packet(PairingPacketKind::ConnectorCommit)
        .unwrap();
    exchange
        .accept_packet(PairingPacketKind::PublisherCommit)
        .unwrap();
    exchange.accept_no_packet().unwrap();
    assert!(exchange.awaits_finish());

    let mut wrong = ActiveDiscoveryExchange::connector("wrong".into(), "discovery".into());
    wrong.accept_opened("discovery").unwrap();
    assert!(wrong
        .accept_packet(PairingPacketKind::PublisherBundle)
        .is_err());
    assert!(validate_rejection(
        DiscoveryOperation::StartPairing,
        None,
        Some("discovery"),
        Some("exchange"),
        DiscoveryRejectionClass::Unavailable,
        "target unavailable",
    )
    .is_ok());
    assert!(validate_rejection(
        DiscoveryOperation::StartPairing,
        None,
        None,
        Some("exchange"),
        DiscoveryRejectionClass::Unavailable,
        "target unavailable",
    )
    .is_err());
}

#[test]
fn share_remote_task_discovery_persists_direct_and_room_relations_idempotently() {
    let peer = reciprocal_peer(2, "peer", "Peer", "peer-lookup", 7);
    let encoded = ConnectorApplicationBundle::Direct(peer.clone())
        .encode_plaintext()
        .unwrap();
    assert_eq!(
        ConnectorApplicationBundle::decode_plaintext(encoded.to_vec()).unwrap(),
        ConnectorApplicationBundle::Direct(peer.clone())
    );

    let mut store = InMemoryRelationStore::default();
    let first = store.persist_direct(&peer).unwrap();
    assert!(first.changed());
    assert!(matches!(
        first.outcome(),
        DiscoveryRelationOutcome::DirectInstalled { .. }
    ));
    let second = store.persist_direct(&peer).unwrap();
    assert!(!second.changed());
    assert_eq!(store.profiles().direct_contacts.len(), 1);
    assert_eq!(
        store.profiles().direct_contacts[0].access_state,
        DirectAccessState::Accepted
    );

    let room_material = RoomRelationMaterial::new("fixture-room", vec![9; 32]).unwrap();
    let room_offer = RoomRelationOffer::new(room_material.clone(), "Fixture Room").unwrap();
    let encoded = PublisherApplicationBundle::Room(room_offer.clone())
        .encode_plaintext()
        .unwrap();
    assert_eq!(
        PublisherApplicationBundle::decode_plaintext(encoded.to_vec()).unwrap(),
        PublisherApplicationBundle::Room(room_offer)
    );
    let first = store
        .persist_room(&room_material, "  Fixture Room  ")
        .unwrap();
    assert!(first.changed());
    let second = store.persist_room(&room_material, "ignored rename").unwrap();
    assert!(!second.changed());
    assert_eq!(store.profiles().rooms.len(), 1);
    assert_eq!(store.profiles().rooms[0].name, "Fixture Room");
}

fn complete_pairing(pin: &[u8], suffix: &str) {
    let (offer_binding, binding) = bindings(DiscoveryKind::Direct, suffix);
    let publisher = PublisherOffer::register(offer_binding, pin).unwrap();
    let (connector, ke1) = ConnectorAwaitingKe2::start(binding.clone(), pin).unwrap();
    let (publisher, ke2) = publisher.start_exchange(binding.clone(), ke1).unwrap();
    let (connector, ke3) = connector
        .finish(ke2, bundle(DiscoveryKind::Direct, b"connector"))
        .unwrap();
    let (publisher, received) = publisher.finish(ke3).unwrap();
    assert_eq!(received.payload(), b"connector");
    let (publisher, publisher_bundle) = publisher
        .publisher_bundle(bundle(DiscoveryKind::Direct, b"publisher"))
        .unwrap();
    let (connector, received) = connector.accept_publisher_bundle(publisher_bundle).unwrap();
    assert_eq!(received.payload(), b"publisher");
    let (connector, connector_commit) = connector.commit().unwrap();
    let publisher = publisher.accept_connector_commit(connector_commit).unwrap();
    let (publisher, publisher_commit) = publisher.commit().unwrap();
    let connector = connector.finish(publisher_commit).unwrap();
    assert_eq!(publisher.binding(), &binding);
    assert_eq!(connector.binding(), &binding);
}

fn bindings(
    kind: DiscoveryKind,
    suffix: &str,
) -> (DiscoveryOfferBinding, super::discovery_domain::DiscoveryExchangeBinding) {
    let offer = DiscoveryOfferBinding::new(
        kind,
        OfferId::new(format!("offer-{suffix}")).unwrap(),
    );
    let exchange = offer.for_exchange(
        DiscoveryId::new(format!("discovery-{suffix}")).unwrap(),
        ExchangeId::new(format!("exchange-{suffix}")).unwrap(),
    );
    (offer, exchange)
}

fn bundle(kind: DiscoveryKind, payload: &[u8]) -> PairingBundle {
    PairingBundle::new(kind, payload.to_vec()).unwrap()
}

fn assert_crypto_error<T: std::fmt::Debug>(
    result: Result<T, DiscoveryCryptoError>,
    expected: DiscoveryCryptoError,
) {
    match result {
        Err(error) => assert_eq!(error, expected),
        Ok(value) => panic!("expected {expected:?}, received {value:?}"),
    }
}

fn reciprocal_peer(
    seed: u8,
    device_id: &str,
    device_name: &str,
    lookup_id: &str,
    secret: u8,
) -> DirectReciprocalPeer {
    let key = iroh::SecretKey::from_bytes(&[seed; 32]);
    let identity = DirectPeerIdentity::from_secret(device_id, device_name, &key);
    let material = DirectRelationMaterial::new(lookup_id, vec![secret; 32]).unwrap();
    DirectReciprocalPeer::authenticated(identity, material).unwrap()
}
