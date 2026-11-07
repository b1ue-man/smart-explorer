//! E16 / M20 (suite-2-entdeckung): a full legacy request inbox makes room
//! again by displacing the oldest refused entry whose refusal survives the
//! entry, and never displaces an authorized device or a user refusal that
//! only the entry holds.
use super::*;
use crate::share::legacy_direct_request::{
    peer_from_presence, LegacyDirectDecisionSource, MAX_LEGACY_DIRECT_REQUESTS,
};

/// A request of a distinct device (its own device id and key).
fn presence_of(identity: &ShareIdentity, seed: u8, expires_at: i64, nonce: &str) -> PeerPresence {
    let mut presence = presence(identity, seed, nonce, expires_at);
    presence.device_id = format!("peer-device-{seed}");
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

/// Fills the inbox with `MAX_LEGACY_DIRECT_REQUESTS` pending requests of
/// distinct devices, the n-th received at `100 + n`.
fn full_inbox(identity: &ShareIdentity) -> ShareProfiles {
    let policy = crate::share::DirectRequestPolicy::Ask;
    let mut profiles = ShareProfiles::default();
    for index in 0..MAX_LEGACY_DIRECT_REQUESTS {
        let now = 100 + index as i64;
        let request = presence_of(
            identity,
            10 + index as u8,
            now + 300,
            &format!("nonce-{index}"),
        );
        assert!(profiles
            .record_verified_legacy_direct_request(
                &identity.direct_lookup_id,
                &request,
                now,
                policy
            )
            .unwrap());
    }
    assert_eq!(
        profiles.legacy_direct_requests.len(),
        MAX_LEGACY_DIRECT_REQUESTS
    );
    profiles
}

#[test]
fn entdeckung_task_a_full_inbox_of_refused_requests_makes_room_and_keeps_the_denial() {
    let identity = identity();
    let policy = crate::share::DirectRequestPolicy::Ask;
    let mut profiles = full_inbox(&identity);
    let mut peers = Vec::new();
    for index in 0..MAX_LEGACY_DIRECT_REQUESTS {
        let entry = profiles.legacy_direct_requests[index].clone();
        // The user refuses each one: an `Ignored` grant keeps the key denied.
        profiles
            .decide_legacy_direct_request(&entry.selector, false, 200)
            .unwrap();
        peers.push(entry);
    }
    let oldest = peers[0].clone();
    assert!(profiles.direct_key_denied(&oldest.peer));

    // Before E16 this request failed with "inbox is full".
    let newcomer = presence_of(&identity, 100, 700, "nonce-newcomer");
    assert!(profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &newcomer, 300, policy)
        .unwrap());
    assert_eq!(
        profiles.legacy_direct_requests.len(),
        MAX_LEGACY_DIRECT_REQUESTS
    );
    assert!(
        profiles.legacy_direct_request(&oldest.selector).is_none(),
        "the oldest refused entry gave way"
    );
    assert!(
        profiles.legacy_direct_request(&peers[1].selector).is_some(),
        "only one entry gave way"
    );
    assert!(
        profiles.direct_key_denied(&oldest.peer),
        "the refusal outlives its entry"
    );

    // The displaced device asks again and is refused without the user.
    let again = presence_of(&identity, 10, 800, "nonce-again");
    assert_eq!(
        peer_from_presence(&again).public_key,
        oldest.peer.public_key
    );
    assert!(profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &again, 400, policy)
        .unwrap());
    let returned = profiles
        .legacy_direct_requests
        .iter()
        .find(|entry| entry.peer.public_key == oldest.peer.public_key)
        .expect("the displaced device is recorded again");
    assert_eq!(returned.decision, LegacyDirectDecisionState::Rejected);
    assert_ne!(
        returned.decision_source,
        Some(LegacyDirectDecisionSource::User)
    );
    assert!(!returned.authorization_active(&profiles));
}

#[test]
fn entdeckung_task_user_refusals_without_a_grant_and_authorized_devices_are_never_displaced() {
    let identity = identity();
    let policy = crate::share::DirectRequestPolicy::Ask;
    let mut profiles = full_inbox(&identity);

    // The oldest request is accepted and holds an active grant.
    let accepted = profiles.legacy_direct_requests[0].selector.clone();
    profiles
        .decide_legacy_direct_request(&accepted, true, 200)
        .unwrap();
    assert!(profiles.legacy_direct_requests[0].authorization_active(&profiles));
    // The others are user refusals held only by their entries (no
    // `Ignored` grant, no removed-peer record), as after an identity conflict.
    for entry in profiles.legacy_direct_requests.iter_mut().skip(1) {
        entry.decision = LegacyDirectDecisionState::Rejected;
        entry.decision_source = Some(LegacyDirectDecisionSource::User);
    }
    let before: Vec<_> = profiles
        .legacy_direct_requests
        .iter()
        .map(|entry| entry.selector.clone())
        .collect();

    let newcomer = presence_of(&identity, 100, 700, "nonce-newcomer");
    let error = profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &newcomer, 300, policy)
        .unwrap_err();
    assert!(
        error.contains("legacy request inbox is full (maximum 24)"),
        "{error}"
    );
    let after: Vec<_> = profiles
        .legacy_direct_requests
        .iter()
        .map(|entry| entry.selector.clone())
        .collect();
    assert_eq!(after, before, "no entry was displaced");
    assert_eq!(
        profiles.legacy_direct_requests[0].decision,
        LegacyDirectDecisionState::Accepted
    );
}

#[test]
fn entdeckung_task_unanswered_requests_still_give_way_before_refused_ones() {
    let identity = identity();
    let policy = crate::share::DirectRequestPolicy::Ask;
    let mut profiles = full_inbox(&identity);
    // Entry 5 is refused (old rule keeps it), the others stay unanswered.
    let refused = profiles.legacy_direct_requests[5].selector.clone();
    profiles
        .decide_legacy_direct_request(&refused, false, 200)
        .unwrap();
    let oldest_unanswered = profiles.legacy_direct_requests[0].selector.clone();
    let newcomer = presence_of(&identity, 100, 700, "nonce-newcomer");
    assert!(profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &newcomer, 300, policy)
        .unwrap());
    assert!(profiles.legacy_direct_request(&oldest_unanswered).is_none());
    assert!(profiles.legacy_direct_request(&refused).is_some());
}
