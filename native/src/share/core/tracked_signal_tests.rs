use super::direct_ledger::{
    DirectEnvelopeKind, DirectRelayOutcome, DirectRequestDirection, DirectRequestEntry,
    DirectRequestRetries,
};
use super::direct_lifecycle::DirectRequestRecord;
use super::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectDecision,
    SignedDirectDecisionReceipt, SignedDirectRequest, SignedDirectRequestReceipt,
};
use super::profiles::ShareProfiles;
use super::tracked_signal_dispatch::parse_tracked_server_message;
use super::tracked_signal_outbox::{pending_envelopes, TrackedOutboxEnvelope};
use super::tracked_signal_verify::{
    verify_decision_for_requester, verify_decision_receipt_for_target, verify_request_for_target,
    verify_request_receipt_for_requester,
};
use super::wire::{DirectRoute, DirectRouteOutcome, TrackedDirectServerMsg};

const SECRET: &[u8] = b"tracked-direct-test-relation-key";
const CREATED: i64 = 1_000;
const EXPIRES: i64 = 2_000;

#[test]
fn duplicate_signed_requests_remain_valid_for_idempotent_persistence() {
    let requester_key = key(1);
    let target_key = key(2);
    let requester = peer("requester", &requester_key);
    let target = peer("target", &target_key);
    let request = signed_request(&requester_key, &requester, &target);

    verify_request_for_target(&request, "lookup-target", &target, SECRET, CREATED + 1).unwrap();
    verify_request_for_target(&request, "lookup-target", &target, SECRET, CREATED + 1).unwrap();
    assert!(verify_request_for_target(
        &request,
        "lookup-target",
        &target,
        b"wrong-secret",
        CREATED + 1
    )
    .is_err());
}

#[test]
fn every_peer_confirmation_is_bound_to_request_and_local_identity() {
    let requester_key = key(3);
    let target_key = key(4);
    let requester = peer("requester", &requester_key);
    let target = peer("target", &target_key);
    let request = signed_request(&requester_key, &requester, &target);
    let receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target.clone(),
        CREATED + 2,
        "receipt-nonce",
        None,
        SECRET,
        &target_key,
    )
    .unwrap();
    verify_request_receipt_for_requester(&receipt, &request, &requester, SECRET, CREATED + 3)
        .unwrap();

    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target.clone(),
        DirectDecisionKind::Accepted,
        1,
        CREATED + 4,
        EXPIRES,
        "decision-nonce",
        None,
        SECRET,
        &target_key,
    )
    .unwrap();
    verify_decision_for_requester(&decision, &request, &requester, SECRET, CREATED + 5).unwrap();

    let decision_receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        CREATED + 6,
        "decision-receipt-nonce",
        None,
        SECRET,
        &requester_key,
    )
    .unwrap();
    verify_decision_receipt_for_target(&decision_receipt, &decision, &target, SECRET, CREATED + 7)
        .unwrap();

    let other_local = peer("other", &key(9));
    assert!(
        verify_decision_for_requester(&decision, &request, &other_local, SECRET, CREATED + 5)
            .is_err()
    );
}

#[test]
fn outbox_resends_the_same_signed_request_until_peer_receipt() {
    let requester_key = key(5);
    let target_key = key(6);
    let requester = peer("requester", &requester_key);
    let target = peer("target", &target_key);
    let request = signed_request(&requester_key, &requester, &target);
    let mut entry = outgoing_entry(request.clone());

    let first = pending_envelopes(&[entry.clone()], CREATED + 1);
    let second = pending_envelopes(&[entry.clone()], CREATED + 1);
    assert_eq!(first, second);
    assert!(matches!(
        &first[0].envelope,
        TrackedOutboxEnvelope::Request(value) if value == &request
    ));

    entry.request_receipt = Some(
        SignedDirectRequestReceipt::sign_with_nonce(
            &request,
            target,
            CREATED + 2,
            "receipt-nonce",
            None,
            SECRET,
            &target_key,
        )
        .unwrap(),
    );
    assert!(pending_envelopes(&[entry], CREATED + 3).is_empty());
}

#[test]
fn duplicate_decision_requeues_a_final_receipt_lost_after_relay_ack() {
    let requester_key = key(7);
    let target_key = key(8);
    let requester = peer("requester", &requester_key);
    let target = peer("target", &target_key);
    let request = signed_request(&requester_key, &requester, &target);
    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target,
        DirectDecisionKind::Accepted,
        1,
        CREATED + 2,
        EXPIRES,
        "decision-nonce",
        None,
        SECRET,
        &target_key,
    )
    .unwrap();
    let receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        CREATED + 3,
        "decision-receipt-nonce",
        None,
        SECRET,
        &requester_key,
    )
    .unwrap();
    let mut profiles = ShareProfiles::default();
    profiles.direct_requests.push(outgoing_entry(request));
    profiles
        .record_direct_decision(decision.clone(), CREATED + 3)
        .unwrap();
    profiles.direct_requests[0].decision_receipt = Some(receipt);
    profiles
        .record_direct_relay_ack(
            &request_id(),
            DirectEnvelopeKind::DecisionReceipt,
            DirectRelayOutcome::Forwarded,
            CREATED + 4,
        )
        .unwrap();
    assert!(profiles.direct_requests[0]
        .pending_outboxes(CREATED + 5)
        .is_empty());

    assert!(profiles
        .record_direct_decision(decision, CREATED + 6)
        .unwrap());
    assert_eq!(
        profiles.direct_requests[0].pending_outboxes(CREATED + 6),
        vec![DirectEnvelopeKind::DecisionReceipt]
    );
}

#[test]
fn tracked_wire_parser_distinguishes_legacy_and_correlated_ack() {
    assert!(parse_tracked_server_message(r#"{"t":"pong"}"#)
        .unwrap()
        .is_none());
    let request_id = request_id();
    let line = format!(
        r#"{{"t":"direct_route_ack","request_id":"{request_id}","route":"decision","outcome":"target_offline"}}"#
    );
    assert_eq!(
        parse_tracked_server_message(&line).unwrap().unwrap(),
        TrackedDirectServerMsg::RouteAck {
            request_id,
            route: DirectRoute::Decision,
            outcome: DirectRouteOutcome::TargetOffline,
        }
    );
}

fn outgoing_entry(request: SignedDirectRequest) -> DirectRequestEntry {
    DirectRequestEntry {
        direction: DirectRequestDirection::Outgoing,
        contact_id: Some("contact-target".into()),
        local_lookup_id: None,
        record: DirectRequestRecord::new(request),
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: DirectRequestRetries::default(),
    }
}

fn signed_request(
    requester_key: &iroh::SecretKey,
    requester: &DirectPeerIdentity,
    target: &DirectPeerIdentity,
) -> SignedDirectRequest {
    SignedDirectRequest::sign_with_nonce(
        request_id(),
        "lookup-target",
        requester.clone(),
        DirectPeerIdentity::pinned_target(target.node_id.clone(), target.fingerprint.clone()),
        CREATED,
        EXPIRES,
        "request-nonce",
        None,
        SECRET,
        requester_key,
    )
    .unwrap()
}

fn request_id() -> DirectRequestId {
    DirectRequestId::parse("123e4567-e89b-42d3-a456-426614174000").unwrap()
}

fn key(byte: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[byte; 32])
}

fn peer(device_id: &str, key: &iroh::SecretKey) -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret(device_id, format!("{device_id} name"), key)
}
