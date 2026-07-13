use super::super::request_selection::tracked_retryable;
use crate::share::{
    DirectContact, DirectPeerIdentity, DirectRelayOutcome, DirectRequestId, ShareProfiles,
    ShareStatus, SignedDirectRequest,
};

#[test]
fn cli_retry_selection_includes_a_legacy_forwarded_request() {
    let requester_secret = iroh::SecretKey::from_bytes(&[3; 32]);
    let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
    let target = DirectPeerIdentity::from_secret("target", "Target", &target_secret);
    let request = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("01234567-89ab-4def-8123-456789abcdef").unwrap(),
        "lookup",
        DirectPeerIdentity::from_secret("requester", "Requester", &requester_secret),
        DirectPeerIdentity::pinned_target(target.node_id.clone(), target.fingerprint.clone()),
        10,
        1_000,
        "request-nonce",
        None,
        &[9; 32],
        &requester_secret,
    )
    .unwrap();
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(DirectContact {
        id: "contact".into(),
        display_name: "Target".into(),
        lookup_id: "lookup".into(),
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
        access_state: crate::share::DirectAccessState::Pending,
        request_sent_at: Some(10),
        accepted_at: None,
        accepted_public_key: None,
    });
    profiles
        .queue_outgoing_direct_request("contact", request)
        .unwrap();
    profiles.direct_requests[0].retries.request.relay_outcome =
        Some(DirectRelayOutcome::LegacyForwarded);

    assert!(profiles.direct_requests[0].pending_outboxes(11).is_empty());
    assert!(tracked_retryable(&profiles.direct_requests[0], 11));
}
