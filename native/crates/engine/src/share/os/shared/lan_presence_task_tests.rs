//! r-entdeckung-2 / -3 (suite-2-entdeckung): the `v=3` mDNS record on the
//! wire. The TXT records of the sealed announcement hold neither the signature
//! nor an address; a receiver with the announcer's Direct secret recovers
//! both from the resolved record and verifies the same signature as for
//! `v=2`; anything else is refused.
use std::net::IpAddr;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use super::*;
use crate::share::lan_privacy::{
    accepted_any_sighting, make_proof, open_proof, rotating_id, seal_proof, verify_any_sighting,
    verify_sighting, ID_EPOCH_SECS, PROOF_LIFETIME_SECS,
};

const NOW: i64 = 1_800;
const SECRET: [u8; 32] = [9; 32];

struct Announced {
    node: String,
    sighting: LanSighting,
    proof: LanProof,
    announcement: LanAnnouncement,
}

fn announced(addresses: &[&str]) -> Announced {
    let key = iroh::SecretKey::from_bytes(&[7; 32]);
    let node = key.public().to_string();
    let epoch = NOW / ID_EPOCH_SECS;
    let sighting = LanSighting {
        id: rotating_id(&node, &SECRET, epoch).expect("rotating id"),
        addrs: addresses
            .iter()
            .map(|address| address.parse::<IpAddr>().expect("address"))
            .collect(),
        p4: 51820,
        p6: 51821,
        uplink: false,
        seen_at: NOW,
    };
    let mut proof = make_proof(&sighting, epoch, NOW + PROOF_LIFETIME_SECS, &key);
    proof.sealed = seal_proof(&sighting, &proof, &SECRET);
    assert!(proof.sealed.is_some());
    let announcement = LanAnnouncement {
        hashed_id: sighting.id.clone(),
        p4: sighting.p4,
        p6: sighting.p6,
        uplink: false,
        plain_v2: false,
        retiring: Vec::new(),
    };
    Announced {
        node,
        sighting,
        proof,
        announcement,
    }
}

/// The record as a listener resolves it: the TXT properties of the
/// announcement, and the addresses the resolver found for the host.
fn resolved(
    prefix: &str,
    id: &str,
    ip: &str,
    properties: HashMap<String, String>,
) -> mdns_sd::ResolvedService {
    mdns_sd::ServiceInfo::new(
        SERVICE,
        &format!("{prefix}{id}"),
        &format!("{prefix}{id}.local."),
        ip,
        51820,
        properties,
    )
    .expect("service info")
    .as_resolved_service()
}

fn authenticated(event: Option<LanEvent>) -> (LanSighting, LanProof) {
    match event {
        Some(LanEvent::SeenAuthenticated { sighting, proof }) => (sighting, proof),
        other => panic!("expected an authenticated sighting, got {other:?}"),
    }
}

fn is_address_key(key: &str) -> bool {
    key.len() > 1 && key.starts_with('a') && key[1..].bytes().all(|byte| byte.is_ascii_digit())
}

fn chunk_keys(properties: &HashMap<String, String>) -> usize {
    properties
        .keys()
        .filter(|key| {
            key.len() > 1 && key.starts_with('c') && key[1..].bytes().all(|b| b.is_ascii_digit())
        })
        .count()
}

#[test]
fn entdeckung_task_sealed_record_carries_neither_signature_nor_addresses_in_clear() {
    let announced = announced(&["10.8.0.2", "192.168.1.5"]);
    let sealed = announced.proof.sealed.clone().expect("sealed");
    let properties = sealed_properties(&announced.announcement, &announced.proof, &sealed);
    assert_eq!(properties["v"], "3");
    assert_eq!(properties["id"], announced.sighting.id);
    assert!(!properties.contains_key("s"), "no clear signature");
    assert!(
        !properties.keys().any(|key| is_address_key(key)),
        "no clear address list"
    );
    for value in properties.values() {
        assert!(!value.contains("10.8.0.2") && !value.contains("192.168.1.5"));
        assert!(!value.contains(&announced.proof.signature));
    }
    // The chunks hold exactly the seal.
    let mut joined = String::new();
    for index in 0..chunk_keys(&properties) {
        let chunk = &properties[&format!("c{index}")];
        assert!(chunk.len() <= 200);
        joined.push_str(chunk);
    }
    assert_eq!(
        URL_SAFE_NO_PAD.decode(joined).expect("base64"),
        sealed.ciphertext
    );
    assert_eq!(
        URL_SAFE_NO_PAD.decode(&properties["n"]).expect("base64"),
        sealed.nonce.to_vec()
    );

    // The plain `v=2` record shows what the seal hides.
    let plain = plain_properties(&announced.announcement, Some(&announced.proof));
    assert_eq!(plain["v"], "2");
    assert_eq!(plain["s"], announced.proof.signature);
    assert_eq!(plain["a0"], "10.8.0.2");
}

#[test]
fn entdeckung_task_a_listener_with_the_secret_recovers_addresses_and_signature() {
    let announced = announced(&["10.8.0.2", "192.168.1.5"]);
    let sealed = announced.proof.sealed.clone().expect("sealed");
    let properties = sealed_properties(&announced.announcement, &announced.proof, &sealed);
    let info = resolved(
        SEALED_INSTANCE_PREFIX,
        &announced.sighting.id,
        "192.168.1.5",
        properties,
    );
    let (sighting, proof) = authenticated(event_from(&info));
    assert_eq!(sighting.id, announced.sighting.id);
    assert_eq!(
        sighting.addrs,
        vec!["192.168.1.5".parse::<IpAddr>().unwrap()]
    );
    assert_eq!(
        (sighting.p4, sighting.p6, sighting.uplink),
        (51820, 51821, false)
    );
    // The received proof holds only the seal.
    assert!(proof.addresses.is_empty() && proof.signature.is_empty());
    assert_eq!(proof.sealed, announced.proof.sealed);
    assert_eq!(
        (proof.epoch, proof.expires_at),
        (announced.proof.epoch, announced.proof.expires_at)
    );
    let opened = open_proof(&sighting, &proof, &SECRET).expect("opens with the secret");
    assert_eq!(opened.addresses, announced.proof.addresses);
    assert_eq!(opened.signature, announced.proof.signature);
    assert!(verify_any_sighting(
        &sighting,
        &proof,
        &announced.node,
        &SECRET,
        NOW
    ));
    // Older receivers find the id in the instance name too.
    assert_eq!(
        id_from_fullname(info.get_fullname()),
        Some(announced.sighting.id.clone())
    );
}

#[test]
fn entdeckung_task_a_long_sealed_advisory_is_split_into_chunks_and_reassembled() {
    let addresses: Vec<String> = (1..=20)
        .map(|index| format!("2001:db8::{index:x}"))
        .collect();
    let refs: Vec<&str> = addresses.iter().map(String::as_str).collect();
    let announced = announced(&refs);
    let sealed = announced.proof.sealed.clone().expect("sealed");
    let properties = sealed_properties(&announced.announcement, &announced.proof, &sealed);
    let chunks = chunk_keys(&properties);
    assert!((2..=8).contains(&chunks), "{chunks} chunks");
    assert!(!properties.keys().any(|key| is_address_key(key)));
    let info = resolved(
        SEALED_INSTANCE_PREFIX,
        &announced.sighting.id,
        "2001:db8::1,2001:db8::2",
        properties,
    );
    let (sighting, proof) = authenticated(event_from(&info));
    assert_eq!(sighting.addrs.len(), 2);
    assert!(verify_any_sighting(
        &sighting,
        &proof,
        &announced.node,
        &SECRET,
        NOW
    ));
}

#[test]
fn entdeckung_task_a_resolved_sealed_record_with_a_wrong_secret_or_changed_fields_is_refused() {
    let announced = announced(&["10.8.0.2", "192.168.1.5"]);
    let sealed = announced.proof.sealed.clone().expect("sealed");
    let properties = sealed_properties(&announced.announcement, &announced.proof, &sealed);
    let record = |ip: &str, properties: HashMap<String, String>| {
        resolved(
            SEALED_INSTANCE_PREFIX,
            &announced.sighting.id,
            ip,
            properties,
        )
    };

    let (honest, proof) = authenticated(event_from(&record("192.168.1.5", properties.clone())));
    assert!(verify_any_sighting(
        &honest,
        &proof,
        &announced.node,
        &SECRET,
        NOW
    ));
    // A wrong secret.
    assert!(!verify_any_sighting(
        &honest,
        &proof,
        &announced.node,
        &[8; 32],
        NOW
    ));
    // An address injected into the unsigned record next to a signed one: the
    // sighting is cut to the signed address, the injected one is dropped (E26).
    let (injected, injected_proof) = authenticated(event_from(&record(
        "192.168.1.5,192.168.9.9",
        properties.clone(),
    )));
    assert_eq!(injected.addrs.len(), 2);
    assert!(verify_any_sighting(
        &injected,
        &injected_proof,
        &announced.node,
        &SECRET,
        NOW
    ));
    let cut = accepted_any_sighting(&injected, &injected_proof, &announced.node, &SECRET, NOW);
    assert_eq!(
        cut.map(|sighting| sighting.addrs),
        Some(vec!["192.168.1.5".parse::<IpAddr>().expect("address")])
    );
    // The clear epoch and expiry of the record are bound into the seal.
    for key in ["e", "x"] {
        let mut changed = properties.clone();
        let value: i64 = changed[key].parse().expect("number");
        changed.insert(key.into(), (value + 1).to_string());
        let (sighting, proof) = authenticated(event_from(&record("192.168.1.5", changed)));
        assert!(
            !verify_any_sighting(&sighting, &proof, &announced.node, &SECRET, NOW),
            "changed {key}"
        );
    }
    // A missing or oversized chunk is no record.
    let mut missing = properties.clone();
    missing.remove("c0");
    assert!(event_from(&record("192.168.1.5", missing)).is_none());
    let mut oversized = properties;
    oversized.insert("c0".into(), "A".repeat(201));
    assert!(event_from(&record("192.168.1.5", oversized)).is_none());
}

#[test]
fn entdeckung_task_plain_v2_v1_records_and_instance_names_stay_compatible() {
    let announced = announced(&["192.168.1.5"]);
    let plain = plain_properties(&announced.announcement, Some(&announced.proof));
    let info = resolved(
        INSTANCE_PREFIX,
        &announced.sighting.id,
        "192.168.1.5",
        plain,
    );
    let (sighting, proof) = authenticated(event_from(&info));
    assert!(proof.sealed.is_none());
    assert_eq!(proof.addresses, announced.proof.addresses);
    assert_eq!(proof.signature, announced.proof.signature);
    // For a plain record both checks give the same verdict.
    for now in [NOW, NOW + 1_000] {
        assert_eq!(
            verify_any_sighting(&sighting, &proof, &announced.node, &SECRET, now),
            verify_sighting(&sighting, &proof, &announced.node, &SECRET, now)
        );
    }
    assert!(verify_any_sighting(
        &sighting,
        &proof,
        &announced.node,
        &SECRET,
        NOW
    ));

    // A `v=1` record has no proof and stays a plain sighting.
    let v1 = LanAnnouncement {
        hashed_id: "0123456789abcdef".into(),
        ..announced.announcement.clone()
    };
    let info = resolved(
        INSTANCE_PREFIX,
        "0123456789abcdef",
        "192.168.1.5",
        plain_properties(&v1, None),
    );
    assert!(matches!(event_from(&info), Some(LanEvent::Seen(_))));

    // Instance names: the sealed `se3-` and the plain `se-` carry the id.
    let id = announced.sighting.id.clone();
    for prefix in ["se3-", "se-"] {
        assert_eq!(
            id_from_fullname(&format!("{prefix}{id}._se-share._udp.local.")),
            Some(id.clone())
        );
    }
    assert!(id_from_fullname(&format!("other-{id}._se-share._udp.local.")).is_none());
}

#[test]
fn code_renewal_task_retiring_announcement_yields_a_sealed_and_a_plain_record_under_its_id() {
    let announced = announced(&["192.168.1.5"]);
    let retiring_id = "00112233445566778899aabbccddeeff".to_string();
    let announcing = |proof: LanProof| LanAnnouncement {
        retiring: vec![RetiringLanAnnouncement {
            hashed_id: retiring_id.clone(),
            proof,
        }],
        ..announced.announcement.clone()
    };

    // The sealed entry yields the `v=3` record and the plain `v=2` record, both
    // under the retiring id, and one view of that id without retiring entries.
    let mut records = Vec::new();
    let mut views = Vec::new();
    super::retiring::append_records(
        &announcing(announced.proof.clone()),
        &mut records,
        &mut views,
    );
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].0, SEALED_INSTANCE_PREFIX);
    assert_eq!(records[1].0, INSTANCE_PREFIX);
    assert_eq!(records[0].1["v"], "3");
    assert_eq!(records[1].1["v"], "2");
    assert!(records
        .iter()
        .all(|(_, properties)| properties["id"] == retiring_id));
    assert_eq!(views.len(), 2);
    assert!(views
        .iter()
        .all(|view| view.hashed_id == retiring_id && view.retiring.is_empty()));

    // An entry without seal yields only the plain record.
    let mut records = Vec::new();
    let mut views = Vec::new();
    super::retiring::append_records(
        &announcing(LanProof {
            sealed: None,
            ..announced.proof.clone()
        }),
        &mut records,
        &mut views,
    );
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].0, INSTANCE_PREFIX);
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].hashed_id, retiring_id);
}

#[test]
fn unbelegt_task_unsigned_sighting_drops_multicast() {
    // An unsigned record whose resolved addresses include a multicast one
    // keeps only the unicast address, so no dial candidate can name the
    // multicast one and the remaining address still counts.
    let properties = HashMap::from([("p4".to_string(), "51820".to_string())]);
    let info = resolved(
        INSTANCE_PREFIX,
        "0123456789abcdef",
        "224.0.0.251,192.168.1.9",
        properties,
    );
    match event_from(&info) {
        Some(LanEvent::Seen(sighting)) => {
            let unicast: IpAddr = "192.168.1.9".parse().expect("address");
            assert_eq!(sighting.addrs, vec![unicast]);
        }
        other => panic!("expected a plain sighting, got {other:?}"),
    }
}
