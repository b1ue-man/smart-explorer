use super::core::{hmac_proof, presence_payload, public_fingerprint};
use super::direct_protocol::DirectPeerIdentity;
use super::direct_reciprocal::{
    DirectReciprocalApply, DirectReciprocalConflict, DirectReciprocalError,
    DirectReciprocalPeer, DirectReciprocalPolicyDenied, DirectRelationMaterial,
};
use super::direct_reciprocal_session::{
    AuthenticatedDirectSession, DirectRepairInitiator, DirectRepairReceiver,
    DirectRepairSessionError, DirectSessionAuthorization,
};
use super::direct_reciprocal_store::{
    DirectRepairPersistPhase, DirectRepairPersistRequest, DirectRepairStore,
    DirectRepairStoreError, DirectRepairStoreReceipt,
};
use super::direct_reciprocal_wire::{
    decode_direct_repair_frame, encode_direct_repair_frame, DirectRepairHello,
    DirectRepairMessage, DirectRepairOffer, DirectRepairPersisted,
};
use super::identity::ShareIdentity;
use super::legacy_direct_request::{
    LegacyDirectDecisionSource, LegacyDirectDecisionState, LegacyDirectDeliveryState,
};
use super::profiles::ShareProfiles;
use super::types::{DirectAccessState, DirectGrant, DirectGrantState, PeerPresence};

#[test]
fn share_remote_task_reciprocal_direct_fresh_autoaccepts_both_sides() {
    let peer_a = reciprocal_peer(2, "device-a", "Device A", "lookup-a", 12);
    let peer_b = reciprocal_peer(3, "device-b", "Device B", "lookup-b", 13);
    let mut profiles_a = ShareProfiles::default();
    let mut profiles_b = ShareProfiles::default();

    let applied_a = profiles_a
        .apply_reciprocal_direct_peer(&peer_b, "contact-b", 100)
        .unwrap();
    let applied_b = profiles_b
        .apply_reciprocal_direct_peer(&peer_a, "contact-a", 100)
        .unwrap();
    assert_eq!(
        applied_a,
        DirectReciprocalApply::Changed {
            contact_id: "contact-b".into()
        }
    );
    assert_eq!(
        applied_b,
        DirectReciprocalApply::Changed {
            contact_id: "contact-a".into()
        }
    );
    assert_complete_relation(&profiles_a, &peer_b, "contact-b");
    assert_complete_relation(&profiles_b, &peer_a, "contact-a");

    assert_eq!(
        profiles_a
            .apply_reciprocal_direct_peer(&peer_b, "unused", 200)
            .unwrap(),
        DirectReciprocalApply::AlreadyComplete {
            contact_id: "contact-b".into()
        }
    );
}

#[test]
fn share_remote_task_reciprocal_direct_repairs_legacy_pins_and_retries_idempotently() {
    let first = run_repair(DirectRepairPersisted::Changed);
    assert_eq!(
        first,
        [
            DirectRepairPersisted::Changed,
            DirectRepairPersisted::Changed
        ]
    );
    let retried = run_repair(DirectRepairPersisted::AlreadyComplete);
    assert_eq!(
        retried,
        [
            DirectRepairPersisted::AlreadyComplete,
            DirectRepairPersisted::AlreadyComplete
        ]
    );
}

#[test]
fn share_remote_task_reciprocal_direct_denial_unsupported_and_identity_conflict_fail_closed() {
    let remote = reciprocal_peer(4, "remote", "Remote", "remote-lookup", 4);
    let unsupported = authenticated_session(
        remote.identity(),
        DirectSessionAuthorization::OutgoingAcceptedContact,
        false,
    );
    assert!(matches!(
        unsupported,
        Err(DirectRepairSessionError::CapabilityNotRequested)
    ));
    let denied = authenticated_session(
        remote.identity(),
        DirectSessionAuthorization::ExplicitPolicyDenied,
        true,
    );
    assert!(matches!(denied, Err(DirectRepairSessionError::PolicyDenied)));

    let mut denied_profiles = ShareProfiles::default();
    denied_profiles.direct_grants.push(grant_for(
        remote.identity(),
        DirectGrantState::Ignored,
    ));
    assert_eq!(
        denied_profiles.apply_reciprocal_direct_peer(&remote, "denied", 100),
        Err(DirectReciprocalError::PolicyDenied(
            DirectReciprocalPolicyDenied::GrantIgnored {
                device_id: "remote".into()
            }
        ))
    );

    let trusted = reciprocal_peer(5, "same-device", "Trusted", "trusted-lookup", 5);
    let replacement = reciprocal_peer(6, "same-device", "Replacement", "new-lookup", 6);
    let mut profiles = ShareProfiles::default();
    profiles
        .apply_reciprocal_direct_peer(&trusted, "trusted", 100)
        .unwrap();
    assert_eq!(
        profiles.apply_reciprocal_direct_peer(&replacement, "replacement", 101),
        Err(DirectReciprocalError::Conflict(
            DirectReciprocalConflict::ContactIdentity {
                device_id: "same-device".into()
            }
        ))
    );
    assert_complete_relation(&profiles, &trusted, "trusted");
}

#[test]
fn share_remote_task_legacy_direct_autoaccept_retry_tombstone_and_denial() {
    let local = local_identity();
    let mut profiles = ShareProfiles::default();
    let first = legacy_presence(&local, 7, "nonce-a", 200);
    assert!(profiles
        .record_verified_legacy_direct_request(&local.direct_lookup_id, &first, 100)
        .unwrap());
    let selector = profiles.legacy_direct_requests[0].selector.clone();
    let entry = profiles.legacy_direct_request(&selector).unwrap();
    assert_eq!(entry.decision, LegacyDirectDecisionState::Accepted);
    assert_eq!(
        entry.decision_source,
        Some(LegacyDirectDecisionSource::AuthenticatedSecretPossession)
    );
    assert!(entry.authorization_active(&profiles));
    assert!(!profiles
        .record_verified_legacy_direct_request(&local.direct_lookup_id, &first, 101)
        .unwrap());

    let revision = profiles
        .legacy_direct_request(&selector)
        .unwrap()
        .decision_revision;
    profiles
        .record_legacy_answer_attempt(&selector, revision, 102, None)
        .unwrap();
    assert_eq!(
        profiles
            .legacy_direct_request(&selector)
            .unwrap()
            .decision_delivery
            .state,
        LegacyDirectDeliveryState::AttemptedUntracked
    );
    profiles.retry_legacy_answer(&selector).unwrap();
    assert_eq!(
        profiles
            .legacy_direct_request(&selector)
            .unwrap()
            .decision_delivery
            .state,
        LegacyDirectDeliveryState::Queued
    );

    profiles
        .revoke_legacy_direct_request(&selector, 110)
        .unwrap();
    assert!(profiles
        .delete_legacy_direct_request(&selector, 111)
        .unwrap());
    assert!(!profiles
        .record_verified_legacy_direct_request(&local.direct_lookup_id, &first, 112)
        .unwrap());
    let later = legacy_presence(&local, 7, "nonce-b", 320);
    assert!(profiles
        .record_verified_legacy_direct_request(&local.direct_lookup_id, &later, 201)
        .unwrap());
    let entry = &profiles.legacy_direct_requests[0];
    assert_eq!(entry.decision, LegacyDirectDecisionState::Rejected);
    assert_eq!(
        profiles.grant_for("legacy-peer").unwrap().state,
        DirectGrantState::Ignored
    );

    let conflicting_key = iroh::SecretKey::from_bytes(&[8; 32]);
    let conflicting_identity =
        DirectPeerIdentity::from_secret("legacy-peer", "Existing", &conflicting_key);
    let mut conflict = ShareProfiles::default();
    conflict.direct_grants.push(grant_for(
        &conflicting_identity,
        DirectGrantState::Accepted,
    ));
    assert!(conflict
        .record_verified_legacy_direct_request(&local.direct_lookup_id, &first, 100)
        .unwrap());
    assert!(conflict.legacy_direct_requests[0].identity_conflict);
    assert_eq!(
        conflict.legacy_direct_requests[0].decision,
        LegacyDirectDecisionState::Rejected
    );
    assert_eq!(
        conflict.direct_grants[0].public_key,
        conflicting_identity.public_key
    );
}

fn run_repair(persisted: DirectRepairPersisted) -> [DirectRepairPersisted; 2] {
    let peer_a = reciprocal_peer(9, "device-a", "Device A", "lookup-a", 9);
    let peer_b = reciprocal_peer(10, "device-b", "Device B", "lookup-b", 10);
    let outgoing = authenticated_session(
        peer_b.identity(),
        DirectSessionAuthorization::OutgoingAcceptedContact,
        true,
    )
    .unwrap();
    let incoming = authenticated_session(
        peer_a.identity(),
        DirectSessionAuthorization::IncomingFreshNoDecisionGrant,
        true,
    )
    .unwrap();
    let (initiator, hello) = DirectRepairInitiator::begin(
        peer_a.identity().clone(),
        peer_a.material(),
        outgoing,
        Some(peer_b.material().clone()),
    )
    .unwrap();
    let receiver = DirectRepairReceiver::new(
        peer_b.identity().clone(),
        peer_b.material().clone(),
        incoming,
        Some(peer_a.material().clone()),
    )
    .unwrap();

    let hello = roundtrip_hello(hello);
    let receiver = receiver.accept_hello(hello).unwrap();
    let mut receiver_store = RecordingStore::new(persisted);
    let (receiver, offer) = receiver.persist_with(&mut receiver_store).unwrap();
    let offer = roundtrip_offer(offer);
    let initiator = initiator.accept_offer(offer).unwrap();
    let mut initiator_store = RecordingStore::new(persisted);
    let (initiator, commit) = initiator.persist_with(&mut initiator_store).unwrap();
    let commit = match roundtrip(DirectRepairMessage::Commit(commit)) {
        DirectRepairMessage::Commit(message) => message,
        _ => panic!("commit changed kind"),
    };
    let (receiver, complete) = receiver.accept_commit(commit).unwrap();
    let complete = match roundtrip(DirectRepairMessage::Complete(complete)) {
        DirectRepairMessage::Complete(message) => message,
        _ => panic!("complete changed kind"),
    };
    let initiator = initiator.accept_complete(complete).unwrap();

    assert_eq!(
        receiver_store.phases,
        vec![DirectRepairPersistPhase::ReceivedHello]
    );
    assert_eq!(receiver_store.devices, vec!["device-a"]);
    assert_eq!(
        initiator_store.phases,
        vec![DirectRepairPersistPhase::ReceivedOffer]
    );
    assert_eq!(initiator_store.devices, vec!["device-b"]);
    assert_eq!(receiver.receiver_persisted, persisted);
    assert_eq!(receiver.initiator_persisted, persisted);
    assert_eq!(initiator.receiver_persisted, persisted);
    assert_eq!(initiator.initiator_persisted, persisted);
    [initiator.receiver_persisted, initiator.initiator_persisted]
}

fn roundtrip_hello(message: DirectRepairHello) -> DirectRepairHello {
    match roundtrip(DirectRepairMessage::Hello(message)) {
        DirectRepairMessage::Hello(message) => message,
        _ => panic!("hello changed kind"),
    }
}

fn roundtrip_offer(message: DirectRepairOffer) -> DirectRepairOffer {
    match roundtrip(DirectRepairMessage::Offer(message)) {
        DirectRepairMessage::Offer(message) => message,
        _ => panic!("offer changed kind"),
    }
}

fn roundtrip(message: DirectRepairMessage) -> DirectRepairMessage {
    let encoded = encode_direct_repair_frame(&message).unwrap();
    decode_direct_repair_frame(encoded.as_bytes().to_vec()).unwrap()
}

fn authenticated_session(
    remote: &DirectPeerIdentity,
    authorization: DirectSessionAuthorization,
    capability: bool,
) -> Result<AuthenticatedDirectSession, DirectRepairSessionError> {
    AuthenticatedDirectSession::from_verified_handshake(
        remote.device_id.clone(),
        remote.node_id.clone(),
        remote.public_key.clone(),
        remote.fingerprint.clone(),
        String::new(),
        authorization,
        capability,
    )
}

struct RecordingStore {
    persisted: DirectRepairPersisted,
    phases: Vec<DirectRepairPersistPhase>,
    devices: Vec<String>,
}

impl RecordingStore {
    fn new(persisted: DirectRepairPersisted) -> Self {
        Self {
            persisted,
            phases: Vec::new(),
            devices: Vec::new(),
        }
    }
}

impl DirectRepairStore for RecordingStore {
    fn persist_reciprocal_peer(
        &mut self,
        request: &DirectRepairPersistRequest<'_>,
    ) -> Result<DirectRepairStoreReceipt, DirectRepairStoreError> {
        self.phases.push(request.phase());
        self.devices
            .push(request.peer().identity().device_id.clone());
        Ok(request.receipt_after_durable_commit(self.persisted))
    }
}

fn assert_complete_relation(
    profiles: &ShareProfiles,
    remote: &DirectReciprocalPeer,
    contact_id: &str,
) {
    let contact = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .unwrap();
    assert_eq!(contact.lookup_id, remote.material().lookup_id());
    assert_eq!(contact.access_state, DirectAccessState::Accepted);
    assert_eq!(
        contact.remote_device_id.as_deref(),
        Some(remote.identity().device_id.as_str())
    );
    let grant = profiles.grant_for(&remote.identity().device_id).unwrap();
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert_eq!(grant.public_key, remote.identity().public_key);
}

fn grant_for(identity: &DirectPeerIdentity, state: DirectGrantState) -> DirectGrant {
    DirectGrant {
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
        node_id: identity.node_id.clone(),
        state,
        updated_at: 1,
        exec: Default::default(),
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

fn local_identity() -> ShareIdentity {
    let key = iroh::SecretKey::from_bytes(&[11; 32]);
    let public_key = key.public().to_string();
    ShareIdentity {
        device_id: "local-device".into(),
        device_name: "Local".into(),
        direct_lookup_id: "local-lookup".into(),
        fingerprint: public_fingerprint(public_key.as_bytes()),
        node_id: public_key.clone(),
        public_key,
        iroh_secret: key,
        direct_secret: [21; 32],
    }
}

fn legacy_presence(
    local: &ShareIdentity,
    seed: u8,
    nonce: &str,
    expires_at: i64,
) -> PeerPresence {
    let key = iroh::SecretKey::from_bytes(&[seed; 32]);
    let public_key = key.public().to_string();
    let mut presence = PeerPresence {
        kind: "direct".into(),
        relation_id: local.direct_lookup_id.clone(),
        device_id: "legacy-peer".into(),
        device_name: "Legacy Peer".into(),
        fingerprint: public_fingerprint(public_key.as_bytes()),
        node_id: public_key.clone(),
        public_key,
        relay_url: "https://relay.invalid".into(),
        candidates: vec!["127.0.0.1:9000".into()],
        expires_at,
        nonce: nonce.into(),
        proof: String::new(),
    };
    let payload = presence_payload(
        "direct",
        &presence.relation_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    presence.proof = hmac_proof(&local.direct_secret(), &payload);
    presence
}
