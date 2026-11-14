//! Revocation by a caller without key and node id, and presences without key
//! or node id (r-zugang U2).

use super::super::direct_protocol::DirectPeerIdentity;
use super::super::legacy_direct_request::{
    LegacyDirectDecisionState, LegacyDirectPresenceEvidence, LegacyDirectRequestEntry,
};
use super::super::profiles::ShareProfiles;
use super::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, PeerPresence, PresenceApply,
    ShareStatus,
};

fn grant(device_id: &str, public_key: &str, node_id: &str) -> DirectGrant {
    DirectGrant {
        device_id: device_id.into(),
        device_name: String::new(),
        public_key: public_key.into(),
        fingerprint: String::new(),
        node_id: node_id.into(),
        state: DirectGrantState::Accepted,
        updated_at: 0,
        exec: Default::default(),
        write: false,
    }
}

fn caller(device_id: &str, public_key: &str, node_id: &str) -> DirectPeerIdentity {
    DirectPeerIdentity {
        device_id: device_id.into(),
        device_name: String::new(),
        node_id: node_id.into(),
        public_key: public_key.into(),
        fingerprint: String::new(),
    }
}

fn states(profiles: &ShareProfiles) -> Vec<DirectGrantState> {
    profiles
        .direct_grants
        .iter()
        .map(|entry| entry.state.clone())
        .collect()
}

#[test]
fn unbelegt_task_entfernen_ohne_kennung_widerruft_nur_kennungslose_eintraege() {
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants = vec![
        grant("dev-a", "", ""),
        grant("dev-a", "key-b", ""),
        grant("dev-a", "", "node-c"),
        grant("dev-x", "", ""),
    ];
    // A caller without key and node id reaches only the keyless entry of its device id.
    profiles.withdraw_direct_key(&caller("dev-a", "", ""), 1_000);
    assert_eq!(
        states(&profiles),
        vec![
            DirectGrantState::Ignored,
            DirectGrantState::Accepted,
            DirectGrantState::Accepted,
            DirectGrantState::Accepted,
        ]
    );
    // A caller with a key still reaches the entry that carries that key.
    profiles.withdraw_direct_key(&caller("dev-a", "key-b", ""), 2_000);
    assert_eq!(
        states(&profiles),
        vec![
            DirectGrantState::Ignored,
            DirectGrantState::Ignored,
            DirectGrantState::Accepted,
            DirectGrantState::Accepted,
        ]
    );
}

fn legacy(device_id: &str, public_key: &str, node_id: &str) -> LegacyDirectRequestEntry {
    LegacyDirectRequestEntry {
        selector: format!("legacy-{device_id}-{public_key}-{node_id}"),
        lookup_id: "lookup".into(),
        peer: caller(device_id, public_key, node_id),
        evidence: LegacyDirectPresenceEvidence {
            event_id: "event".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: 30,
            nonce: "nonce".into(),
            proof: "proof".into(),
        },
        first_received_at: 1,
        last_received_at: 10,
        decision: LegacyDirectDecisionState::Accepted,
        decision_source: None,
        decision_changed_at: 5,
        decision_revision: 2,
        decision_delivery: Default::default(),
        identity_conflict: false,
    }
}

#[test]
fn unbelegt_task_legacy_widerruf_ohne_kennung_trifft_nur_kennungslose_eintraege() {
    let mut profiles = ShareProfiles::default();
    profiles.legacy_direct_requests = vec![
        legacy("dev-a", "", ""),
        legacy("dev-a", "key-b", ""),
        legacy("dev-a", "", "node-c"),
        legacy("dev-x", "", ""),
    ];
    // The legacy path follows the same rule as the grants above.
    let changed = profiles.mark_legacy_revoked_for_peer(&caller("dev-a", "", ""), 1_000);
    assert_eq!(changed, 1);
    let decisions: Vec<LegacyDirectDecisionState> = profiles
        .legacy_direct_requests
        .iter()
        .map(|entry| entry.decision)
        .collect();
    assert_eq!(
        decisions,
        vec![
            LegacyDirectDecisionState::Revoked,
            LegacyDirectDecisionState::Accepted,
            LegacyDirectDecisionState::Accepted,
            LegacyDirectDecisionState::Accepted,
        ]
    );
    // A caller with a node id still reaches the entry that carries it.
    let changed = profiles.mark_legacy_revoked_for_peer(&caller("dev-a", "", "node-c"), 2_000);
    assert_eq!(changed, 1);
    assert_eq!(
        profiles.legacy_direct_requests[2].decision,
        LegacyDirectDecisionState::Revoked
    );
}

fn unpinned_contact() -> DirectContact {
    DirectContact {
        id: "contact-a".into(),
        display_name: "Device a".into(),
        lookup_id: "lookup-a".into(),
        expected_fingerprint: "fp".into(),
        expected_node_id: String::new(),
        remote_device_id: None,
        remote_public_key: None,
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Offline,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn presence(public_key: &str, node_id: &str) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: "lookup-a".into(),
        device_id: "dev-a".into(),
        device_name: "Device a".into(),
        public_key: public_key.into(),
        fingerprint: "fp".into(),
        node_id: node_id.into(),
        relay_url: String::new(),
        candidates: Vec::new(),
        expires_at: i64::MAX,
        nonce: "nonce".into(),
        proof: String::new(),
    }
}

#[test]
fn unbelegt_task_leere_praesenz_setzt_keinen_pin() {
    let mut contact = unpinned_contact();
    // A presence without key and node id pins neither of them; an empty and a
    // blank node id both count as absent.
    contact.apply_verified_presence(presence("", ""), 5);
    contact.apply_verified_presence(presence("", " "), 10);
    assert_eq!(contact.expected_node_id, "");
    assert_eq!(contact.remote_public_key, None);
    // The identity that follows is pinned, not refused as a conflict.
    assert_eq!(
        contact.apply_verified_presence(presence("key-a", "node-a"), 20),
        PresenceApply::Pinned
    );
    assert_eq!(contact.expected_node_id, "node-a");
    assert_eq!(contact.remote_public_key.as_deref(), Some("key-a"));
}

#[test]
fn unbelegt_task_wieder_erlauben_erreicht_was_der_widerruf_erreicht() {
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants = vec![grant("dev-a", "", ""), grant("dev-a", "key-b", "")];
    profiles.withdraw_direct_key(&caller("dev-a", "", ""), 1_000);
    profiles.withdraw_direct_key(&caller("dev-a", "key-b", ""), 1_000);
    assert_eq!(
        states(&profiles),
        vec![DirectGrantState::Ignored, DirectGrantState::Ignored]
    );
    // „Wieder erlauben“ of the grant without identity reaches exactly the
    // entries its revocation reached; the keyed grant stays blocked.
    assert_eq!(profiles.allow_direct_grant_again("dev-a", 2_000), Ok(true));
    assert_eq!(
        states(&profiles),
        vec![DirectGrantState::Accepted, DirectGrantState::Ignored]
    );
}
