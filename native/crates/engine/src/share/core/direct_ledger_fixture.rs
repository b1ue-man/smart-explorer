use crate::share::core::public_fingerprint;
use crate::share::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectDecision,
    SignedDirectDecisionReceipt, SignedDirectRequest, SignedDirectRequestReceipt,
};
use crate::share::profiles::ShareProfiles;
use crate::share::types::{DirectAccessState, DirectContact, ShareStatus};

const SECRET: [u8; 32] = [0x66; 32];
const REQUEST_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

fn key(byte: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[byte; 32])
}

fn requester() -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret("requester-a", "Requester", &key(1))
}

fn target() -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret("target-a", "Target", &key(2))
}

pub(super) fn request(message: Option<&str>) -> SignedDirectRequest {
    let target_public = key(2).public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(REQUEST_ID).unwrap(),
        "lookup-a",
        requester(),
        DirectPeerIdentity::pinned_target(
            target_public.clone(),
            public_fingerprint(target_public.as_bytes()),
        ),
        100,
        200,
        "request-nonce",
        message.map(str::to_string),
        &SECRET,
        &key(1),
    )
    .unwrap()
}

pub(super) fn request_receipt(request: &SignedDirectRequest) -> SignedDirectRequestReceipt {
    SignedDirectRequestReceipt::sign_with_nonce(
        request,
        target(),
        120,
        "request-receipt-nonce",
        None,
        &SECRET,
        &key(2),
    )
    .unwrap()
}

pub(super) fn decision(
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

pub(super) fn decision_receipt(
    decision: &SignedDirectDecision,
    at: i64,
) -> SignedDirectDecisionReceipt {
    SignedDirectDecisionReceipt::sign_with_nonce(
        decision,
        at,
        format!("decision-receipt-{}", decision.decision_revision),
        None,
        &SECRET,
        &key(1),
    )
    .unwrap()
}

fn contact() -> DirectContact {
    let target = target();
    DirectContact {
        id: "contact-a".into(),
        display_name: "Target".into(),
        lookup_id: "lookup-a".into(),
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
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

pub(super) fn outgoing_profiles(request: &SignedDirectRequest) -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(contact());
    assert!(profiles
        .queue_outgoing_direct_request("contact-a", request.clone())
        .unwrap());
    profiles
}
