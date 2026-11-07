use super::*;
use crate::share::{DirectContact, DirectGrant, ShareStatus};

fn contact(device: &str) -> DirectContact {
    DirectContact {
        id: format!("contact-{device}"),
        display_name: format!("Peer {device}"),
        lookup_id: format!("lookup-{device}"),
        expected_fingerprint: format!("fp-{device}"),
        expected_node_id: format!("node-{device}"),
        remote_device_id: Some(device.into()),
        remote_public_key: Some(format!("key-{device}")),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Available,
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

fn grant(device: &str, state: DirectGrantState) -> DirectGrant {
    DirectGrant {
        device_id: device.into(),
        device_name: format!("Peer {device}"),
        public_key: format!("key-{device}"),
        fingerprint: format!("fp-{device}"),
        node_id: format!("node-{device}"),
        state,
        updated_at: 1,
        exec: Default::default(),
        write: false,
    }
}

#[test]
fn rechte_task_one_way_contact_and_suspended_grant_are_gaps() {
    let cases: [(&str, fn()); 2] = [
        (
            "rechte_task_one_way_contact_and_suspended_grant_are_gaps",
            rechte_task_one_way_contact_and_suspended_grant_are_gaps_case,
        ),
        (
            "rechte_task_pending_contact_is_no_gap",
            rechte_task_pending_contact_is_no_gap,
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

fn rechte_task_one_way_contact_and_suspended_grant_are_gaps_case() {
    let mut profiles = ShareProfiles::default();
    // a: reached by this device, never shared back (one-sided pairing, U1).
    profiles.direct_contacts.push(contact("a"));
    // b: suspended after a code rotation (W4/W5).
    profiles.direct_contacts.push(contact("b"));
    profiles
        .direct_grants
        .push(grant("b", DirectGrantState::Reconfirm));
    // c: accepted and complete, no gap; d: blocked by the user, no gap.
    profiles.direct_contacts.push(contact("c"));
    profiles
        .direct_grants
        .push(grant("c", DirectGrantState::Accepted));
    profiles
        .direct_grants
        .push(grant("d", DirectGrantState::Ignored));

    let gaps = access_gaps(&profiles);
    let kinds: Vec<_> = gaps
        .iter()
        .map(|gap| (gap.device_id.as_str(), gap.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("b", AccessGapKind::Reconfirm),
            ("a", AccessGapKind::NotShared)
        ]
    );
    assert_eq!(gaps[1].contact_id.as_deref(), Some("contact-a"));
    assert!(gaps[1].message.contains("Peer a"), "{}", gaps[1].message);
    assert_eq!(AccessGapKind::RightsPending.code(), "rights_pending");
}

fn rechte_task_pending_contact_is_no_gap() {
    let mut profiles = ShareProfiles::default();
    let mut waiting = contact("w");
    waiting.access_state = DirectAccessState::Pending;
    profiles.direct_contacts.push(waiting);
    assert!(access_gaps(&profiles).is_empty());
}
