//! S32: a verified envelope stays persistable throughout the accepted skew.
use super::DirectRequestEntry;
use crate::share::direct_lifecycle::DirectDecisionDeliveryState;
use crate::share::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectDecision,
    SignedDirectRequest,
};
use crate::share::ShareProfiles;

const NOW: i64 = 10_000;
const SECRET: [u8; 32] = [7; 32];

fn request(created_at: i64) -> SignedDirectRequest {
    let target = DirectPeerIdentity::from_secret("target", "Target", &key(2));
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        "lookup",
        DirectPeerIdentity::from_secret("requester", "Requester", &key(1)),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        created_at,
        created_at + 600,
        "request-nonce",
        None,
        &SECRET,
        &key(1),
    ).unwrap()
}

fn key(byte: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[byte; 32])
}

#[test]
fn review_task_future_request_is_received_without_waiting_for_local_clock() {
    let request = request(NOW + 90);
    request.verify_at(&SECRET, NOW).unwrap();
    let mut profiles = ShareProfiles::default();
    profiles.record_incoming_direct_request("lookup", request.clone(), NOW).unwrap();
    assert_eq!(profiles.direct_requests[0].record.delivery.changed_at, NOW + 90);
    assert_eq!(profiles.direct_requests[0].record.request, request);
    profiles.validate_direct_ledger().unwrap();
}

#[test]
fn review_task_decision_delivery_accepts_clock_skew_in_both_directions() {
    for (created_at, decided_at) in [(NOW + 90, NOW), (NOW, NOW + 90)] {
        let request = request(created_at);
        let decision = SignedDirectDecision::sign_with_nonce(
            &request,
            DirectPeerIdentity::from_secret("target", "Target", &key(2)),
            DirectDecisionKind::Accepted,
            1,
            decided_at,
            decided_at + 600,
            "decision-nonce",
            None,
            &SECRET,
            &key(2),
        ).unwrap();
        decision.verify_for(&request, &SECRET, NOW).unwrap();
        let mut profiles = ShareProfiles::default();
        profiles.direct_requests.push(DirectRequestEntry::outgoing("contact".into(), request));
        profiles.record_direct_decision(decision.clone(), NOW).unwrap();
        let entry = &profiles.direct_requests[0];
        assert_eq!(entry.record.decision.changed_at, created_at.max(decided_at));
        assert_eq!(entry.record.decision_delivery.changed_at, created_at.max(decided_at));
        assert_eq!(entry.record.decision_delivery.state, DirectDecisionDeliveryState::Received);
        assert_eq!(entry.decision.as_ref(), Some(&decision));
        profiles.validate_direct_ledger().unwrap();
    }
}
