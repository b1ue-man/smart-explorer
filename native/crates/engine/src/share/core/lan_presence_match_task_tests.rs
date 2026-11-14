//! r-entdeckung-4 (suite-2-entdeckung): a contact that delivered a signed
//! presence is no longer matched by an unsigned `v=1` record, whose hashed id
//! anybody who knows the node id can compute.
use super::*;
use crate::share::types::ShareStatus;

fn contact(node: &str, signed_presence: bool) -> DirectContact {
    let mut contact = DirectContact {
        id: format!("contact-{node}"),
        display_name: format!("Device {node}"),
        lookup_id: format!("lookup-{node}"),
        expected_fingerprint: "fp".into(),
        expected_node_id: node.into(),
        remote_device_id: Some(format!("device-{node}")),
        remote_public_key: Some(node.into()),
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
    };
    contact.relation.signed_presence = signed_presence;
    contact
}

fn v1_sighting(node: &str) -> LanSighting {
    LanSighting {
        id: hashed_lan_id(node),
        addrs: vec!["192.168.1.9".parse().unwrap()],
        p4: 4000,
        p6: 0,
        uplink: false,
        seen_at: 100,
    }
}

#[test]
fn entdeckung_task_a_signing_contact_gets_no_v1_id_and_no_v1_match() {
    let signing = contact("node-signing", true);
    assert_eq!(contact_lan_id(&signing), None);
    let sighting = v1_sighting("node-signing");
    assert!(match_sighting(&[signing], &sighting, &[]).is_none());
}

#[test]
fn entdeckung_task_a_contact_that_never_signed_keeps_its_v1_match() {
    let older = contact("node-older", false);
    assert_eq!(contact_lan_id(&older), Some(hashed_lan_id("node-older")));
    let (contact_id, candidates) =
        match_sighting(&[older], &v1_sighting("node-older"), &[]).expect("matches");
    assert_eq!(contact_id, "contact-node-older");
    assert_eq!(candidates, vec!["192.168.1.9:4000".to_string()]);
}

#[test]
fn unbelegt_task_candidates_exclude_loopback_multicast_and_mapped() {
    let addresses = [
        "127.0.0.1",
        "0.0.0.0",
        "224.0.0.251",
        "::1",
        "::",
        "ff02::fb",
        "::ffff:127.0.0.1",
        "::ffff:224.0.0.251",
        "2001:db8::5",
        "192.168.1.9",
    ];
    let seen = LanSighting {
        id: "x".into(),
        addrs: addresses
            .iter()
            .map(|ip| ip.parse().expect("address"))
            .collect(),
        p4: 4000,
        p6: 6000,
        uplink: false,
        seen_at: 100,
    };
    assert_eq!(
        candidates_for(&seen, &[]),
        ["192.168.1.9:4000", "[2001:db8::5]:6000"]
    );
    // A matched announcement stores exactly these candidates as LAN evidence.
    let mut announcement = seen.clone();
    announcement.id = hashed_lan_id("node-filtered");
    let (_, stored) =
        match_sighting(&[contact("node-filtered", false)], &announcement, &[]).expect("matches");
    assert_eq!(stored, ["192.168.1.9:4000", "[2001:db8::5]:6000"]);
}
