//! r-entdeckung-2 / -3 (suite-2-entdeckung): the sealed LAN advisory (`v=3`)
//! hides the signature and the address list from everybody without the
//! announcer's Direct secret; a holder opens it and verifies the same
//! signature and rules as for `v=2`.
use super::*;

const NOW: i64 = 1_800;
const SECRET: [u8; 32] = [9; 32];

struct Fixture {
    node: String,
    sighting: LanSighting,
    /// The announcer's own proof (plain fields and seal).
    proof: LanProof,
    /// The proof as a listener receives it: only the seal.
    received: LanProof,
}

fn address(text: &str) -> IpAddr {
    text.parse().expect("address")
}

fn fixture() -> Fixture {
    let key = iroh::SecretKey::from_bytes(&[7; 32]);
    let node = key.public().to_string();
    let epoch = NOW / ID_EPOCH_SECS;
    let sighting = LanSighting {
        id: rotating_id(&node, &SECRET, epoch).expect("rotating id"),
        addrs: vec![address("10.8.0.2"), address("192.168.1.5")],
        p4: 51820,
        p6: 0,
        uplink: false,
        seen_at: NOW,
    };
    let mut proof = make_proof(&sighting, epoch, NOW + PROOF_LIFETIME_SECS, &key);
    proof.sealed = seal_proof(&sighting, &proof, &SECRET);
    assert!(proof.sealed.is_some());
    let received = LanProof {
        epoch: proof.epoch,
        expires_at: proof.expires_at,
        addresses: Vec::new(),
        signature: String::new(),
        sealed: proof.sealed.clone(),
    };
    Fixture {
        node,
        sighting,
        proof,
        received,
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[test]
fn entdeckung_task_a_sealed_proof_exposes_neither_signature_nor_addresses() {
    let fixture = fixture();
    let sealed = fixture.proof.sealed.as_ref().expect("sealed");
    assert!(fixture.received.signature.is_empty());
    assert!(fixture.received.addresses.is_empty());
    // Nothing a listener could test against known node ids.
    assert!(!contains(
        &sealed.ciphertext,
        fixture.proof.signature.as_bytes()
    ));
    assert!(!contains(&sealed.ciphertext, &[10, 8, 0, 2]));
    assert!(!contains(&sealed.ciphertext, &[192, 168, 1, 5]));
    let wire = serde_json::to_string(&fixture.received).expect("json");
    assert!(!wire.contains(&fixture.proof.signature));
    assert!(!wire.contains("10.8.0.2") && !wire.contains("192.168.1.5"));
    // Without the secret the received form verifies as nothing.
    assert!(!verify_sighting(
        &fixture.sighting,
        &fixture.received,
        &fixture.node,
        &SECRET,
        NOW
    ));
    assert!(!verify_any_sighting(
        &fixture.sighting,
        &fixture.received,
        &fixture.node,
        &[8; 32],
        NOW
    ));
}

#[test]
fn entdeckung_task_opening_with_the_secret_yields_both_addresses_and_a_valid_signature() {
    let fixture = fixture();
    let opened = open_proof(&fixture.sighting, &fixture.received, &SECRET).expect("opens");
    assert_eq!(opened.addresses, fixture.proof.addresses);
    assert_eq!(opened.signature, fixture.proof.signature);
    assert!(opened.sealed.is_none());
    assert!(verify_sighting(
        &fixture.sighting,
        &opened,
        &fixture.node,
        &SECRET,
        NOW
    ));
    // A listener resolved one of the announced addresses.
    let mut resolved = fixture.sighting.clone();
    resolved.addrs = vec![address("192.168.1.5")];
    assert!(verify_any_sighting(
        &resolved,
        &fixture.received,
        &fixture.node,
        &SECRET,
        NOW
    ));
    // A plain `v=2` proof goes through the old check unchanged.
    let mut plain = fixture.proof.clone();
    plain.sealed = None;
    assert!(verify_any_sighting(
        &fixture.sighting,
        &plain,
        &fixture.node,
        &SECRET,
        NOW
    ));
    assert_eq!(
        verify_any_sighting(
            &fixture.sighting,
            &plain,
            &fixture.node,
            &SECRET,
            NOW + 1_000
        ),
        verify_sighting(
            &fixture.sighting,
            &plain,
            &fixture.node,
            &SECRET,
            NOW + 1_000
        )
    );
}

#[test]
fn entdeckung_task_a_wrong_secret_changed_clear_fields_or_an_injected_address_are_refused() {
    let fixture = fixture();
    let verifies = |sighting: &LanSighting, proof: &LanProof, secret: &[u8]| {
        verify_any_sighting(sighting, proof, &fixture.node, secret, NOW)
    };
    assert!(verifies(&fixture.sighting, &fixture.received, &SECRET));

    // Wrong secret.
    assert!(open_proof(&fixture.sighting, &fixture.received, &[8; 32]).is_none());
    assert!(!verifies(&fixture.sighting, &fixture.received, &[8; 32]));
    // The clear epoch and expiry are bound into the seal.
    let mut other_epoch = fixture.received.clone();
    other_epoch.epoch += 1;
    assert!(open_proof(&fixture.sighting, &other_epoch, &SECRET).is_none());
    assert!(!verifies(&fixture.sighting, &other_epoch, &SECRET));
    let mut other_expiry = fixture.received.clone();
    other_expiry.expires_at += 1;
    assert!(open_proof(&fixture.sighting, &other_expiry, &SECRET).is_none());
    assert!(!verifies(&fixture.sighting, &other_expiry, &SECRET));
    // So is the rotating id of the record.
    let mut other_id = fixture.sighting.clone();
    other_id.id = rotating_id(&fixture.node, &SECRET, NOW / ID_EPOCH_SECS + 1).expect("id");
    assert!(open_proof(&other_id, &fixture.received, &SECRET).is_none());
    // A changed seal.
    let mut tampered = fixture.received.clone();
    let sealed = tampered.sealed.as_mut().expect("sealed");
    sealed.ciphertext[0] ^= 0x01;
    assert!(open_proof(&fixture.sighting, &tampered, &SECRET).is_none());
    let mut other_nonce = fixture.received.clone();
    other_nonce.sealed.as_mut().expect("sealed").nonce[0] ^= 0x01;
    assert!(open_proof(&fixture.sighting, &other_nonce, &SECRET).is_none());
    // An address resolved from an unsigned record is not in the signed list: it
    // is dropped (E26), the signed address stays, so the sighting verifies with
    // that address only.
    let mut injected = fixture.sighting.clone();
    injected.addrs = vec![address("192.168.1.5"), address("192.168.9.9")];
    assert!(verifies(&injected, &fixture.received, &SECRET));
    let accepted = accepted_any_sighting(&injected, &fixture.received, &fixture.node, &SECRET, NOW);
    assert_eq!(
        accepted.map(|cut| cut.addrs),
        Some(vec![address("192.168.1.5")])
    );
    // An expired proof stays expired.
    assert!(!verify_any_sighting(
        &fixture.sighting,
        &fixture.received,
        &fixture.node,
        &SECRET,
        NOW + PROOF_LIFETIME_SECS + 1
    ));
}

#[test]
fn entdeckung_task_a_seal_needs_a_32_byte_secret_and_a_signature() {
    let fixture = fixture();
    assert!(seal_proof(&fixture.sighting, &fixture.proof, &[9; 16]).is_none());
    let mut unsigned = fixture.proof.clone();
    unsigned.signature.clear();
    assert!(seal_proof(&fixture.sighting, &unsigned, &SECRET).is_none());
    let mut without_addresses = fixture.proof.clone();
    without_addresses.addresses.clear();
    assert!(seal_proof(&fixture.sighting, &without_addresses, &SECRET).is_none());
}
