use super::direct_ledger::{
    DirectEnvelopeKind, DirectLedgerError, DirectRelayOutcome, DirectRequestDirection,
};
use super::direct_lifecycle::{
    DirectDecisionDeliveryState, DirectDecisionState, DirectDeliveryState, DirectFailure,
};
use super::direct_protocol::DirectDecisionKind;
use super::profiles::ShareProfiles;
use super::types::{DirectAccessState, DirectGrantState};

#[path = "direct_ledger_fixture.rs"]
mod fixture;
use fixture::{decision, decision_receipt, outgoing_profiles, request, request_receipt};

#[test]
fn outgoing_request_retains_every_signed_envelope_and_peer_confirmed_state() {
    let request = request(Some("please allow"));
    let mut profiles = outgoing_profiles(&request);
    assert!(!profiles
        .queue_outgoing_direct_request("contact-a", request.clone())
        .unwrap());

    assert!(profiles
        .record_direct_attempt(
            &request.request_id,
            DirectEnvelopeKind::Request,
            1,
            110,
            None,
        )
        .unwrap());
    assert!(profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::Request,
            DirectRelayOutcome::Forwarded,
            125,
        )
        .unwrap());
    assert!(profiles
        .record_direct_request_receipt(request_receipt(&request))
        .unwrap());

    let accepted = decision(&request, DirectDecisionKind::Accepted, 1, 130);
    assert!(profiles
        .record_direct_decision(accepted.clone(), 140)
        .unwrap());
    let accepted_receipt = decision_receipt(&accepted, 150);
    assert!(profiles
        .record_direct_decision_receipt(accepted_receipt.clone())
        .unwrap());

    let entry = profiles.direct_request(&request.request_id).unwrap();
    assert_eq!(entry.direction, DirectRequestDirection::Outgoing);
    assert_eq!(entry.record.delivery.state, DirectDeliveryState::Received);
    assert_eq!(entry.record.delivery.changed_at, 125);
    assert_eq!(entry.record.decision.state, DirectDecisionState::Accepted);
    assert_eq!(
        entry.record.decision_delivery.state,
        DirectDecisionDeliveryState::Received
    );
    assert_eq!(entry.request_receipt, Some(request_receipt(&request)));
    assert_eq!(entry.decision, Some(accepted.clone()));
    assert_eq!(entry.decision_receipt, Some(accepted_receipt.clone()));
    assert_eq!(
        entry.pending_outboxes(151),
        vec![DirectEnvelopeKind::DecisionReceipt]
    );
    assert_eq!(
        profiles.direct_contacts[0].access_state,
        DirectAccessState::Accepted
    );
    assert_eq!(
        profiles.direct_contacts[0].remote_device_id.as_deref(),
        Some("target-a")
    );
    profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::DecisionReceipt,
            DirectRelayOutcome::Forwarded,
            151,
        )
        .unwrap();
    assert_eq!(
        profiles
            .direct_request(&request.request_id)
            .unwrap()
            .pending_outboxes(152),
        Vec::<DirectEnvelopeKind>::new()
    );
    assert!(profiles.record_direct_decision(accepted, 152).unwrap());
    assert_eq!(
        profiles
            .direct_request(&request.request_id)
            .unwrap()
            .pending_outboxes(152),
        vec![DirectEnvelopeKind::DecisionReceipt]
    );

    let encoded = serde_json::to_string_pretty(&profiles).unwrap();
    let restored: ShareProfiles = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored.direct_requests, profiles.direct_requests);
}

#[test]
fn incoming_decision_outbox_survives_accept_and_newer_revoke() {
    let request = request(None);
    let mut profiles = ShareProfiles::default();
    assert!(profiles
        .record_incoming_direct_request("lookup-a", request.clone(), 110)
        .unwrap());
    assert!(!profiles
        .record_incoming_direct_request("lookup-a", request.clone(), 110)
        .unwrap());
    let request_receipt = request_receipt(&request);
    profiles
        .record_direct_request_receipt(request_receipt)
        .unwrap();
    profiles
        .record_direct_attempt(
            &request.request_id,
            DirectEnvelopeKind::RequestReceipt,
            1,
            121,
            None,
        )
        .unwrap();
    profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::RequestReceipt,
            DirectRelayOutcome::Forwarded,
            122,
        )
        .unwrap();
    assert!(profiles
        .direct_request(&request.request_id)
        .unwrap()
        .pending_outboxes(123)
        .is_empty());
    assert!(profiles
        .record_incoming_direct_request("lookup-a", request.clone(), 123)
        .unwrap());
    assert_eq!(
        profiles
            .direct_request(&request.request_id)
            .unwrap()
            .pending_outboxes(123),
        vec![DirectEnvelopeKind::RequestReceipt]
    );

    let accepted = decision(&request, DirectDecisionKind::Accepted, 1, 130);
    profiles
        .record_direct_decision(accepted.clone(), 130)
        .unwrap();
    profiles
        .record_direct_attempt(
            &request.request_id,
            DirectEnvelopeKind::Decision,
            1,
            135,
            None,
        )
        .unwrap();
    profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::Decision,
            DirectRelayOutcome::Forwarded,
            145,
        )
        .unwrap();
    let accepted_receipt = decision_receipt(&accepted, 140);
    profiles
        .record_direct_decision_receipt(accepted_receipt.clone())
        .unwrap();
    assert_eq!(
        profiles
            .direct_request(&request.request_id)
            .unwrap()
            .record
            .decision_delivery
            .changed_at,
        145
    );
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Accepted);
    // An accepted new device may write (Direct default since 2026-10-09).
    assert!(profiles.direct_grants[0].write);
    profiles.direct_grants[0].exec.enabled = true;

    let revoked = decision(&request, DirectDecisionKind::Revoked, 2, 250);
    profiles
        .record_direct_decision(revoked.clone(), 250)
        .unwrap();
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Ignored);
    assert!(!profiles.direct_grants[0].exec.enabled);
    assert_eq!(profiles.direct_grants[0].exec.policy_revision, 1);
    assert_eq!(
        profiles.direct_grants[0].exec.source_request_id.as_ref(),
        Some(&request.request_id)
    );
    assert_eq!(
        profiles.direct_grants[0].exec.source_decision_revision,
        Some(2)
    );
    let entry = profiles.direct_request(&request.request_id).unwrap();
    assert_eq!(entry.decision, Some(revoked));
    assert!(entry.decision_receipt.is_none());
    assert_eq!(
        entry.record.decision_delivery.state,
        DirectDecisionDeliveryState::Queued
    );
    assert_eq!(entry.retries.decision.attempt_count, 0);
    assert!(!profiles
        .record_direct_decision_receipt(accepted_receipt)
        .unwrap());
}

#[test]
fn history_deletion_waits_for_terminal_peer_delivery() {
    let request = request(None);
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("lookup-a", request.clone(), 110)
        .unwrap();
    profiles
        .record_direct_request_receipt(request_receipt(&request))
        .unwrap();
    profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::RequestReceipt,
            DirectRelayOutcome::Forwarded,
            121,
        )
        .unwrap();

    let rejected = decision(&request, DirectDecisionKind::Rejected, 1, 130);
    profiles
        .record_direct_decision(rejected.clone(), 130)
        .unwrap();
    assert!(!profiles
        .direct_request(&request.request_id)
        .unwrap()
        .removable_from_history(131));

    profiles
        .record_direct_decision_receipt(decision_receipt(&rejected, 140))
        .unwrap();
    assert!(profiles
        .direct_request(&request.request_id)
        .unwrap()
        .removable_from_history(141));
}

#[test]
fn retry_and_relay_updates_are_absolute_monotonic_and_idempotent() {
    let request = request(None);
    let mut profiles = outgoing_profiles(&request);
    let failure = DirectFailure {
        code: "network".into(),
        message: "temporary failure".into(),
    };
    assert!(profiles
        .record_direct_attempt(
            &request.request_id,
            DirectEnvelopeKind::Request,
            2,
            110,
            Some(failure),
        )
        .unwrap());
    assert!(!profiles
        .record_direct_attempt(
            &request.request_id,
            DirectEnvelopeKind::Request,
            2,
            110,
            None,
        )
        .unwrap());
    assert!(profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::Request,
            DirectRelayOutcome::TargetOffline,
            112,
        )
        .unwrap());
    assert!(!profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::Request,
            DirectRelayOutcome::TargetOffline,
            112,
        )
        .unwrap());
    let retry = profiles
        .direct_request(&request.request_id)
        .unwrap()
        .retry(DirectEnvelopeKind::Request);
    assert_eq!(retry.attempt_count, 2);
    assert_eq!(retry.relay_outcome, Some(DirectRelayOutcome::TargetOffline));
    assert!(retry.last_error.is_none());
    assert!(profiles
        .retry_direct_envelope_now(&request.request_id, DirectEnvelopeKind::Request, 113,)
        .unwrap());
    assert!(!profiles
        .retry_direct_envelope_now(&request.request_id, DirectEnvelopeKind::Request, 113,)
        .unwrap());
    assert_eq!(
        profiles
            .direct_request(&request.request_id)
            .unwrap()
            .retry(DirectEnvelopeKind::Request),
        &Default::default()
    );
}

#[test]
fn legacy_forwarding_stops_automatic_request_retries_until_manual_retry() {
    let request = request(None);
    let request_id = request.request_id.clone();
    let mut profiles = outgoing_profiles(&request);
    profiles
        .record_direct_attempt(&request_id, DirectEnvelopeKind::Request, 1, 110, None)
        .unwrap();
    profiles
        .record_direct_relay_ack(
            &request_id,
            DirectEnvelopeKind::Request,
            DirectRelayOutcome::LegacyForwarded,
            111,
        )
        .unwrap();

    let entry = profiles.direct_request(&request_id).unwrap();
    assert_eq!(entry.record.delivery.state.code(), "server_queued");
    assert!(entry.pending_outboxes(112).is_empty());
    assert_eq!(
        entry.manually_retryable_outboxes(112),
        vec![DirectEnvelopeKind::Request]
    );

    assert!(profiles
        .retry_direct_envelope_now(&request_id, DirectEnvelopeKind::Request, 113)
        .unwrap());
    assert_eq!(
        profiles
            .direct_request(&request_id)
            .unwrap()
            .pending_outboxes(113),
        vec![DirectEnvelopeKind::Request]
    );
}

#[test]
fn request_ids_and_signed_artifacts_cannot_be_rebound() {
    let original = request(None);
    let mut profiles = outgoing_profiles(&original);
    let different = request(Some("changed body"));
    assert_eq!(
        profiles.queue_outgoing_direct_request("contact-a", different),
        Err(DirectLedgerError::RequestIdConflict)
    );

    let mut receipt = request_receipt(&original);
    receipt.request_digest = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into();
    assert_eq!(
        profiles.record_direct_request_receipt(receipt),
        Err(DirectLedgerError::EnvelopeConflict)
    );
}

#[test]
fn persisted_ledger_validation_rejects_duplicate_request_ids() {
    let request = request(None);
    let mut profiles = outgoing_profiles(&request);
    profiles
        .direct_requests
        .push(profiles.direct_requests[0].clone());
    let error = profiles.validate_direct_ledger().unwrap_err();
    assert!(error.contains("doppelte direkte Request-ID"));
}
