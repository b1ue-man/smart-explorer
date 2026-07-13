use super::*;
use crate::share::core::public_fingerprint;
use crate::share::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, SignedDirectDecision, SignedDirectDecisionReceipt,
};
use crate::share::tracked_signal_outbox::pending_envelopes;
use crate::share::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, ShareStatus,
};

const SECRET: [u8; 32] = [0x66; 32];
const REQUEST_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

#[test]
fn incoming_pending_delete_is_durable_and_replay_stays_hidden() {
    let request = request();
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("lookup-a", request.clone(), 110)
        .unwrap();
    assert!(
        !pending_envelopes(&profiles.direct_requests, 120).is_empty()
            || profiles.direct_request(&request.request_id).is_some()
    );

    assert!(profiles
        .delete_direct_request_locally(&request.request_id, 120)
        .unwrap());
    assert!(profiles.direct_requests.is_empty());
    assert!(pending_envelopes(&profiles.direct_requests, 120).is_empty());
    assert_eq!(
        profiles.direct_request_tombstones[0].disposition,
        DirectRequestDeleteDisposition::IncomingDismissed
    );

    let encoded = serde_json::to_string_pretty(&profiles).unwrap();
    let mut restored: ShareProfiles = serde_json::from_str(&encoded).unwrap();
    restored.validate_direct_ledger().unwrap();
    assert!(!restored
        .record_incoming_direct_request("lookup-a", request, 121)
        .unwrap());
    assert!(restored.direct_requests.is_empty());
}

#[test]
fn outgoing_pending_delete_stops_retry_and_late_accept_cannot_grant() {
    let request = request();
    let mut profiles = outgoing_profiles(&request);
    assert_eq!(pending_envelopes(&profiles.direct_requests, 110).len(), 1);

    profiles
        .delete_direct_request_locally(&request.request_id, 120)
        .unwrap();
    assert!(pending_envelopes(&profiles.direct_requests, 120).is_empty());
    assert_eq!(
        profiles.direct_request_tombstones[0].disposition,
        DirectRequestDeleteDisposition::OutgoingCancelled
    );

    let accepted = decision(&request, DirectDecisionKind::Accepted, 1, 130);
    assert!(!profiles.record_direct_decision(accepted, 131).unwrap());
    assert!(profiles.direct_grants.is_empty());
    assert_eq!(
        profiles.direct_contacts[0].access_state,
        DirectAccessState::Pending
    );
    assert_eq!(
        profiles.queue_outgoing_direct_request("contact-a", request),
        Err(DirectLedgerError::RequestIdConflict)
    );
}

#[test]
fn accepted_incoming_history_requires_delivered_signed_revoke_before_deletion() {
    let request = request();
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("lookup-a", request.clone(), 110)
        .unwrap();
    profiles
        .record_direct_decision(
            decision(&request, DirectDecisionKind::Accepted, 1, 130),
            130,
        )
        .unwrap();
    profiles.direct_grants[0].exec.enabled = true;

    assert_eq!(
        profiles.delete_direct_request_locally(&request.request_id, 140),
        Err(DirectLedgerError::ActiveGrantRequiresRevoke)
    );
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Accepted);
    assert!(profiles.direct_grants[0].exec.enabled);
    assert!(profiles.direct_request(&request.request_id).is_some());
    assert!(profiles.direct_request_tombstones.is_empty());

    let revoked = decision(&request, DirectDecisionKind::Revoked, 2, 150);
    profiles
        .record_direct_decision(revoked.clone(), 150)
        .unwrap();
    assert_eq!(
        profiles.delete_direct_request_locally(&request.request_id, 151),
        Err(DirectLedgerError::PendingPeerDelivery)
    );
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Ignored);
    assert!(profiles.direct_request_tombstones.is_empty());

    profiles
        .record_direct_decision_receipt(decision_receipt(&revoked, 160))
        .unwrap();
    assert!(profiles
        .delete_direct_request_locally(&request.request_id, 161)
        .unwrap());
    assert!(profiles.direct_requests.is_empty());
    assert_eq!(
        profiles.direct_request_tombstones[0].disposition,
        DirectRequestDeleteDisposition::HistoryDeleted
    );
    assert_eq!(
        profiles.direct_request_tombstones[0].decision_state,
        DirectDecisionState::Revoked
    );
}

#[test]
fn deleted_outgoing_accept_still_applies_a_newer_signed_remote_revocation() {
    let request = request();
    let mut profiles = outgoing_profiles(&request);
    profiles
        .record_direct_decision(
            decision(&request, DirectDecisionKind::Accepted, 1, 130),
            130,
        )
        .unwrap();
    profiles.direct_grants.push(unrelated_incoming_grant());
    assert_eq!(
        profiles.direct_contacts[0].access_state,
        DirectAccessState::Accepted
    );

    profiles
        .delete_direct_request_locally(&request.request_id, 140)
        .unwrap();
    assert_eq!(profiles.direct_request_tombstones[0].retain_until, i64::MAX);
    assert!(profiles
        .record_direct_decision(decision(&request, DirectDecisionKind::Revoked, 2, 150), 150,)
        .unwrap());

    assert_eq!(
        profiles.direct_contacts[0].access_state,
        DirectAccessState::Ignored
    );
    assert_eq!(
        profiles.direct_request_tombstones[0].decision_state,
        crate::share::DirectDecisionState::Revoked
    );
    // The peer's revocation concerns our outgoing access to it. It must not
    // silently revoke the independent incoming grant that peer has to us.
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Accepted);
    assert!(profiles.direct_grants[0].exec.enabled);
}

#[test]
fn saturated_unexpired_tombstones_fail_before_removing_the_request() {
    let request = request();
    let mut profiles = outgoing_profiles(&request);
    for index in 0..MAX_DIRECT_REQUEST_TOMBSTONES {
        profiles
            .direct_request_tombstones
            .push(tombstone(index, 500));
    }

    assert_eq!(
        profiles.delete_direct_request_locally(&request.request_id, 120),
        Err(DirectLedgerError::TombstoneFull)
    );
    assert!(profiles.direct_request(&request.request_id).is_some());
    assert_eq!(
        profiles.direct_request_tombstones.len(),
        MAX_DIRECT_REQUEST_TOMBSTONES
    );
}

#[test]
fn expired_tombstones_are_pruned_before_capacity_is_reused() {
    let request = request();
    let mut profiles = outgoing_profiles(&request);
    for index in 0..MAX_DIRECT_REQUEST_TOMBSTONES {
        profiles
            .direct_request_tombstones
            .push(tombstone(index, 119));
    }

    assert!(profiles
        .delete_direct_request_locally(&request.request_id, 120)
        .unwrap());
    assert_eq!(profiles.direct_request_tombstones.len(), 1);
    assert_eq!(
        profiles.direct_request_tombstones[0].request.request_id,
        request.request_id
    );
}

#[test]
fn deletion_has_no_half_persisted_snapshot() {
    let request = request();
    let before = outgoing_profiles(&request);
    let mut candidate = before.clone();
    candidate
        .delete_direct_request_locally(&request.request_id, 120)
        .unwrap();
    candidate.validate_direct_ledger().unwrap();

    assert!(before.direct_request(&request.request_id).is_some());
    assert!(before.direct_request_tombstones.is_empty());
    assert!(candidate.direct_request(&request.request_id).is_none());
    assert!(candidate
        .direct_request_tombstone(&request.request_id)
        .is_some());
}

#[test]
fn failed_durable_commit_keeps_visible_entry_and_tombstone_absent() {
    let request = request();
    let mut live = outgoing_profiles(&request);
    let mut candidate = live.clone();
    candidate
        .delete_direct_request_locally(&request.request_id, 120)
        .unwrap();
    let encoded = serde_json::to_string(&live).unwrap();
    let mut storage = V5Storage(Some(encoded), true);

    assert!(live
        .persist_replacement_with(candidate, &mut storage)
        .is_err());
    assert!(live.direct_request(&request.request_id).is_some());
    assert!(live.direct_request_tombstones.is_empty());
}

#[test]
fn v5_profile_migration_preserves_exec_and_defaults_tombstones_empty() {
    let mut previous = ShareProfiles::default();
    previous.direct_grants.push(unrelated_incoming_grant());
    let mut value = serde_json::to_value(&previous).unwrap();
    value["schema_version"] = serde_json::json!(5);
    value
        .as_object_mut()
        .unwrap()
        .remove("direct_request_tombstones");
    let mut storage = V5Storage(Some(serde_json::to_string(&value).unwrap()), false);

    let migrated = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    assert_eq!(migrated.schema_version, 7);
    assert!(migrated.direct_grants[0].exec.enabled);
    assert!(migrated.direct_request_tombstones.is_empty());
}

struct V5Storage(Option<String>, bool);

impl crate::share::profile_persistence::ProfilePersistence for V5Storage {
    fn load_profiles(&mut self) -> Result<Option<String>, String> {
        Ok(self.0.clone())
    }

    fn save_profiles(
        &mut self,
        contents: &str,
        _expected: &crate::share::ProfileRevision,
    ) -> Result<crate::share::ProfileRevision, String> {
        if self.1 {
            return Err("injected crash before atomic replace".into());
        }
        self.0 = Some(contents.into());
        Ok(crate::share::ProfileRevision::from_contents(contents))
    }

    fn save_secret(&mut self, _account: &str, _secret: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete_secret(&mut self, _account: &str) -> Result<(), String> {
        Ok(())
    }
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

fn request() -> SignedDirectRequest {
    request_for(REQUEST_ID, "lookup-a")
}

fn request_for(request_id: &str, lookup_id: &str) -> SignedDirectRequest {
    let public = key(2).public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(request_id).unwrap(),
        lookup_id,
        requester(),
        DirectPeerIdentity::pinned_target(public.clone(), public_fingerprint(public.as_bytes())),
        100,
        200,
        "request-nonce",
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
        "decision-nonce",
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
        "decision-receipt-nonce",
        None,
        &SECRET,
        &key(1),
    )
    .unwrap()
}

fn unrelated_incoming_grant() -> DirectGrant {
    let peer = target();
    DirectGrant {
        device_id: peer.device_id,
        device_name: peer.device_name,
        public_key: peer.public_key,
        fingerprint: peer.fingerprint,
        node_id: peer.node_id,
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: crate::share::ExecGrant {
            enabled: true,
            policy_revision: 1,
            ..crate::share::ExecGrant::default()
        },
    }
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
    }
}

fn tombstone(index: usize, retain_until: i64) -> DirectRequestTombstone {
    let mut deleted_request = request();
    deleted_request.request_id =
        DirectRequestId::parse(format!("00000000-0000-4000-8000-{index:012x}")).unwrap();
    DirectRequestTombstone {
        request: deleted_request,
        direction: DirectRequestDirection::Incoming,
        contact_id: None,
        decision_state: crate::share::DirectDecisionState::Pending,
        decision_revision: 0,
        deleted_at: 110,
        retain_until,
        disposition: DirectRequestDeleteDisposition::IncomingDismissed,
    }
}
