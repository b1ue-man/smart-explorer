use super::{decision_label, request_views, timestamp, transport_state};
use crate::share::{
    DirectDecisionKind, DirectDecisionState, DirectEnvelopeKind, DirectFailure, DirectPeerIdentity,
    DirectRelayOutcome, DirectRequestDirection, DirectRequestEntry, DirectRequestId,
    DirectRequestRecord, DirectRequestRetries, DirectRetryState, ShareProfiles,
    SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};

const SECRET: [u8; 32] = [42; 32];

#[test]
fn local_transport_never_calls_relay_forwarding_peer_received() {
    let mut retry = DirectRetryState::default();
    assert!(transport_state(&retry).starts_with("queued"));
    retry.attempt_count = 1;
    retry.last_attempt_at = Some(10);
    assert!(transport_state(&retry).starts_with("sent"));
    retry.relay_outcome = Some(DirectRelayOutcome::Forwarded);
    let label = transport_state(&retry);
    assert!(label.starts_with("relay_forwarded"));
    assert!(label.contains("nicht bestaetigt"));
    retry.last_error = Some(DirectFailure {
        code: "offline".into(),
        message: "peer offline".into(),
    });
    assert!(retry.last_error.is_some());
}

#[test]
fn decision_and_timestamp_labels_are_stable() {
    assert_eq!(decision_label(DirectDecisionState::Accepted), "accepted");
    assert_eq!(decision_label(DirectDecisionState::Revoked), "revoked");
    assert_eq!(timestamp(0), "1970-01-01 00:00:00 UTC");
}

#[test]
fn outgoing_projection_separates_relay_forwarding_from_peer_receipt() {
    let requester_key = iroh::SecretKey::from_bytes(&[21; 32]);
    let target_key = iroh::SecretKey::from_bytes(&[22; 32]);
    let target = DirectPeerIdentity::from_secret("target", "Target", &target_key);
    let request = signed_request(&requester_key, &target_key);
    let mut entry = DirectRequestEntry {
        direction: DirectRequestDirection::Outgoing,
        contact_id: Some("missing-contact".into()),
        local_lookup_id: None,
        record: DirectRequestRecord::new(request),
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: DirectRequestRetries::default(),
    };
    entry.retries.request.attempt_count = 1;
    entry.retries.request.relay_outcome = Some(DirectRelayOutcome::Forwarded);
    let mut profiles = ShareProfiles::default();
    profiles.direct_requests.push(entry);

    let (_, outgoing) = request_views(&profiles, 200);
    let facts = &outgoing[0].facts;
    assert!(facts
        .iter()
        .find(|fact| fact.label == "Lokaler Versand Anfrage")
        .unwrap()
        .value
        .starts_with("relay_forwarded"));
    assert!(facts
        .iter()
        .find(|fact| fact.label == "Peer-Empfang Anfrage")
        .unwrap()
        .value
        .starts_with("unconfirmed"));
    assert_eq!(target.device_name, "Target");
}

#[test]
fn rejected_incoming_request_leaves_open_inbox_then_becomes_deletable() {
    let requester_key = iroh::SecretKey::from_bytes(&[21; 32]);
    let target_key = iroh::SecretKey::from_bytes(&[22; 32]);
    let request = signed_request(&requester_key, &target_key);
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("lookup-target", request.clone(), 110)
        .unwrap();
    let (incoming, _) = request_views(&profiles, 111);
    assert!(incoming[0].can_decide);
    assert!(!incoming[0].can_delete);

    let target = DirectPeerIdentity::from_secret("target", "Target", &target_key);
    let request_receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target.clone(),
        120,
        "request-receipt",
        None,
        &SECRET,
        &target_key,
    )
    .unwrap();
    profiles
        .record_direct_request_receipt(request_receipt)
        .unwrap();
    profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::RequestReceipt,
            DirectRelayOutcome::Forwarded,
            121,
        )
        .unwrap();
    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target,
        DirectDecisionKind::Rejected,
        1,
        130,
        330,
        "decision",
        None,
        &SECRET,
        &target_key,
    )
    .unwrap();
    profiles
        .record_direct_decision(decision.clone(), 130)
        .unwrap();
    let (incoming, _) = request_views(&profiles, 131);
    assert!(!incoming[0].can_decide);
    assert!(!incoming[0].can_delete);

    let receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        140,
        "decision-receipt",
        None,
        &SECRET,
        &requester_key,
    )
    .unwrap();
    profiles.record_direct_decision_receipt(receipt).unwrap();
    let (incoming, _) = request_views(&profiles, 141);
    assert!(incoming[0].can_delete);
}

fn signed_request(
    requester_key: &iroh::SecretKey,
    target_key: &iroh::SecretKey,
) -> SignedDirectRequest {
    let target = DirectPeerIdentity::from_secret("target", "Target", target_key);
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        "lookup-target",
        DirectPeerIdentity::from_secret("requester", "Requester", requester_key),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        100,
        1_000,
        "nonce",
        None,
        &SECRET,
        requester_key,
    )
    .unwrap()
}
