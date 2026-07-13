use super::core::{hmac_proof, presence_payload, public_fingerprint};
use super::identity::ShareIdentity;
use super::{
    DirectDecisionKind, DirectGrantState, DirectLedgerError, DirectPeerIdentity, DirectRequestId,
    ExecGrant, PeerPresence, ShareProfiles, SignedDirectDecision, SignedDirectRequest,
};

const RELATION_SECRET: [u8; 32] = [0x66; 32];
const REQUEST_A: &str = "123e4567-e89b-42d3-a456-426614174000";
const REQUEST_B: &str = "223e4567-e89b-42d3-a456-426614174000";

#[test]
fn tracked_conflicts_are_symmetric_in_both_arrival_orders() {
    for (first, second) in [(1, 2), (2, 1)] {
        let request_a = request(REQUEST_A, first);
        let request_b = request(REQUEST_B, second);
        let mut profiles = ShareProfiles::default();
        profiles
            .record_incoming_direct_request("local-lookup", request_a.clone(), 110)
            .unwrap();
        profiles
            .record_incoming_direct_request("local-lookup", request_b.clone(), 111)
            .unwrap();

        assert!(profiles.tracked_identity_conflict(&request_a.request_id));
        assert!(profiles.tracked_identity_conflict(&request_b.request_id));
        assert_eq!(
            profiles.record_direct_decision(
                decision(&request_a, DirectDecisionKind::Accepted, 130),
                130,
            ),
            Err(DirectLedgerError::IdentityConflict)
        );
        assert!(profiles.direct_grants.is_empty());
    }
}

#[test]
fn rejecting_a_spoof_never_changes_the_legitimate_grant_or_exec_policy() {
    let legitimate = request(REQUEST_A, 1);
    let spoof = request(REQUEST_B, 2);
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("local-lookup", legitimate.clone(), 110)
        .unwrap();
    profiles
        .record_direct_decision(
            decision(&legitimate, DirectDecisionKind::Accepted, 120),
            120,
        )
        .unwrap();
    profiles.direct_grants[0].exec = ExecGrant {
        enabled: true,
        policy_revision: 7,
        ..ExecGrant::default()
    };
    let expected = profiles.direct_grants[0].clone();
    profiles
        .record_incoming_direct_request("local-lookup", spoof.clone(), 121)
        .unwrap();

    profiles
        .record_direct_decision(decision(&spoof, DirectDecisionKind::Rejected, 130), 130)
        .unwrap();

    assert_eq!(profiles.direct_grants[0], expected);
    assert!(!profiles.tracked_identity_conflict(&spoof.request_id));
    assert!(!profiles.tracked_identity_conflict(&legitimate.request_id));
}

#[test]
fn explicit_accept_replaces_only_an_inactive_different_key_pin() {
    let rejected = request(REQUEST_A, 1);
    let accepted = request(REQUEST_B, 2);
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("local-lookup", rejected.clone(), 110)
        .unwrap();
    profiles
        .record_direct_decision(decision(&rejected, DirectDecisionKind::Rejected, 120), 120)
        .unwrap();
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Ignored);
    profiles.direct_grants[0].exec.enabled = true;
    let old_revision = profiles.direct_grants[0].exec.policy_revision;
    profiles
        .record_incoming_direct_request("local-lookup", accepted.clone(), 121)
        .unwrap();
    assert!(!profiles.tracked_identity_conflict(&accepted.request_id));

    profiles
        .record_direct_decision(decision(&accepted, DirectDecisionKind::Accepted, 130), 130)
        .unwrap();

    let grant = &profiles.direct_grants[0];
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert_eq!(grant.public_key, accepted.requester.public_key);
    assert_eq!(grant.fingerprint, accepted.requester.fingerprint);
    assert!(!grant.exec.enabled);
    assert_eq!(grant.exec.policy_revision, old_revision.saturating_add(1));
}

#[test]
fn tracked_and_legacy_claims_conflict_then_resolve_through_reject() {
    let identity = local_identity();
    let tracked = request(REQUEST_A, 1);
    let legacy = legacy_presence(&identity, 2, "legacy-b");
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("local-lookup", tracked.clone(), 110)
        .unwrap();
    profiles
        .record_verified_legacy_direct_request("local-lookup", &legacy, 111)
        .unwrap();
    let selector = profiles.legacy_direct_requests[0].selector.clone();

    assert!(profiles.tracked_identity_conflict(&tracked.request_id));
    assert!(profiles.legacy_direct_requests[0].identity_conflict);
    profiles
        .record_direct_decision(decision(&tracked, DirectDecisionKind::Rejected, 120), 120)
        .unwrap();
    assert!(!profiles.legacy_direct_requests[0].identity_conflict);
    profiles
        .decide_legacy_direct_request(&selector, true, 130)
        .unwrap();
    let grant = &profiles.direct_grants[0];
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert_eq!(grant.public_key, legacy.public_key);
}

#[test]
fn legacy_reject_resolves_conflict_before_tracked_accept() {
    let identity = local_identity();
    let legacy = legacy_presence(&identity, 1, "legacy-a");
    let tracked = request(REQUEST_A, 2);
    let mut profiles = ShareProfiles::default();
    profiles
        .record_verified_legacy_direct_request("local-lookup", &legacy, 110)
        .unwrap();
    let selector = profiles.legacy_direct_requests[0].selector.clone();
    profiles
        .record_incoming_direct_request("local-lookup", tracked.clone(), 111)
        .unwrap();
    assert!(profiles.tracked_identity_conflict(&tracked.request_id));
    assert!(profiles.legacy_direct_requests[0].identity_conflict);

    profiles
        .decide_legacy_direct_request(&selector, false, 120)
        .unwrap();
    assert!(!profiles.tracked_identity_conflict(&tracked.request_id));
    profiles
        .record_direct_decision(decision(&tracked, DirectDecisionKind::Accepted, 130), 130)
        .unwrap();
    let grant = &profiles.direct_grants[0];
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert_eq!(grant.public_key, tracked.requester.public_key);
}

fn key(seed: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[seed; 32])
}

fn local_identity() -> ShareIdentity {
    let secret = key(9);
    let public = secret.public().to_string();
    ShareIdentity {
        device_id: "local-device".into(),
        device_name: "Local".into(),
        direct_lookup_id: "local-lookup".into(),
        public_key: public.clone(),
        fingerprint: public_fingerprint(public.as_bytes()),
        node_id: public,
        iroh_secret: secret,
        direct_secret: [7; 32],
    }
}

fn request(request_id: &str, requester_seed: u8) -> SignedDirectRequest {
    let target_public = key(9).public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(request_id).unwrap(),
        "local-lookup",
        DirectPeerIdentity::from_secret("shared-device", "Peer", &key(requester_seed)),
        DirectPeerIdentity::pinned_target(
            target_public.clone(),
            public_fingerprint(target_public.as_bytes()),
        ),
        100,
        200,
        format!("request-{requester_seed}"),
        None,
        &RELATION_SECRET,
        &key(requester_seed),
    )
    .unwrap()
}

fn decision(
    request: &SignedDirectRequest,
    kind: DirectDecisionKind,
    at: i64,
) -> SignedDirectDecision {
    SignedDirectDecision::sign_with_nonce(
        request,
        DirectPeerIdentity::from_secret("local-device", "Local", &key(9)),
        kind,
        1,
        at,
        at + 60,
        format!("decision-{at}"),
        None,
        &RELATION_SECRET,
        &key(9),
    )
    .unwrap()
}

fn legacy_presence(identity: &ShareIdentity, seed: u8, nonce: &str) -> PeerPresence {
    let public = key(seed).public().to_string();
    let mut presence = PeerPresence {
        kind: "direct".into(),
        relation_id: identity.direct_lookup_id.clone(),
        device_id: "shared-device".into(),
        device_name: "Peer".into(),
        public_key: public.clone(),
        fingerprint: public_fingerprint(public.as_bytes()),
        node_id: public,
        relay_url: "https://relay.invalid".into(),
        candidates: vec!["127.0.0.1:9000".into()],
        expires_at: 200,
        nonce: nonce.into(),
        proof: String::new(),
    };
    let payload = presence_payload(
        "direct",
        &presence.relation_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    presence.proof = hmac_proof(&identity.direct_secret(), &payload);
    presence
}
