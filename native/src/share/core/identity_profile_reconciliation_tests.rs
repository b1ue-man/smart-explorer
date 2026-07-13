use super::core::public_fingerprint;
use super::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectDecision,
    SignedDirectDecisionReceipt, SignedDirectRequest,
};
use super::direct_request_tombstone::{
    DirectRequestDeleteDisposition, DirectRequestTombstone, MAX_DIRECT_REQUEST_TOMBSTONES,
};
use super::tracked_signal_outbox::pending_envelopes;
use super::{
    DirectAccessState, DirectContact, DirectDecisionState, DirectGrantState,
    DirectRequestDirection, IdentityRepairAction, ShareProfiles, ShareStatus,
};

const SECRET: [u8; 32] = [0x66; 32];
const REQUEST_A: &str = "123e4567-e89b-42d3-a456-426614174000";
const REQUEST_B: &str = "223e4567-e89b-42d3-a456-426614174000";
const REQUEST_C: &str = "323e4567-e89b-42d3-a456-426614174000";

#[test]
fn post_rotation_reconciliation_removes_every_stale_incoming_outbox() {
    let accepted = request(REQUEST_A, "lookup-old");
    let pending = request(REQUEST_B, "lookup-old");
    let current = request(REQUEST_C, "lookup-new");
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("lookup-old", accepted.clone(), 110)
        .unwrap();
    profiles
        .record_direct_decision(
            decision(&accepted, DirectDecisionKind::Accepted, 1, 130),
            130,
        )
        .unwrap();
    profiles
        .record_incoming_direct_request("lookup-old", pending.clone(), 111)
        .unwrap();
    profiles
        .record_incoming_direct_request("lookup-new", current.clone(), 112)
        .unwrap();
    assert_eq!(pending_envelopes(&profiles.direct_requests, 140).len(), 1);

    profiles
        .invalidate_direct_grants_after_identity_rotation(
            "lookup-new",
            140,
            IdentityRepairAction::DirectCodeRotated,
        )
        .unwrap();

    assert_eq!(profiles.direct_requests.len(), 1);
    assert!(profiles.direct_request(&current.request_id).is_some());
    assert!(profiles.direct_request(&accepted.request_id).is_none());
    assert!(profiles.direct_request(&pending.request_id).is_none());
    assert!(pending_envelopes(&profiles.direct_requests, 140).is_empty());
    assert!(profiles.direct_request_tombstones.is_empty());
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Ignored);
    profiles.validate_direct_ledger().unwrap();
}

#[test]
fn direct_code_rotation_preserves_outgoing_authorization_and_history() {
    let outgoing = request(REQUEST_A, "peer-lookup");
    let incoming = request(REQUEST_B, "local-old");
    let mut profiles = outgoing_profiles(&outgoing);
    profiles
        .record_direct_decision(
            decision(&outgoing, DirectDecisionKind::Accepted, 1, 130),
            130,
        )
        .unwrap();
    profiles
        .record_incoming_direct_request("local-old", incoming.clone(), 111)
        .unwrap();

    profiles
        .invalidate_direct_grants_after_identity_rotation(
            "local-new",
            140,
            IdentityRepairAction::DirectCodeRotated,
        )
        .unwrap();

    assert!(profiles.direct_request(&outgoing.request_id).is_some());
    assert!(profiles.direct_request(&incoming.request_id).is_none());
    assert_eq!(
        profiles.direct_contacts[0].access_state,
        DirectAccessState::Accepted
    );
    assert!(profiles.direct_request_tombstones.is_empty());
}

#[test]
fn full_identity_replacement_stops_outbox_and_resets_contact_projection() {
    let outgoing = request(REQUEST_A, "peer-lookup");
    let mut profiles = outgoing_profiles(&outgoing);
    let accepted = decision(&outgoing, DirectDecisionKind::Accepted, 1, 130);
    profiles
        .record_direct_decision(accepted.clone(), 130)
        .unwrap();
    let receipt = decision_receipt(&accepted, 140);
    profiles
        .record_direct_decision_receipt(receipt.clone())
        .unwrap();
    profiles.direct_contacts[0].request_sent_at = Some(100);
    profiles.direct_contacts[0].last_error = Some("old transport error".into());
    let expected_fingerprint = profiles.direct_contacts[0].expected_fingerprint.clone();
    let expected_node_id = profiles.direct_contacts[0].expected_node_id.clone();
    let remote_device_id = profiles.direct_contacts[0].remote_device_id.clone();
    let remote_public_key = profiles.direct_contacts[0].remote_public_key.clone();
    assert_eq!(pending_envelopes(&profiles.direct_requests, 141).len(), 1);

    profiles
        .invalidate_direct_grants_after_identity_rotation(
            "replacement-lookup",
            150,
            IdentityRepairAction::IdentityReplaced,
        )
        .unwrap();

    assert!(profiles.direct_requests.is_empty());
    assert!(pending_envelopes(&profiles.direct_requests, 150).is_empty());
    assert!(profiles.direct_request_tombstones.is_empty());
    let contact = &profiles.direct_contacts[0];
    assert_eq!(contact.access_state, DirectAccessState::Pending);
    assert_eq!(contact.status, ShareStatus::WaitingForAccess);
    assert!(contact.request_sent_at.is_none());
    assert!(contact.accepted_at.is_none());
    assert!(contact.accepted_public_key.is_none());
    assert!(contact.last_error.is_none());
    assert_eq!(contact.expected_fingerprint, expected_fingerprint);
    assert_eq!(contact.expected_node_id, expected_node_id);
    assert_eq!(contact.remote_device_id, remote_device_id);
    assert_eq!(contact.remote_public_key, remote_public_key);

    let revoke = decision(&outgoing, DirectDecisionKind::Revoked, 2, 160);
    assert!(profiles.record_direct_decision(revoke, 160).is_err());
    assert_eq!(
        profiles.direct_contacts[0].access_state,
        DirectAccessState::Pending
    );
    profiles.validate_direct_ledger().unwrap();
}

#[test]
fn full_identity_replacement_ignores_saturated_permanent_tombstones() {
    let outgoing = request(REQUEST_A, "peer-lookup");
    let incoming = request(REQUEST_B, "local-old");
    let mut profiles = outgoing_profiles(&outgoing);
    profiles
        .record_direct_decision(
            decision(&outgoing, DirectDecisionKind::Accepted, 1, 115),
            115,
        )
        .unwrap();
    profiles
        .delete_direct_request_locally(&outgoing.request_id, 116)
        .unwrap();
    assert_eq!(profiles.direct_request_tombstones[0].retain_until, i64::MAX);
    profiles
        .record_incoming_direct_request("local-old", incoming.clone(), 111)
        .unwrap();
    for index in 0..(MAX_DIRECT_REQUEST_TOMBSTONES - 1) {
        profiles
            .direct_request_tombstones
            .push(tombstone(index, 500));
    }
    assert_eq!(
        profiles.direct_request_tombstones.len(),
        MAX_DIRECT_REQUEST_TOMBSTONES
    );

    profiles
        .invalidate_direct_grants_after_identity_rotation(
            "replacement-lookup",
            120,
            IdentityRepairAction::IdentityReplaced,
        )
        .unwrap();

    assert!(profiles.direct_requests.is_empty());
    assert!(profiles.direct_request_tombstones.is_empty());
    assert_eq!(
        profiles.direct_contacts[0].access_state,
        DirectAccessState::Pending
    );
    assert_eq!(
        profiles.direct_contacts[0].status,
        ShareStatus::WaitingForAccess
    );
    profiles.validate_direct_ledger().unwrap();
}

fn key(byte: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[byte; 32])
}

fn requester() -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret("requester-a", "Requester", &key(1))
}

fn target() -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret("target-a", "Target", &key(2))
}

fn request(request_id: &str, lookup_id: &str) -> SignedDirectRequest {
    let public = key(2).public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(request_id).unwrap(),
        lookup_id,
        requester(),
        DirectPeerIdentity::pinned_target(public.clone(), public_fingerprint(public.as_bytes())),
        100,
        200,
        format!("request-{request_id}"),
        None,
        &SECRET,
        &key(1),
    )
    .unwrap()
}

fn decision(
    request: &SignedDirectRequest,
    kind: DirectDecisionKind,
    revision: u64,
    at: i64,
) -> SignedDirectDecision {
    SignedDirectDecision::sign_with_nonce(
        request,
        target(),
        kind,
        revision,
        at,
        at + 200,
        format!("decision-{revision}"),
        None,
        &SECRET,
        &key(2),
    )
    .unwrap()
}

fn decision_receipt(decision: &SignedDirectDecision, at: i64) -> SignedDirectDecisionReceipt {
    SignedDirectDecisionReceipt::sign_with_nonce(
        decision,
        at,
        "decision-receipt",
        None,
        &SECRET,
        &key(1),
    )
    .unwrap()
}

fn outgoing_profiles(request: &SignedDirectRequest) -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(contact());
    profiles
        .queue_outgoing_direct_request("contact-a", request.clone())
        .unwrap();
    profiles
}

fn contact() -> DirectContact {
    let target = target();
    DirectContact {
        id: "contact-a".into(),
        display_name: "Target".into(),
        lookup_id: "peer-lookup".into(),
        expected_fingerprint: target.fingerprint,
        expected_node_id: target.node_id,
        remote_device_id: None,
        remote_public_key: None,
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::WaitingForAccess,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Pending,
        request_sent_at: Some(100),
        accepted_at: None,
        accepted_public_key: None,
    }
}

fn tombstone(index: usize, retain_until: i64) -> DirectRequestTombstone {
    let deleted_request = request(
        &format!("00000000-0000-4000-8000-{index:012x}"),
        "old-local",
    );
    DirectRequestTombstone {
        request: deleted_request,
        direction: DirectRequestDirection::Incoming,
        contact_id: None,
        decision_state: DirectDecisionState::Pending,
        decision_revision: 0,
        deleted_at: 110,
        retain_until,
        disposition: DirectRequestDeleteDisposition::IncomingDismissed,
    }
}
