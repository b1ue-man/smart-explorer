use super::direct_ledger::{
    DirectEnvelopeKind, DirectLedgerError, DirectRelayOutcome, DirectRetryState,
};
use super::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectDecision,
    SignedDirectDecisionReceipt, SignedDirectRequest, SignedDirectRequestReceipt,
    MAX_TRACKED_DIRECT_CLOCK_SKEW_SECS,
};
use super::profile_persistence::ProfilePersistence;
use super::profiles::ProfileRevision;
use super::profiles::ShareProfiles;

const RELATION_SECRET: [u8; 32] = [9; 32];

#[test]
fn domain_rejects_legacy_forwarding_for_non_request_envelopes() {
    let (mut profiles, request_id) = incoming_profiles();

    assert_eq!(
        profiles.record_direct_relay_ack(
            &request_id,
            DirectEnvelopeKind::RequestReceipt,
            DirectRelayOutcome::LegacyForwarded,
            12,
        ),
        Err(DirectLedgerError::EnvelopeConflict)
    );
}

#[test]
fn persisted_ledger_rejects_impossible_legacy_forwarding_state() {
    let (mut profiles, _) = incoming_profiles();
    profiles.direct_requests[0].retries.request_receipt = DirectRetryState {
        attempt_count: 1,
        last_attempt_at: Some(12),
        relay_outcome: Some(DirectRelayOutcome::LegacyForwarded),
        relay_changed_at: Some(12),
        last_error: None,
    };

    let error = profiles.validate_direct_ledger().unwrap_err();
    assert!(error.contains("envelope conflicts"));
}

#[test]
fn persisted_ledger_rejects_overlong_timestamps_in_every_signed_envelope() {
    let profiles = complete_incoming_profiles();

    let mut overlong_request = profiles.clone();
    overlong_request.direct_requests[0]
        .record
        .request
        .expires_at = i64::MAX;
    assert_lifetime_corruption(overlong_request);

    let mut overlong_receipt = profiles.clone();
    overlong_receipt.direct_requests[0]
        .request_receipt
        .as_mut()
        .unwrap()
        .expires_at = i64::MAX;
    assert_lifetime_corruption(overlong_receipt);

    let mut overlong_decision = profiles.clone();
    overlong_decision.direct_requests[0]
        .decision
        .as_mut()
        .unwrap()
        .expires_at = i64::MAX;
    assert_lifetime_corruption(overlong_decision);

    let mut overlong_decision_receipt = profiles;
    overlong_decision_receipt.direct_requests[0]
        .decision_receipt
        .as_mut()
        .unwrap()
        .expires_at = i64::MAX;
    assert_lifetime_corruption(overlong_decision_receipt);
}

#[test]
fn persisted_ledger_rejects_overlong_tombstone_request() {
    let (mut profiles, request_id) = incoming_profiles();
    profiles
        .delete_direct_request_locally(&request_id, 120)
        .unwrap();
    profiles.direct_request_tombstones[0].request.expires_at = i64::MAX;

    assert_lifetime_corruption(profiles);
}

#[test]
fn persisted_ledger_tolerates_clock_rollback_for_bounded_signed_state() {
    let future = super::core::now_secs()
        .saturating_add(MAX_TRACKED_DIRECT_CLOCK_SKEW_SECS)
        .saturating_add(60);
    let requester_secret = iroh::SecretKey::from_bytes(&[3; 32]);
    let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
    let target = DirectPeerIdentity::from_secret("target", "Target", &target_secret);
    let request = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("fedcba98-7654-4321-8765-4321fedcba98").unwrap(),
        "future-lookup",
        DirectPeerIdentity::from_secret("requester", "Requester", &requester_secret),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        future,
        future.saturating_add(60),
        "future-request-nonce",
        None,
        &RELATION_SECRET,
        &requester_secret,
    )
    .unwrap();
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("future-lookup", request, future)
        .unwrap();

    profiles.validate_direct_ledger().unwrap();
}

#[test]
fn profile_load_fails_closed_with_actionable_lifetime_corruption() {
    let (mut profiles, _) = incoming_profiles();
    profiles.direct_requests[0].record.request.expires_at = i64::MAX;
    let mut storage = ReadOnlyProfile(Some(serde_json::to_string(&profiles).unwrap()));

    let error = ShareProfiles::load_checked_with(None, &mut storage).unwrap_err();

    assert!(error.contains("Share-Profile sind beschaedigt"));
    assert!(error.contains("lifetime_exceeded"));
    assert!(error.contains("ungueltiger direkter Request"));
}

fn assert_lifetime_corruption(profiles: ShareProfiles) {
    let error = profiles.validate_direct_ledger().unwrap_err();
    assert!(error.contains("lifetime_exceeded"), "{error}");
}

fn complete_incoming_profiles() -> ShareProfiles {
    let (mut profiles, request_id) = incoming_profiles();
    let request = profiles
        .direct_request(&request_id)
        .unwrap()
        .record
        .request
        .clone();
    let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
    let target = DirectPeerIdentity::from_secret("target", "Target", &target_secret);
    let receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target.clone(),
        12,
        "receipt-nonce",
        None,
        &RELATION_SECRET,
        &target_secret,
    )
    .unwrap();
    profiles.record_direct_request_receipt(receipt).unwrap();
    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target,
        DirectDecisionKind::Accepted,
        1,
        13,
        200,
        "decision-nonce",
        None,
        &RELATION_SECRET,
        &target_secret,
    )
    .unwrap();
    profiles
        .record_direct_decision(decision.clone(), 13)
        .unwrap();
    let decision_receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        14,
        "decision-receipt-nonce",
        None,
        &RELATION_SECRET,
        &iroh::SecretKey::from_bytes(&[3; 32]),
    )
    .unwrap();
    profiles
        .record_direct_decision_receipt(decision_receipt)
        .unwrap();
    profiles.validate_direct_ledger().unwrap();
    profiles
}

fn incoming_profiles() -> (ShareProfiles, DirectRequestId) {
    let requester_secret = iroh::SecretKey::from_bytes(&[3; 32]);
    let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
    let target = DirectPeerIdentity::from_secret("target", "Target", &target_secret);
    let request_id = DirectRequestId::parse("01234567-89ab-4def-8123-456789abcdef").unwrap();
    let request = SignedDirectRequest::sign_with_nonce(
        request_id.clone(),
        "lookup",
        DirectPeerIdentity::from_secret("requester", "Requester", &requester_secret),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        10,
        100,
        "request-nonce",
        None,
        &RELATION_SECRET,
        &requester_secret,
    )
    .unwrap();
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("lookup", request, 11)
        .unwrap();
    (profiles, request_id)
}

struct ReadOnlyProfile(Option<String>);

impl ProfilePersistence for ReadOnlyProfile {
    fn load_profiles(&mut self) -> Result<Option<String>, String> {
        Ok(self.0.clone())
    }

    fn save_profiles(
        &mut self,
        _contents: &str,
        _expected: &ProfileRevision,
    ) -> Result<ProfileRevision, String> {
        Err("unexpected save".into())
    }

    fn save_secret(&mut self, _account: &str, _secret: &str) -> Result<(), String> {
        Err("unexpected secret save".into())
    }

    fn delete_secret(&mut self, _account: &str) -> Result<(), String> {
        Err("unexpected secret deletion".into())
    }
}
