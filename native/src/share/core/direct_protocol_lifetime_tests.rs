use super::*;

const NOW: i64 = 1_000_000;
const RELATION_SECRET: [u8; 32] = [0x5a; 32];
const REQUEST_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

#[test]
fn maximum_lifetime_and_clock_skew_are_accepted_for_every_envelope() {
    let timestamp = NOW + MAX_TRACKED_DIRECT_CLOCK_SKEW_SECS;
    let expires_at = timestamp + MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS;
    let request = request(timestamp, expires_at).unwrap();
    request.verify_at(&RELATION_SECRET, NOW).unwrap();

    let receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target(),
        timestamp,
        "request-receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    receipt.verify_for(&request, &RELATION_SECRET, NOW).unwrap();

    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target(),
        DirectDecisionKind::Accepted,
        1,
        timestamp,
        expires_at,
        "decision-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    decision
        .verify_for(&request, &RELATION_SECRET, NOW)
        .unwrap();

    let decision_receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        timestamp,
        "decision-receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(1),
    )
    .unwrap();
    decision_receipt
        .verify_for(&decision, &RELATION_SECRET, NOW)
        .unwrap();
}

#[test]
fn overlong_and_i64_max_envelopes_fail_before_authentication() {
    let overlong = NOW + MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS + 1;
    assert_eq!(
        request(NOW, overlong),
        Err(DirectProtocolError::LifetimeExceeded)
    );
    assert_eq!(
        request(NOW, i64::MAX),
        Err(DirectProtocolError::LifetimeExceeded)
    );

    let valid_expires = NOW + MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS;
    let valid_request = request(NOW, valid_expires).unwrap();
    let mut receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &valid_request,
        target(),
        NOW,
        "request-receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    receipt.expires_at = overlong;
    assert_eq!(
        receipt.verify_for(&valid_request, &RELATION_SECRET, NOW),
        Err(DirectProtocolError::LifetimeExceeded)
    );

    assert_eq!(
        SignedDirectDecision::sign_with_nonce(
            &valid_request,
            target(),
            DirectDecisionKind::Accepted,
            1,
            NOW,
            overlong,
            "overlong-decision",
            None,
            &RELATION_SECRET,
            &key(2),
        ),
        Err(DirectProtocolError::LifetimeExceeded)
    );
    let decision = SignedDirectDecision::sign_with_nonce(
        &valid_request,
        target(),
        DirectDecisionKind::Accepted,
        1,
        NOW,
        valid_expires,
        "decision-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    let mut decision_receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        NOW,
        "decision-receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(1),
    )
    .unwrap();
    decision_receipt.expires_at = overlong;
    assert_eq!(
        decision_receipt.verify_for(&decision, &RELATION_SECRET, NOW),
        Err(DirectProtocolError::LifetimeExceeded)
    );
}

#[test]
fn timestamps_beyond_clock_skew_fail_for_every_envelope() {
    let future = NOW + MAX_TRACKED_DIRECT_CLOCK_SKEW_SECS + 1;
    let future_expires = future + MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS;
    let future_request = request(future, future_expires).unwrap();
    assert_eq!(
        future_request.verify_at(&RELATION_SECRET, NOW),
        Err(DirectProtocolError::TimestampTooFarFuture)
    );

    let request = request(NOW, NOW + MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS).unwrap();
    let receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target(),
        future,
        "request-receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    assert_eq!(
        receipt.verify_for(&request, &RELATION_SECRET, NOW),
        Err(DirectProtocolError::TimestampTooFarFuture)
    );

    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target(),
        DirectDecisionKind::Accepted,
        1,
        future,
        future_expires,
        "decision-nonce",
        None,
        &RELATION_SECRET,
        &key(2),
    )
    .unwrap();
    assert_eq!(
        decision.verify_for(&request, &RELATION_SECRET, NOW),
        Err(DirectProtocolError::TimestampTooFarFuture)
    );

    let decision_receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        future,
        "decision-receipt-nonce",
        None,
        &RELATION_SECRET,
        &key(1),
    )
    .unwrap();
    assert_eq!(
        decision_receipt.verify_for(&decision, &RELATION_SECRET, NOW),
        Err(DirectProtocolError::TimestampTooFarFuture)
    );
}

fn request(created_at: i64, expires_at: i64) -> Result<SignedDirectRequest, DirectProtocolError> {
    let target = target();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(REQUEST_ID).unwrap(),
        "lookup",
        DirectPeerIdentity::from_secret("requester", "Requester", &key(1)),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        created_at,
        expires_at,
        "request-nonce",
        None,
        &RELATION_SECRET,
        &key(1),
    )
}

fn target() -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret("target", "Target", &key(2))
}

fn key(byte: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[byte; 32])
}
