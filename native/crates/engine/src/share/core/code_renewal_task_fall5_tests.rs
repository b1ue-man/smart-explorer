//! Task suite of the Direct-code renewal (E17, M21), cases 2 (c)(d), 5 and the
//! replay of an old renewal message: the reciprocal exchange without a
//! network. Initiator and receiver run their state machines against each
//! other through the real wire frames; the stores record what they were asked
//! to persist.
use super::{CodeRenewalState, RenewalPeer};
use crate::share::direct_protocol::DirectPeerIdentity;
use crate::share::direct_reciprocal::{
    DirectReciprocalApply, DirectReciprocalConflict, DirectReciprocalError, DirectReciprocalPeer,
    DirectRelationMaterial,
};
use crate::share::direct_reciprocal_session::{
    AuthenticatedDirectSession, DirectRepairInitiator, DirectRepairInitiatorAwaitingOffer,
    DirectRepairReceiver, DirectRepairSessionError, DirectSessionAuthorization,
};
use crate::share::direct_reciprocal_store::{
    DirectRepairPersistRequest, DirectRepairStore, DirectRepairStoreError, DirectRepairStoreReceipt,
};
use crate::share::direct_reciprocal_wire::{
    decode_direct_repair_frame, encode_direct_repair_frame, DirectRepairHello, DirectRepairMessage,
    DirectRepairPersisted,
};
use crate::share::identity::code_renewal_registry::{self as registry, RetiringMaterial};
use crate::share::profiles::ShareProfiles;
use crate::share::removed_direct_peers::PairingOrigin;

use zeroize::Zeroizing;

/// The host H renewed its Direct code (`alt` runs out, `neu` is current); the
/// peer P holds the code of H and has its own data `own` at H.
struct World {
    host: DirectPeerIdentity,
    peer: DirectPeerIdentity,
    alt: DirectRelationMaterial,
    neu: DirectRelationMaterial,
    own: DirectRelationMaterial,
}

fn world() -> World {
    let identity = |device: &str, seed: u8| {
        DirectPeerIdentity::from_secret(device, device, &iroh::SecretKey::from_bytes(&[seed; 32]))
    };
    let material = |lookup: &str, byte: u8| DirectRelationMaterial::new(lookup, vec![byte; 32]);
    World {
        host: identity("host", 181),
        peer: identity("peer", 182),
        alt: material("lookup-alt", 1).unwrap(),
        neu: material("lookup-neu", 2).unwrap(),
        own: material("lookup-own", 3).unwrap(),
    }
}

fn session(
    remote: &DirectPeerIdentity,
    authorization: DirectSessionAuthorization,
) -> AuthenticatedDirectSession {
    AuthenticatedDirectSession::from_verified_handshake(
        remote.device_id.clone(),
        remote.node_id.clone(),
        remote.public_key.clone(),
        remote.fingerprint.clone(),
        String::new(),
        authorization,
        true,
    )
    .unwrap()
}

/// P opens the exchange: it holds `alt` of H. With `renewal` it asks for
/// renewed data (kind 5, only after H's signed presence announced it).
fn initiator(
    world: &World,
    renewal: bool,
) -> (DirectRepairInitiatorAwaitingOffer, DirectRepairHello) {
    let outgoing = session(
        &world.host,
        DirectSessionAuthorization::OutgoingAcceptedContact,
    );
    let expected = Some(world.alt.clone());
    let begun = if renewal {
        DirectRepairInitiator::begin_renewal(world.peer.clone(), &world.own, outgoing, expected)
    } else {
        DirectRepairInitiator::begin(world.peer.clone(), &world.own, outgoing, expected)
    };
    begun.unwrap()
}

/// H answers: its current data are `neu`; with `retiring` the session came in
/// over the running out lookup id and H still knows its data.
fn receiver(world: &World, retiring: bool) -> DirectRepairReceiver {
    let incoming = session(
        &world.peer,
        DirectSessionAuthorization::IncomingAcceptedGrant,
    );
    let receiver = DirectRepairReceiver::new(
        world.host.clone(),
        world.neu.clone(),
        incoming,
        Some(world.own.clone()),
    )
    .unwrap();
    if retiring {
        receiver.with_retiring_material(world.alt.clone())
    } else {
        receiver
    }
}

/// What a store was asked to persist: lookup id of the peer's data and
/// whether they replace the data this device holds.
#[derive(Default)]
struct Recorder {
    lookups: Vec<String>,
    renewals: Vec<bool>,
}

impl DirectRepairStore for Recorder {
    fn persist_reciprocal_peer(
        &mut self,
        request: &DirectRepairPersistRequest<'_>,
    ) -> Result<DirectRepairStoreReceipt, DirectRepairStoreError> {
        self.lookups
            .push(request.peer().material().lookup_id().to_string());
        self.renewals.push(request.peer().is_renewal());
        Ok(request.receipt_after_durable_commit(DirectRepairPersisted::Changed))
    }
}

fn frame_bytes(message: &DirectRepairMessage) -> Vec<u8> {
    encode_direct_repair_frame(message)
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn hello_of(bytes: Vec<u8>) -> DirectRepairHello {
    match decode_direct_repair_frame(bytes).unwrap() {
        DirectRepairMessage::Hello(hello) => hello,
        other => panic!("expected a hello, got {other:?}"),
    }
}

/// The frame kind byte: length (4), magic (4), version (1), kind.
fn kind_of(bytes: &[u8]) -> u8 {
    bytes[9]
}

/// One complete exchange; returns what H and P stored and whether the offer
/// carried H's current data.
struct Exchange {
    kind: u8,
    accepts_renewal: bool,
    offers_current: bool,
    host_store: Recorder,
    peer_store: Recorder,
}

fn exchange(
    world: &World,
    renewal: bool,
    retiring: bool,
) -> Result<Exchange, DirectRepairSessionError> {
    let (awaiting_offer, hello) = initiator(world, renewal);
    let bytes = frame_bytes(&DirectRepairMessage::Hello(hello));
    let kind = kind_of(&bytes);
    let hello = hello_of(bytes);
    let accepts_renewal = hello.accepts_renewal;
    let awaiting_store = receiver(world, retiring).accept_hello(hello)?;
    let offers_current = awaiting_store.offers_current();
    let mut host_store = Recorder::default();
    let (awaiting_commit, offer) = awaiting_store.persist_with(&mut host_store)?;
    let offer = match decode_direct_repair_frame(frame_bytes(&DirectRepairMessage::Offer(offer))) {
        Ok(DirectRepairMessage::Offer(offer)) => offer,
        other => panic!("expected an offer, got {other:?}"),
    };
    let mut peer_store = Recorder::default();
    let (awaiting_complete, commit) = awaiting_offer
        .accept_offer(offer)?
        .persist_with(&mut peer_store)?;
    let (_, complete) = awaiting_commit.accept_commit(commit)?;
    awaiting_complete.accept_complete(complete)?;
    Ok(Exchange {
        kind,
        accepts_renewal,
        offers_current,
        host_store,
        peer_store,
    })
}

#[test]
fn code_renewal_task_renewal_hello_takes_the_hosts_new_data_after_the_commit() {
    let world = world();
    let done = exchange(&world, true, true).unwrap();
    // Kind 5 on the wire, accepted by the host as a request for renewed data.
    assert_eq!(done.kind, 5);
    assert!(done.accepts_renewal);
    assert!(done.offers_current);
    // P stores the new data of H as a replacement of the old ones; H stores
    // P's own data unchanged, not as a renewal.
    assert_eq!(done.peer_store.lookups, ["lookup-neu"]);
    assert_eq!(done.peer_store.renewals, [true]);
    assert_eq!(done.host_store.lookups, ["lookup-own"]);
    assert_eq!(done.host_store.renewals, [false]);
    // The transcript binds the kind: kind 5 cannot be read as kind 1.
    let (_, hello) = initiator(&world, true);
    let renewal_digest = hello.digest();
    let mut as_plain = hello_of(frame_bytes(&DirectRepairMessage::Hello(hello)));
    assert!(as_plain.accepts_renewal);
    as_plain.accepts_renewal = false;
    assert!(as_plain.digest() != renewal_digest);
    let (_, plain) = initiator(&world, false);
    assert_eq!(kind_of(&frame_bytes(&DirectRepairMessage::Hello(plain))), 1);
}

#[test]
fn code_renewal_task_old_peer_without_the_capability_keeps_the_old_data() {
    let world = world();
    // Kind 1 over the running out lookup id: H offers exactly the old data.
    let done = exchange(&world, false, true).unwrap();
    assert_eq!(done.kind, 1);
    assert!(!done.accepts_renewal);
    assert!(!done.offers_current);
    assert_eq!(done.peer_store.lookups, ["lookup-alt"]);
    assert_eq!(done.peer_store.renewals, [false]);
    assert_eq!(done.host_store.lookups, ["lookup-own"]);
    // Kind 1 against a host that does not know the old data offers the
    // current ones; a peer that did not ask for renewed data refuses them as
    // before (no silent overwrite).
    assert_eq!(
        exchange(&world, false, false).err(),
        Some(DirectRepairSessionError::RelationMaterialConflict)
    );
}

#[test]
fn code_renewal_task_replayed_renewal_messages_are_discarded() {
    let world = world();
    // Record the messages of a first exchange on the wire.
    let (awaiting_offer, hello) = initiator(&world, true);
    let hello_bytes = frame_bytes(&DirectRepairMessage::Hello(hello));
    let awaiting_store = receiver(&world, true)
        .accept_hello(hello_of(hello_bytes.clone()))
        .unwrap();
    let mut host_store = Recorder::default();
    let (_, offer) = awaiting_store.persist_with(&mut host_store).unwrap();
    let offer_bytes = frame_bytes(&DirectRepairMessage::Offer(offer));
    let offer_of = |bytes: &[u8]| match decode_direct_repair_frame(bytes.to_vec()).unwrap() {
        DirectRepairMessage::Offer(offer) => offer,
        other => panic!("expected an offer, got {other:?}"),
    };
    let mut peer_store = Recorder::default();
    let (_, commit) = awaiting_offer
        .accept_offer(offer_of(&offer_bytes))
        .unwrap()
        .persist_with(&mut peer_store)
        .unwrap();
    let commit_bytes = frame_bytes(&DirectRepairMessage::Commit(commit));
    // The first offer in a second exchange: another repair id, no effect.
    let (second_offer, _second_hello) = initiator(&world, true);
    assert_eq!(
        second_offer.accept_offer(offer_of(&offer_bytes)).err(),
        Some(DirectRepairSessionError::RepairIdMismatch)
    );
    // The first commit in a second exchange: no effect either.
    let second_store = receiver(&world, true)
        .accept_hello(hello_of(frame_bytes(&DirectRepairMessage::Hello(
            initiator(&world, true).1,
        ))))
        .unwrap();
    let (second_commit, _) = second_store.persist_with(&mut Recorder::default()).unwrap();
    let old_commit = match decode_direct_repair_frame(commit_bytes).unwrap() {
        DirectRepairMessage::Commit(commit) => commit,
        other => panic!("expected a commit, got {other:?}"),
    };
    assert_eq!(
        second_commit.accept_commit(old_commit).err(),
        Some(DirectRepairSessionError::RepairIdMismatch)
    );
    // The first hello over the session of another device: the pins of the
    // authenticated session do not name the sender of the hello.
    let thief =
        DirectPeerIdentity::from_secret("thief", "thief", &iroh::SecretKey::from_bytes(&[183; 32]));
    let stolen = DirectRepairReceiver::new(
        world.host.clone(),
        world.neu.clone(),
        session(&thief, DirectSessionAuthorization::IncomingAcceptedGrant),
        None,
    )
    .unwrap();
    assert_eq!(
        stolen.accept_hello(hello_of(hello_bytes)).err(),
        Some(DirectRepairSessionError::IdentityConflict)
    );
    // Only the first exchange stored anything.
    assert_eq!(host_store.lookups, ["lookup-own"]);
    assert_eq!(peer_store.lookups, ["lookup-neu"]);
}

#[test]
fn code_renewal_task_replayed_confirmations_change_nothing() {
    let peer = |device: &str| RenewalPeer {
        device_id: device.into(),
        device_name: device.into(),
        public_key: format!("key-{device}"),
        node_id: format!("node-{device}"),
        legacy: false,
    };
    let mut state = CodeRenewalState::default();
    state.initialize(1);
    state.prepare("alt", "neu", vec![peer("p"), peer("q")], Some(2), 100);
    assert!(state.settle("neu").is_empty());
    // p confirmed once.
    assert!(state
        .confirm("p", "key-p", "node-p", "neu", "neu")
        .is_empty());
    let after_first = state.clone();
    // The same confirmation again, or one for the ended generation: no effect.
    assert!(state
        .confirm("p", "key-p", "node-p", "neu", "neu")
        .is_empty());
    assert!(state
        .confirm("p", "key-p", "node-p", "alt", "neu")
        .is_empty());
    assert!(!state.mark_legacy("p", "key-p", "node-p", "alt"));
    assert_eq!(state, after_first);
    // The last confirmation ends the generation; a replay finds nothing left.
    assert_eq!(state.confirm("q", "key-q", "node-q", "neu", "neu"), ["alt"]);
    let ended = state.clone();
    assert!(state
        .confirm("q", "key-q", "node-q", "neu", "neu")
        .is_empty());
    assert!(state
        .confirm("q", "key-q", "node-q", "alt", "neu")
        .is_empty());
    assert!(!state.mark_legacy("q", "key-q", "node-q", "alt"));
    assert_eq!(state, ended);
    assert!(state.retiring.is_empty());
}

#[test]
fn code_renewal_task_a_push_to_a_waiting_peer_carries_the_retiring_data() {
    let host = "cr-task-fall5-host";
    let waiting = ("device-p", "key-p", "node-p");
    let material = RetiringMaterial {
        lookup_id: "lookup-alt".into(),
        secret: Zeroizing::new(vec![1; 32]),
        pending: vec![
            (waiting.0.into(), waiting.1.into(), waiting.2.into()),
            // A grant persisted without a node pin names the node of its key.
            ("device-r".into(), "key-r".into(), String::new()),
        ],
    };
    registry::install(
        host,
        "lookup-neu",
        vec![material],
        Vec::new(),
        Default::default(),
    );
    let found = registry::pending_material_for(host, waiting.0, waiting.1, waiting.2);
    let (lookup_id, secret) = found.expect("a waiting peer is found");
    assert_eq!(lookup_id, "lookup-alt");
    assert_eq!(secret.as_slice(), [1u8; 32]);
    assert!(registry::pending_material_for(host, "device-r", "key-r", "key-r").is_some());
    // A peer that is not waiting, a different key or node, or another host
    // gets nothing from the retiring data: its push carries the current data.
    assert!(registry::pending_material_for(host, "device-q", "key-q", "node-q").is_none());
    assert!(registry::pending_material_for(host, "device-p", "key-other", "node-p").is_none());
    assert!(registry::pending_material_for(host, "device-p", "key-p", "node-other").is_none());
    assert!(
        registry::pending_material_for("cr-task-fall5-other", "device-p", "key-p", "node-p")
            .is_none()
    );
    assert!(registry::is_pending(host, "device-p", "key-p"));
    assert!(!registry::is_pending(host, "device-q", "key-q"));
    // After the renewal ended the same peer gets nothing.
    registry::install(
        host,
        "lookup-neu",
        Vec::new(),
        Vec::new(),
        Default::default(),
    );
    assert!(registry::pending_material_for(host, waiting.0, waiting.1, waiting.2).is_none());
}

#[test]
fn code_renewal_task_unrenewed_material_stays_a_conflict_and_the_renewed_contact_is_complete() {
    let world = world();
    let old = DirectReciprocalPeer::authenticated(world.host.clone(), world.alt.clone()).unwrap();
    let new = DirectReciprocalPeer::authenticated(world.host.clone(), world.neu.clone()).unwrap();
    let mut profiles = ShareProfiles::default();
    let applied = profiles
        .apply_reciprocal_direct_peer(&old, "contact-h", 100, PairingOrigin::UserPairing)
        .unwrap();
    assert_eq!(
        applied,
        DirectReciprocalApply::Changed {
            contact_id: "contact-h".into()
        }
    );
    // Other data than the stored ones never overwrite the contact by the
    // ordinary path; only the renewal path of the persistence layer does.
    let refused =
        profiles.apply_reciprocal_direct_peer(&new, "other", 200, PairingOrigin::AutomaticRepair);
    assert_eq!(
        refused,
        Err(DirectReciprocalError::Conflict(
            DirectReciprocalConflict::ContactIdentity {
                device_id: "host".into()
            }
        ))
    );
    assert_eq!(profiles.direct_contacts.len(), 1);
    assert_eq!(profiles.direct_contacts[0].lookup_id, "lookup-alt");
    // What the renewal path leaves behind (same contact id, new lookup id, no
    // presence of the old id): the next exchange finds the relation complete.
    profiles.direct_contacts[0].lookup_id = "lookup-neu".into();
    profiles.direct_contacts[0].presence = None;
    let again = profiles
        .apply_reciprocal_direct_peer(&new, "other", 300, PairingOrigin::AutomaticRepair)
        .unwrap();
    assert_eq!(
        again,
        DirectReciprocalApply::AlreadyComplete {
            contact_id: "contact-h".into()
        }
    );
    assert_eq!(profiles.direct_contacts[0].id, "contact-h");
}
