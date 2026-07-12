use super::core::public_fingerprint;
use super::direct_ledger::{
    DirectEnvelopeKind, DirectLedgerError, DirectRelayOutcome, MAX_DIRECT_REQUEST_ENTRIES,
};
use super::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectDecision,
    SignedDirectDecisionReceipt, SignedDirectRequest,
};
use super::profiles::ShareProfiles;
use super::types::{DirectAccessState, DirectContact, ShareStatus};

const SECRET: [u8; 32] = [0x77; 32];

fn key(byte: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[byte; 32])
}

fn request(index: usize, created_at: i64) -> SignedDirectRequest {
    let target_key = key(2);
    let target_public = target_key.public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(format!(
            "123e4567-e89b-42d3-a{:03x}-{:012x}",
            index & 0xfff,
            index
        ))
        .unwrap(),
        "lookup-a",
        DirectPeerIdentity::from_secret("requester-a", "Requester", &key(1)),
        DirectPeerIdentity::pinned_target(
            target_public.clone(),
            public_fingerprint(target_public.as_bytes()),
        ),
        created_at,
        created_at + 100,
        format!("nonce-{index}"),
        None,
        &SECRET,
        &key(1),
    )
    .unwrap()
}

fn profiles() -> ShareProfiles {
    let target = DirectPeerIdentity::from_secret("target-a", "Target", &key(2));
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(DirectContact {
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
    });
    profiles
}

fn fill_pending(profiles: &mut ShareProfiles, start: usize, end: usize) {
    for index in start..end {
        profiles
            .queue_outgoing_direct_request("contact-a", request(index, 100))
            .unwrap();
    }
}

#[test]
fn full_ledger_never_evicts_pending_requests() {
    let mut profiles = profiles();
    fill_pending(&mut profiles, 0, MAX_DIRECT_REQUEST_ENTRIES);
    let error = profiles
        .queue_outgoing_direct_request("contact-a", request(MAX_DIRECT_REQUEST_ENTRIES, 200))
        .unwrap_err();
    assert_eq!(error, DirectLedgerError::LedgerFull);
    assert_eq!(profiles.direct_requests.len(), MAX_DIRECT_REQUEST_ENTRIES);
}

#[test]
fn capacity_prunes_terminal_inactive_history_before_new_request() {
    let mut profiles = profiles();
    let terminal = request(0, 100);
    profiles
        .queue_outgoing_direct_request("contact-a", terminal.clone())
        .unwrap();
    let rejected = SignedDirectDecision::sign_with_nonce(
        &terminal,
        DirectPeerIdentity::from_secret("target-a", "Target", &key(2)),
        DirectDecisionKind::Rejected,
        1,
        130,
        300,
        "decision-nonce",
        None,
        &SECRET,
        &key(2),
    )
    .unwrap();
    profiles
        .record_direct_decision(rejected.clone(), 140)
        .unwrap();
    let receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &rejected,
        150,
        "receipt-nonce",
        None,
        &SECRET,
        &key(1),
    )
    .unwrap();
    profiles.record_direct_decision_receipt(receipt).unwrap();
    profiles
        .record_direct_relay_ack(
            &terminal.request_id,
            DirectEnvelopeKind::DecisionReceipt,
            DirectRelayOutcome::Forwarded,
            151,
        )
        .unwrap();
    fill_pending(&mut profiles, 1, MAX_DIRECT_REQUEST_ENTRIES);

    let replacement = request(MAX_DIRECT_REQUEST_ENTRIES, 200);
    assert!(profiles
        .queue_outgoing_direct_request("contact-a", replacement.clone())
        .unwrap());
    assert_eq!(profiles.direct_requests.len(), MAX_DIRECT_REQUEST_ENTRIES);
    assert!(profiles.direct_request(&terminal.request_id).is_none());
    assert!(profiles.direct_request(&replacement.request_id).is_some());
    assert!(
        profiles
            .direct_requests
            .iter()
            .filter(|entry| entry.record.decision.state == super::DirectDecisionState::Pending)
            .count()
            >= MAX_DIRECT_REQUEST_ENTRIES - 1
    );
}
