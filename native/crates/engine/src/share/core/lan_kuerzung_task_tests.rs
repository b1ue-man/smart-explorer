//! Task suite of the cut sighting (E26): a sighting keeps the addresses its
//! signed list names, never gains one the list does not name, and the
//! signature and the other checks stay as they were.
use super::*;

const NOW: i64 = 1800;
const SECRET: [u8; 32] = [9; 32];

fn key() -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[7; 32])
}

/// A sighting of `key`'s node that carries `addrs`.
fn sighting_of(key: &iroh::SecretKey, addrs: &[&str]) -> LanSighting {
    let node = key.public().to_string();
    LanSighting {
        id: rotating_id(&node, &SECRET, NOW / ID_EPOCH_SECS).expect("lookup id"),
        addrs: addrs
            .iter()
            .map(|addr| addr.parse::<IpAddr>().expect("address"))
            .collect(),
        p4: 51820,
        p6: 0,
        uplink: false,
        seen_at: NOW,
    }
}

/// The proof `key` signs for the addresses `sighting` carries now.
fn signed(sighting: &LanSighting, key: &iroh::SecretKey) -> LanProof {
    make_proof(
        sighting,
        NOW / ID_EPOCH_SECS,
        NOW + PROOF_LIFETIME_SECS,
        key,
    )
}

#[test]
fn lan_kuerzung_task_the_signed_address_stays_and_the_foreign_one_is_dropped() {
    let key = key();
    let node = key.public().to_string();
    let mut sighting = sighting_of(&key, &["192.168.4.2"]);
    let proof = signed(&sighting, &key);
    let signed_ip = "192.168.4.2".parse::<IpAddr>().expect("address");
    let foreign = "192.168.5.2".parse::<IpAddr>().expect("address");
    sighting.addrs = vec![signed_ip, foreign];

    let accepted = accepted_sighting(&sighting, &proof, &node, &SECRET, NOW).expect("accepted");
    assert_eq!(accepted.addrs, vec![signed_ip]);
    assert_eq!(accepted.id, sighting.id);
    assert_eq!(accepted.uplink, sighting.uplink);
}

#[test]
fn lan_kuerzung_task_a_sighting_without_any_signed_address_is_dropped() {
    let key = key();
    let node = key.public().to_string();
    let mut sighting = sighting_of(&key, &["192.168.4.2"]);
    let proof = signed(&sighting, &key);

    sighting.addrs = vec!["192.168.5.2".parse::<IpAddr>().expect("address")];
    assert!(accepted_sighting(&sighting, &proof, &node, &SECRET, NOW).is_none());
    sighting.addrs = Vec::new();
    assert!(accepted_sighting(&sighting, &proof, &node, &SECRET, NOW).is_none());
}

#[test]
fn lan_kuerzung_task_no_address_outside_the_signed_list_is_ever_adopted() {
    let key = key();
    let node = key.public().to_string();
    let mut sighting = sighting_of(&key, &["192.168.4.2", "192.168.4.3"]);
    let proof = signed(&sighting, &key);
    let foreign = "192.168.5.2".parse::<IpAddr>().expect("address");
    sighting.addrs = vec![
        "192.168.4.3".parse::<IpAddr>().expect("address"),
        foreign,
        "192.168.4.2".parse::<IpAddr>().expect("address"),
        foreign,
    ];

    let accepted = accepted_sighting(&sighting, &proof, &node, &SECRET, NOW).expect("accepted");
    assert_eq!(accepted.addrs.len(), 2);
    assert!(accepted.addrs.iter().all(|ip| proof.addresses.contains(ip)));
    assert!(!accepted.addrs.contains(&foreign));
}

#[test]
fn lan_kuerzung_task_the_signature_and_the_other_checks_are_unchanged() {
    let key = key();
    let node = key.public().to_string();
    let mut sighting = sighting_of(&key, &["192.168.4.2"]);
    let proof = signed(&sighting, &key);
    assert!(accepted_sighting(&sighting, &proof, &node, &SECRET, NOW).is_some());

    // The lookup id must match the pinned node and the secret.
    assert!(accepted_sighting(&sighting, &proof, &node, &[8; 32], NOW).is_none());

    // The uplink flag is part of the signed payload.
    sighting.uplink = true;
    assert!(accepted_sighting(&sighting, &proof, &node, &SECRET, NOW).is_none());
    sighting.uplink = false;

    // An address added to the signed list after signing breaks the signature.
    let mut forged = proof.clone();
    forged
        .addresses
        .push("192.168.5.2".parse::<IpAddr>().expect("address"));
    sighting
        .addrs
        .push("192.168.5.2".parse::<IpAddr>().expect("address"));
    assert!(accepted_sighting(&sighting, &forged, &node, &SECRET, NOW).is_none());

    // The proof is refused after its lifetime.
    let late = NOW + PROOF_LIFETIME_SECS + 1;
    assert!(accepted_sighting(&sighting, &proof, &node, &SECRET, late).is_none());
}

#[test]
fn lan_kuerzung_task_the_strict_verifier_takes_a_mixed_sighting_like_the_candidate_path() {
    let key = key();
    let node = key.public().to_string();
    let mut sighting = sighting_of(&key, &["192.168.4.2"]);
    let proof = signed(&sighting, &key);
    assert!(verify_sighting(&sighting, &proof, &node, &SECRET, NOW));

    // E26: the verifier takes a mixed sighting; the foreign address is dropped
    // and the signed one stays, so the verifier and `accepted_sighting` agree.
    sighting
        .addrs
        .push("192.168.5.2".parse::<IpAddr>().expect("address"));
    assert!(verify_sighting(&sighting, &proof, &node, &SECRET, NOW));
    let accepted = accepted_sighting(&sighting, &proof, &node, &SECRET, NOW).expect("accepted");
    assert_eq!(
        accepted.addrs,
        vec!["192.168.4.2".parse::<IpAddr>().expect("address")]
    );
}
