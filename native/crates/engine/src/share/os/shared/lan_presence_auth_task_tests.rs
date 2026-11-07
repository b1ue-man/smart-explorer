//! r-entdeckung-2 / -3 (suite-2-entdeckung): the plain `v=2` record is
//! announced next to the sealed `v=3` one until every audience contact
//! showed that it reads `v=3`. The helpers build an authenticator without
//! credentials (no `refresh`) and the announcements a contact's device makes;
//! the daemon's LAN tests use them too.
use std::net::IpAddr;

use super::*;
use crate::share::lan_privacy::{
    make_proof, rotating_id, seal_proof, ID_EPOCH_SECS, PROOF_LIFETIME_SECS,
};
use crate::share::power::test_support::identity;

const NOW: i64 = 1_800;

/// A contact as the authenticator holds it: id, pinned node, Direct secret.
pub(crate) struct KnownContact {
    pub(crate) id: String,
    pub(crate) node: String,
    pub(crate) secret: Vec<u8>,
}

impl KnownContact {
    /// A contact with a deterministic device key; returns the key too.
    pub(crate) fn with_key(id: &str, seed: u8) -> (Self, iroh::SecretKey) {
        let key = iroh::SecretKey::from_bytes(&[seed; 32]);
        let contact = Self {
            id: id.to_string(),
            node: key.public().to_string(),
            secret: vec![seed; 32],
        };
        (contact, key)
    }
}

/// An authenticator that knows `contacts` (their rotating ids for the epochs
/// around `now`) and announces as `own`.
pub(crate) fn authenticator_for(
    own: &ShareIdentity,
    contacts: &[KnownContact],
    now: i64,
) -> LanAuthenticator {
    let mut authenticator = LanAuthenticator::new();
    for contact in contacts {
        authenticator.principals.push(Principal {
            contact_id: contact.id.clone(),
            node: contact.node.clone(),
            secret: contact.secret.clone(),
        });
    }
    let epoch = now.div_euclid(ID_EPOCH_SECS);
    for (index, principal) in authenticator.principals.iter().enumerate() {
        for epoch in [epoch - 1, epoch, epoch + 1] {
            if let Some(id) = rotating_id(&principal.node, &principal.secret, epoch) {
                authenticator.ids.insert(id, index);
            }
        }
    }
    authenticator.audience = contacts
        .iter()
        .map(|contact| (contact.id.clone(), contact.node.clone(), String::new()))
        .collect();
    authenticator.identity = Some(own.clone());
    authenticator.own_node = Some(own.node_id.clone());
    authenticator.refreshed_at = Some(Instant::now());
    authenticator
}

/// What the device of `contact` announces and a listener receives: the
/// sighting with the resolved addresses and the proof, as the sealed `v=3`
/// form (only the seal) or as the plain `v=2` form.
pub(crate) fn announcement_of(
    contact: &KnownContact,
    key: &iroh::SecretKey,
    addresses: &[IpAddr],
    now: i64,
    sealed: bool,
) -> (LanSighting, LanProof) {
    let epoch = now.div_euclid(ID_EPOCH_SECS);
    let sighting = LanSighting {
        id: rotating_id(&contact.node, &contact.secret, epoch).expect("rotating id"),
        addrs: addresses.to_vec(),
        p4: 51820,
        p6: 0,
        uplink: false,
        seen_at: now,
    };
    let mut proof = make_proof(&sighting, epoch, now + PROOF_LIFETIME_SECS, key);
    if sealed {
        let seal = seal_proof(&sighting, &proof, &contact.secret).expect("seal");
        proof = LanProof {
            epoch: proof.epoch,
            expires_at: proof.expires_at,
            addresses: Vec::new(),
            signature: String::new(),
            sealed: Some(seal),
        };
    }
    (sighting, proof)
}

fn announce(authenticator: &LanAuthenticator) -> (LanAnnouncement, LanProof) {
    authenticator
        .announcement(
            (Some(51820), None),
            false,
            vec!["192.168.1.7".parse().expect("address")],
            NOW,
        )
        .expect("announcement")
}

#[test]
fn entdeckung_task_plain_v2_is_announced_until_the_listener_showed_a_verified_v3() {
    let own = identity("lan own").expect("identity");
    let (contact, key) = KnownContact::with_key("contact-a", 21);
    let authenticator = authenticator_for(&own, std::slice::from_ref(&contact), NOW);
    let address: IpAddr = "192.168.1.5".parse().expect("address");

    let (announcement, proof) = announce(&authenticator);
    assert!(proof.sealed.is_some(), "the sealed record is always made");
    assert!(
        announcement.plain_v2,
        "plain v=2 until the listener reads v=3"
    );

    // A plain v=2 announcement of the contact proves nothing about v=3.
    let (sighting, plain) = announcement_of(&contact, &key, &[address], NOW, false);
    assert_eq!(
        authenticator.authenticate(&sighting, &plain, NOW),
        Some("contact-a".to_string())
    );
    assert!(announce(&authenticator).0.plain_v2);

    // A sealed record under another secret does not authenticate.
    let (stranger, stranger_key) = KnownContact::with_key("contact-a", 22);
    let stranger = KnownContact {
        node: contact.node.clone(),
        ..stranger
    };
    let (sighting, forged) = announcement_of(&stranger, &stranger_key, &[address], NOW, true);
    assert_eq!(authenticator.authenticate(&sighting, &forged, NOW), None);
    assert!(announce(&authenticator).0.plain_v2);

    // The contact's verified sealed record ends the plain announcement.
    let (sighting, sealed) = announcement_of(&contact, &key, &[address], NOW, true);
    assert_eq!(
        authenticator.authenticate(&sighting, &sealed, NOW),
        Some("contact-a".to_string())
    );
    assert!(!announce(&authenticator).0.plain_v2);
}

#[test]
fn entdeckung_task_plain_v2_stays_until_every_audience_contact_showed_v3() {
    let own = identity("lan own two").expect("identity");
    let (first, first_key) = KnownContact::with_key("contact-a", 31);
    let (second, second_key) = KnownContact::with_key("contact-b", 32);
    let address: IpAddr = "192.168.1.5".parse().expect("address");
    let contacts = [first, second];
    let authenticator = authenticator_for(&own, &contacts, NOW);

    let (sighting, proof) = announcement_of(&contacts[0], &first_key, &[address], NOW, true);
    assert!(authenticator.authenticate(&sighting, &proof, NOW).is_some());
    assert!(
        announce(&authenticator).0.plain_v2,
        "the second contact has not shown v=3"
    );
    let (sighting, proof) = announcement_of(&contacts[1], &second_key, &[address], NOW, true);
    assert!(authenticator.authenticate(&sighting, &proof, NOW).is_some());
    assert!(!announce(&authenticator).0.plain_v2);
}

// E17 (cr-F4d, Plan 11.5, M21): the LAN announcement under a lookup id that
// runs out after a Direct-code renewal.
use crate::share::identity::code_renewal_registry::{self as registry, RetiringMaterial};
use crate::share::lan_privacy::verify_any_sighting;
use zeroize::Zeroizing;

/// Empties the process view of one node when the test ends, also on a panic.
struct ResetView(String);

impl Drop for ResetView {
    fn drop(&mut self) {
        registry::install(&self.0, "", Vec::new(), Vec::new(), Default::default());
    }
}

#[test]
fn code_renewal_task_lan_announcement_seals_the_retiring_lookup_id_with_its_old_secret() {
    let own = identity("code renewal lan retiring").expect("identity");
    let (contact, _) = KnownContact::with_key("contact-l21", 212);
    let authenticator = authenticator_for(&own, std::slice::from_ref(&contact), NOW);
    let _reset = ResetView(own.node_id.clone());
    let address: IpAddr = "192.168.1.7".parse().expect("address");

    // Without a registry view for this node no lookup id runs out.
    assert!(announce(&authenticator).0.retiring.is_empty());

    // A lookup id that runs out is announced under its own rotating id, sealed
    // with its old secret: that secret verifies it, the direct secret does not.
    registry::install(
        &own.node_id,
        "lookup-current-l21",
        vec![RetiringMaterial {
            lookup_id: "lookup-old-l21".into(),
            secret: Zeroizing::new(vec![7; 32]),
            pending: vec![(
                contact.id.clone(),
                contact.node.clone(),
                contact.node.clone(),
            )],
        }],
        Vec::new(),
        Default::default(),
    );
    let (announcement, _) = announce(&authenticator);
    assert_eq!(announcement.retiring.len(), 1);
    let retiring = &announcement.retiring[0];
    assert_eq!(
        rotating_id(&own.node_id, &[7; 32], NOW.div_euclid(ID_EPOCH_SECS)),
        Some(retiring.hashed_id.clone())
    );
    assert!(
        retiring.proof.sealed.is_some(),
        "the retiring record is sealed"
    );
    let sighting = LanSighting {
        id: retiring.hashed_id.clone(),
        addrs: vec![address],
        p4: 51820,
        p6: 0,
        uplink: false,
        seen_at: NOW,
    };
    assert!(verify_any_sighting(
        &sighting,
        &retiring.proof,
        &own.node_id,
        &[7; 32],
        NOW
    ));
    assert!(!verify_any_sighting(
        &sighting,
        &retiring.proof,
        &own.node_id,
        &own.direct_secret,
        NOW
    ));

    // When the lookup id ends, nothing is announced under it any more.
    registry::install(
        &own.node_id,
        "lookup-current-l21",
        Vec::new(),
        Vec::new(),
        Default::default(),
    );
    assert!(announce(&authenticator).0.retiring.is_empty());
}
