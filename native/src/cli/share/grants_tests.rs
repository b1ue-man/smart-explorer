use super::{grant_selector_matches, latest_accepted_request, related_requests};
use crate::share::{
    DirectDecisionState, DirectGrant, DirectGrantState, DirectPeerIdentity, DirectRequestDirection,
    DirectRequestEntry, DirectRequestId, DirectRequestRecord, DirectRequestRetries, ShareProfiles,
    SignedDirectRequest,
};

const SECRET: [u8; 32] = [42; 32];

#[test]
fn tracked_history_links_to_a_grant_only_for_the_exact_peer_identity() {
    let key_a = iroh::SecretKey::from_bytes(&[21; 32]);
    let key_b = iroh::SecretKey::from_bytes(&[22; 32]);
    let target_key = iroh::SecretKey::from_bytes(&[23; 32]);
    let request_a = entry(&key_a, &target_key, "123e4567-e89b-42d3-a456-426614174000");
    let request_b = entry(&key_b, &target_key, "223e4567-e89b-42d3-a456-426614174000");
    let peer_b = request_b.record.request.requester.clone();
    let grant = DirectGrant {
        device_id: peer_b.device_id.clone(),
        device_name: peer_b.device_name.clone(),
        public_key: peer_b.public_key.clone(),
        fingerprint: peer_b.fingerprint.clone(),
        node_id: peer_b.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 200,
        exec: Default::default(),
    };
    let selector_a = request_a.record.request.request_id.to_string();
    let selector_b = request_b.record.request.request_id.to_string();
    let mut profiles = ShareProfiles {
        direct_requests: vec![request_a, request_b],
        ..ShareProfiles::default()
    };
    profiles.direct_grants.push(grant.clone());

    let related = related_requests(&profiles, &grant).collect::<Vec<_>>();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].record.request.request_id.to_string(), selector_b);
    assert_eq!(
        latest_accepted_request(&profiles, &grant)
            .unwrap()
            .record
            .request
            .requester
            .public_key,
        peer_b.public_key
    );
    assert!(!grant_selector_matches(&profiles, &grant, &selector_a));
    assert!(grant_selector_matches(&profiles, &grant, &selector_b));
}

fn entry(
    requester_key: &iroh::SecretKey,
    target_key: &iroh::SecretKey,
    request_id: &str,
) -> DirectRequestEntry {
    let target = DirectPeerIdentity::from_secret("target", "Target", target_key);
    let request = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(request_id).unwrap(),
        "lookup-target",
        DirectPeerIdentity::from_secret("shared-device", "Requester", requester_key),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        100,
        1_000,
        format!("nonce-{request_id}"),
        None,
        &SECRET,
        requester_key,
    )
    .unwrap();
    let mut record = DirectRequestRecord::new(request);
    record.decision.state = DirectDecisionState::Accepted;
    record.decision.revision = 1;
    record.decision.changed_at = 120;
    DirectRequestEntry {
        direction: DirectRequestDirection::Incoming,
        contact_id: None,
        local_lookup_id: Some("lookup-target".into()),
        record,
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: DirectRequestRetries::default(),
    }
}
