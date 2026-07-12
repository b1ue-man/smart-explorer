use super::core::public_fingerprint;
use super::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectProtocolError, DirectRequestId,
    SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::wire::{
    DirectRoute, DirectRouteOutcome, TrackedDirectClientMsg, TrackedDirectServerMsg,
    TRACKED_DIRECT_CAPABILITY,
};

const RELATION_SECRET: [u8; 32] = [0x55; 32];
const REQUEST_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

fn key(byte: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[byte; 32])
}

fn identity(device: &str, name: &str, key: &iroh::SecretKey) -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret(device, name, key)
}

fn request_with(request_id: &str, message: Option<String>, nonce: &str) -> SignedDirectRequest {
    let requester_key = key(1);
    let target_key = key(2);
    let target_public = target_key.public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(request_id).unwrap(),
        "lookup-a",
        identity("requester-a", "Requester", &requester_key),
        DirectPeerIdentity::pinned_target(
            target_public.clone(),
            public_fingerprint(target_public.as_bytes()),
        ),
        100,
        200,
        nonce,
        message,
        &RELATION_SECRET,
        &requester_key,
    )
    .unwrap()
}

fn request() -> SignedDirectRequest {
    request_with(
        REQUEST_ID,
        Some("please allow access".into()),
        "request-nonce",
    )
}

fn target_identity() -> DirectPeerIdentity {
    identity("target-a", "Target", &key(2))
}

#[test]
fn request_ids_are_canonical_uuid_v4_values() {
    let parsed = DirectRequestId::parse(REQUEST_ID.to_ascii_uppercase()).unwrap();
    assert_eq!(parsed.as_str(), REQUEST_ID);
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        format!(r#""{REQUEST_ID}""#)
    );
    assert!(serde_json::from_str::<DirectRequestId>(r#""not-a-uuid""#).is_err());
    assert!(DirectRequestId::parse("123e4567-e89b-12d3-a456-426614174000").is_err());
    let generated = DirectRequestId::generate().unwrap();
    assert_eq!(generated.as_str().as_bytes()[14], b'4');
    assert!(matches!(
        generated.as_str().as_bytes()[19],
        b'8' | b'9' | b'a' | b'b'
    ));
}

#[test]
fn legacy_d3_target_pin_can_sign_offline_request() {
    let request = request();
    assert!(request.target.device_id.is_empty());
    assert!(request.target.device_name.is_empty());
    request.verify_at(&RELATION_SECRET, 150).unwrap();
    assert_eq!(request.requester.validate().unwrap(), key(1).public());
    assert_eq!(request.target.validate_pin().unwrap(), key(2).public());
    assert!(request.target.validate().is_err());
}

#[test]
fn request_receipt_decision_and_decision_receipt_verify_end_to_end() {
    let request = request();
    let target = target_identity();
    let request_receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target.clone(),
        120,
        "receipt-nonce",
        Some("persisted".into()),
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    request_receipt
        .verify_for(&request, &RELATION_SECRET, 130)
        .unwrap();

    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target,
        DirectDecisionKind::Accepted,
        1,
        130,
        300,
        "decision-nonce",
        Some("approved".into()),
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    decision
        .verify_for(&request, &RELATION_SECRET, 140)
        .unwrap();

    let decision_receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        150,
        "decision-receipt-nonce",
        Some("applied".into()),
        &RELATION_SECRET,
        &key(1),
    )
    .unwrap();
    decision_receipt
        .verify_for(&decision, &RELATION_SECRET, 160)
        .unwrap();
}

#[test]
fn every_request_field_is_bound_by_hmac_and_requester_signature() {
    let request = request();
    let mut changed = request.clone();
    changed.message = Some("different".into());
    assert_eq!(
        changed.verify_at(&RELATION_SECRET, 150),
        Err(DirectProtocolError::InvalidHmac)
    );

    let mut changed = request.clone();
    changed.expires_at += 1;
    assert_eq!(
        changed.verify_at(&RELATION_SECRET, 150),
        Err(DirectProtocolError::InvalidHmac)
    );

    assert_eq!(
        request.verify_at(&[0x99; 32], 150),
        Err(DirectProtocolError::InvalidHmac)
    );

    let mut changed = request.clone();
    changed.identity_signature = request_with(
        "123e4567-e89b-42d3-b456-426614174001",
        request.message.clone(),
        "other-nonce",
    )
    .identity_signature;
    assert_eq!(
        changed.verify_at(&RELATION_SECRET, 150),
        Err(DirectProtocolError::InvalidSignature)
    );

    let mut changed = request.clone();
    changed.requester.device_id.push_str("-tampered");
    assert_eq!(
        changed.verify_at(&RELATION_SECRET, 150),
        Err(DirectProtocolError::InvalidHmac)
    );

    let mut changed = request;
    changed.target.fingerprint = "00".repeat(16);
    assert_eq!(
        changed.verify_at(&RELATION_SECRET, 150),
        Err(DirectProtocolError::InvalidFingerprint)
    );
}

#[test]
fn decision_and_revision_are_cryptographically_bound() {
    let request = request();
    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target_identity(),
        DirectDecisionKind::Accepted,
        1,
        130,
        300,
        "decision-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();

    let mut changed = decision.clone();
    changed.decision = DirectDecisionKind::Rejected;
    assert_eq!(
        changed.verify_for(&request, &RELATION_SECRET, 140),
        Err(DirectProtocolError::InvalidHmac)
    );
    let mut changed = decision.clone();
    changed.decision_revision = 2;
    assert_eq!(
        changed.verify_for(&request, &RELATION_SECRET, 140),
        Err(DirectProtocolError::InvalidHmac)
    );
}

#[test]
fn receipts_reject_wrong_request_or_decision_digest() {
    let request = request();
    let receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target_identity(),
        120,
        "receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    let other = request_with(
        "123e4567-e89b-42d3-b456-426614174001",
        request.message.clone(),
        "other-nonce",
    );
    assert_eq!(
        receipt.verify_for(&other, &RELATION_SECRET, 130),
        Err(DirectProtocolError::DigestMismatch)
    );

    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target_identity(),
        DirectDecisionKind::Rejected,
        1,
        130,
        300,
        "decision-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    let mut decision_receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        150,
        "decision-receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(1),
    )
    .unwrap();
    decision_receipt.decision_digest = request.digest().unwrap();
    assert_eq!(
        decision_receipt.verify_for(&decision, &RELATION_SECRET, 160),
        Err(DirectProtocolError::DigestMismatch)
    );
}

#[test]
fn expiry_and_signer_mismatch_fail_closed() {
    let request = request();
    assert_eq!(
        request.verify_at(&RELATION_SECRET, 201),
        Err(DirectProtocolError::Expired)
    );
    assert_eq!(
        SignedDirectRequestReceipt::sign_with_nonce(
            &request,
            target_identity(),
            201,
            "late",
            None,
            &RELATION_SECRET,
            &key(2),
        ),
        Err(DirectProtocolError::Expired)
    );
    assert_eq!(
        SignedDirectDecision::sign_with_nonce(
            &request,
            target_identity(),
            DirectDecisionKind::Accepted,
            1,
            201,
            300,
            "late",
            None,
            &RELATION_SECRET,
            &key(2),
        ),
        Err(DirectProtocolError::Expired)
    );

    let requester = identity("requester-a", "Requester", &key(1));
    let target_public = key(2).public().to_string();
    assert_eq!(
        SignedDirectRequest::sign_with_nonce(
            DirectRequestId::parse(REQUEST_ID).unwrap(),
            "lookup-a",
            requester,
            DirectPeerIdentity::pinned_target(
                target_public.clone(),
                public_fingerprint(target_public.as_bytes()),
            ),
            100,
            200,
            "nonce",
            None,
            &RELATION_SECRET,
            &key(3),
        ),
        Err(DirectProtocolError::SignerMismatch)
    );
}

#[test]
fn optional_message_presence_is_bound_without_ambiguity() {
    let absent = request_with(REQUEST_ID, None, "same-nonce");
    let empty = request_with(REQUEST_ID, Some(String::new()), "same-nonce");
    assert_ne!(absent.hmac_proof, empty.hmac_proof);
    assert_ne!(absent.identity_signature, empty.identity_signature);
}

#[test]
fn tracked_wire_tags_match_server_schema() {
    assert_eq!(TRACKED_DIRECT_CAPABILITY, "tracked_direct_v1");
    let request = request();
    let encoded = serde_json::to_value(TrackedDirectClientMsg::Request {
        request: request.clone(),
    })
    .unwrap();
    assert_eq!(encoded["t"], "submit_direct_request");
    assert_eq!(encoded["request"]["request_id"], REQUEST_ID);
    assert!(encoded["request"]["target"]["device_id"]
        .as_str()
        .unwrap()
        .is_empty());

    let ack = serde_json::json!({
        "t": "direct_route_ack",
        "request_id": REQUEST_ID,
        "route": "decision_receipt",
        "outcome": "target_offline"
    });
    assert_eq!(
        serde_json::from_value::<TrackedDirectServerMsg>(ack).unwrap(),
        TrackedDirectServerMsg::RouteAck {
            request_id: request.request_id,
            route: DirectRoute::DecisionReceipt,
            outcome: DirectRouteOutcome::TargetOffline,
        }
    );
}

#[test]
fn every_tracked_wire_payload_uses_the_coordinated_tag() {
    let request = request();
    let receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target_identity(),
        120,
        "receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target_identity(),
        DirectDecisionKind::Accepted,
        1,
        130,
        300,
        "decision-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    let decision_receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        140,
        "decision-receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(1),
    )
    .unwrap();

    let client = [
        (
            TrackedDirectClientMsg::Request {
                request: request.clone(),
            },
            "submit_direct_request",
        ),
        (
            TrackedDirectClientMsg::RequestReceipt {
                receipt: receipt.clone(),
            },
            "submit_direct_request_receipt",
        ),
        (
            TrackedDirectClientMsg::Decision {
                decision: decision.clone(),
            },
            "submit_direct_decision",
        ),
        (
            TrackedDirectClientMsg::DecisionReceipt {
                receipt: decision_receipt.clone(),
            },
            "submit_direct_decision_receipt",
        ),
    ];
    for (message, expected) in client {
        assert_eq!(serde_json::to_value(message).unwrap()["t"], expected);
    }

    let server = [
        (
            TrackedDirectServerMsg::Request { request },
            "direct_request",
        ),
        (
            TrackedDirectServerMsg::RequestReceipt { receipt },
            "direct_request_receipt",
        ),
        (
            TrackedDirectServerMsg::Decision { decision },
            "direct_decision",
        ),
        (
            TrackedDirectServerMsg::DecisionReceipt {
                receipt: decision_receipt,
            },
            "direct_decision_receipt",
        ),
    ];
    for (message, expected) in server {
        assert_eq!(serde_json::to_value(message).unwrap()["t"], expected);
    }
}
