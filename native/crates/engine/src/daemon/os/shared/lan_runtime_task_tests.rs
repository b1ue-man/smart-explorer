//! r-entdeckung-4 and its runtime half G2 (suite-2-entdeckung): a contact
//! whose signed `v=2`/`v=3` record was verified in this runtime, or that
//! delivered a signed presence, is never matched by an unsigned `v=1`
//! record; a contact that never signed keeps the `v=1` path.
use super::*;
use crate::share::lan_presence_auth::task_tests::{
    announcement_of, authenticator_for, KnownContact,
};
use crate::share::power::test_support::identity;
use crate::share::{DirectAccessState, ShareStatus};

const NOW: i64 = 1_800;
const LATER: i64 = NOW + 1_000;

fn contact_of(known: &KnownContact, signed_presence: bool) -> DirectContact {
    let mut contact = DirectContact {
        id: known.id.clone(),
        display_name: known.id.clone(),
        lookup_id: format!("lookup-{}", known.id),
        expected_fingerprint: "fp".into(),
        expected_node_id: known.node.clone(),
        remote_device_id: Some(format!("device-{}", known.id)),
        remote_public_key: Some(known.node.clone()),
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

/// The unsigned `v=1` record anybody can forge for a known node id.
fn insert_v1(runtime: &mut LanRuntime, node: &str, address: &str, now: i64) {
    let sighting = LanSighting {
        id: lan_presence_match::hashed_lan_id(node),
        addrs: vec![address.parse().expect("address")],
        p4: 51820,
        p6: 0,
        uplink: false,
        seen_at: now,
    };
    runtime.sightings.insert(sighting.id.clone(), sighting);
}

/// A signed record of the contact's device that verifies.
fn insert_verified(
    runtime: &mut LanRuntime,
    known: &KnownContact,
    key: &iroh::SecretKey,
    address: &str,
    now: i64,
    sealed: bool,
) {
    let (sighting, proof) = announcement_of(
        known,
        key,
        &[address.parse().expect("address")],
        now,
        sealed,
    );
    runtime.proofs.insert(sighting.id.clone(), proof);
    runtime.sightings.insert(sighting.id.clone(), sighting);
}

fn seen(events: &[ShareEvent]) -> Vec<(String, Vec<String>)> {
    events
        .iter()
        .filter_map(|event| match event {
            ShareEvent::LanPeerSeen {
                contact_id,
                candidates,
                ..
            } => Some((contact_id.clone(), candidates.clone())),
            _ => None,
        })
        .collect()
}

fn lost(events: &[ShareEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            ShareEvent::LanPeerLost { contact_id } => Some(contact_id.clone()),
            _ => None,
        })
        .collect()
}

fn runtime_knowing(known: &KnownContact) -> LanRuntime {
    let own = identity("lan runtime own").expect("identity");
    let mut runtime = LanRuntime::new();
    runtime.authenticator = authenticator_for(&own, std::slice::from_ref(known), NOW);
    runtime
}

#[test]
fn entdeckung_task_after_a_verified_signed_record_an_expired_one_is_not_replaced_by_v1() {
    for sealed in [false, true] {
        let (known, key) = KnownContact::with_key("contact-g2", 41);
        let contact = contact_of(&known, false);
        let mut runtime = runtime_knowing(&known);

        insert_verified(&mut runtime, &known, &key, "192.168.1.5", NOW, sealed);
        let events = runtime.reconcile(std::slice::from_ref(&contact), NOW);
        assert_eq!(
            seen(&events),
            vec![(
                "contact-g2".to_string(),
                vec!["192.168.1.5:51820".to_string()]
            )],
            "sealed={sealed}"
        );
        assert_eq!(runtime.authenticated_contacts, ["contact-g2"]);

        // The verified record ran out; a forged `v=1` record takes its place.
        runtime.expire_sightings(LATER);
        assert!(runtime.sightings.is_empty() && runtime.proofs.is_empty());
        insert_v1(&mut runtime, &known.node, "203.0.113.9", LATER);
        let events = runtime.reconcile(std::slice::from_ref(&contact), LATER);
        assert!(seen(&events).is_empty(), "{events:?}");
        assert_eq!(lost(&events), ["contact-g2"]);
        assert_eq!(runtime.unknown_devices, 1);
        assert_eq!(runtime.authenticated_contacts, ["contact-g2"]);
    }
}

#[test]
fn entdeckung_task_in_one_comparison_the_candidates_come_from_the_verified_record_only() {
    let (known, key) = KnownContact::with_key("contact-g2", 42);
    let contact = contact_of(&known, false);
    let mut runtime = runtime_knowing(&known);
    insert_verified(&mut runtime, &known, &key, "192.168.1.5", NOW, true);
    insert_v1(&mut runtime, &known.node, "203.0.113.9", NOW);
    assert_eq!(runtime.sightings.len(), 2);

    let events = runtime.reconcile(std::slice::from_ref(&contact), NOW);
    assert_eq!(
        seen(&events),
        vec![(
            "contact-g2".to_string(),
            vec!["192.168.1.5:51820".to_string()]
        )]
    );
    assert_eq!(runtime.unknown_devices, 1, "the forged v=1 record");
}

#[test]
fn entdeckung_task_a_contact_with_a_signed_presence_gets_no_v1_match_in_the_runtime() {
    let (known, _) = KnownContact::with_key("contact-signed", 43);
    let mut runtime = LanRuntime::new();
    insert_v1(&mut runtime, &known.node, "203.0.113.9", NOW);
    let events = runtime.reconcile(&[contact_of(&known, true)], NOW);
    assert!(seen(&events).is_empty(), "{events:?}");
    assert_eq!(runtime.unknown_devices, 1);
}

#[test]
fn entdeckung_task_a_contact_that_never_signed_keeps_the_v1_path() {
    let (known, _) = KnownContact::with_key("contact-older", 44);
    let contact = contact_of(&known, false);
    let mut runtime = LanRuntime::new();
    insert_v1(&mut runtime, &known.node, "192.168.1.7", NOW);
    let events = runtime.reconcile(std::slice::from_ref(&contact), NOW);
    assert_eq!(
        seen(&events),
        vec![(
            "contact-older".to_string(),
            vec!["192.168.1.7:51820".to_string()]
        )]
    );
    assert!(runtime.authenticated_contacts.is_empty());
    assert_eq!(runtime.reported_hashes.len(), 0, "v=1 proves nothing");

    // The contact leaves the list: its sighting is lost.
    let events = runtime.reconcile(&[], NOW);
    assert_eq!(lost(&events), ["contact-older"]);
}

#[test]
fn entdeckung_task_the_memory_of_verified_contacts_follows_the_contact_list() {
    let (known, key) = KnownContact::with_key("contact-g2", 45);
    let contact = contact_of(&known, false);
    let mut runtime = runtime_knowing(&known);
    insert_verified(&mut runtime, &known, &key, "192.168.1.5", NOW, true);
    runtime.reconcile(std::slice::from_ref(&contact), NOW);
    assert_eq!(runtime.authenticated_contacts, ["contact-g2"]);

    // The contact was removed (the authenticator forgets it on its refresh).
    runtime.sightings.clear();
    runtime.proofs.clear();
    let events = runtime.reconcile(&[], LATER);
    assert!(runtime.authenticated_contacts.is_empty());
    assert_eq!(lost(&events), ["contact-g2"]);
}
