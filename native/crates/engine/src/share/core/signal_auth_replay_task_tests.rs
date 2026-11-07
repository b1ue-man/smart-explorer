//! r-entdeckung-1 (suite-2-entdeckung): signature markers have their own
//! bound, never count against the replay keys and keep the downgrade
//! protection of pinned peers.
use super::*;

use crate::share::power::test_support::{auth_state, identity};
use crate::share::types::{DirectAccessState, ShareStatus};

fn signed_presence(index: usize) -> PeerPresence {
    let key = format!("entd-key-{index}");
    PeerPresence {
        kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: format!("entd-device-{index}"),
        device_name: "Peer".into(),
        public_key: key.clone(),
        fingerprint: "fp".into(),
        node_id: key,
        relay_url: String::new(),
        candidates: Vec::new(),
        expires_at: 2_000,
        nonce: format!("random-{index}.ps1.signature"),
        proof: String::new(),
    }
}

fn marker_count(seen: &HashSet<String>) -> usize {
    seen.iter().filter(|entry| is_marker(entry)).count()
}

fn contact_pinning(key: &str) -> DirectContact {
    DirectContact {
        id: "contact-pinned".into(),
        display_name: "Pinned".into(),
        lookup_id: "lookup-pinned".into(),
        expected_fingerprint: "fp".into(),
        expected_node_id: key.into(),
        remote_device_id: Some("device-pinned".into()),
        remote_public_key: Some(key.into()),
        auto_connect: false,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Waiting,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(key.into()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

#[test]
fn entdeckung_task_signature_markers_are_bounded_and_the_least_recent_unpinned_gives_way() {
    let mut seen = HashSet::new();
    for index in 0..=MAX_SIGNATURE_MARKERS {
        let accepted = remember_presence_in(
            &mut seen,
            replay_key(2_000, &format!("message-{index}")),
            100 + index as i64,
            &signed_presence(index),
            HashSet::new,
        );
        assert!(accepted, "signed presence {index} is never refused");
        assert!(marker_count(&seen) <= MAX_SIGNATURE_MARKERS);
    }
    assert_eq!(marker_count(&seen), MAX_SIGNATURE_MARKERS);
    assert!(
        !signature_seen(&seen, &signed_presence(0)),
        "the marker verified longest ago gave way"
    );
    assert!(signature_seen(&seen, &signed_presence(1)));
    assert!(signature_seen(
        &seen,
        &signed_presence(MAX_SIGNATURE_MARKERS)
    ));
}

#[test]
fn entdeckung_task_replay_keys_keep_their_own_bound_next_to_the_markers() {
    let mut seen = HashSet::new();
    let signed = MAX_SIGNATURE_MARKERS + 6;
    for index in 0..signed {
        assert!(remember_presence_in(
            &mut seen,
            replay_key(2_000, &format!("message-{index}")),
            100,
            &signed_presence(index),
            HashSet::new,
        ));
    }
    // The markers fill their own bound; the replay keys still reach 4096.
    for index in signed..MAX_REPLAY_KEYS {
        assert!(
            remember_replay(&mut seen, replay_key(2_000, &format!("fresh-{index}")), 100),
            "replay key {index} is accepted although {MAX_SIGNATURE_MARKERS} markers exist"
        );
    }
    assert!(!remember_replay(
        &mut seen,
        replay_key(2_000, "one-too-many"),
        100
    ));
    assert_eq!(replay_count(&seen), MAX_REPLAY_KEYS);
    assert_eq!(marker_count(&seen), MAX_SIGNATURE_MARKERS);
    assert_eq!(seen.len(), MAX_REPLAY_KEYS + MAX_SIGNATURE_MARKERS);

    // Expiry prunes replay keys, never the markers.
    assert!(remember_replay(
        &mut seen,
        replay_key(9_000, "after-expiry"),
        2_001
    ));
    assert_eq!(replay_count(&seen), 1);
    assert_eq!(marker_count(&seen), MAX_SIGNATURE_MARKERS);
}

#[test]
fn entdeckung_task_a_pinned_contacts_marker_survives_a_flood_of_unpinned_markers() {
    let local = identity("replay local").expect("identity");
    let mut state = auth_state(&local);
    let pinned = signed_presence(5_000);
    state
        .direct_contacts
        .push(contact_pinning(&pinned.public_key));
    // The pinned peer's marker is the oldest of all.
    assert!(remember_presence(
        &mut state,
        replay_key(2_000, "pinned"),
        1,
        &pinned
    ));
    for index in 0..MAX_SIGNATURE_MARKERS + 10 {
        assert!(remember_presence(
            &mut state,
            replay_key(2_000, &format!("flood-{index}")),
            10 + index as i64,
            &signed_presence(index),
        ));
    }
    assert_eq!(marker_count(&state.seen_nonces), MAX_SIGNATURE_MARKERS);
    assert!(
        signature_seen(&state.seen_nonces, &pinned),
        "the pinned peer keeps its downgrade protection"
    );
    assert!(
        !signature_seen(&state.seen_nonces, &signed_presence(0)),
        "unpinned markers gave way first"
    );
}
