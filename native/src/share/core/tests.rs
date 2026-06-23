use super::core::{
    hmac_proof, presence_payload, random_bytes, relation_psk, room_psk, sanitize_name, verify_hmac,
};
use super::profiles::ShareProfiles;
use super::wire::{Ctrl, FileMeta};

#[test]
fn relation_psk_is_stable_and_relation_specific() {
    let secret = random_bytes::<32>();
    assert_eq!(
        relation_psk("direct", &secret, "device-a", "device-b"),
        relation_psk("direct", &secret, "device-b", "device-a")
    );
    assert_ne!(
        relation_psk("direct", &secret, "device-a", "device-b"),
        room_psk(&secret, "room-a")
    );
}

#[test]
fn room_code_uses_persistent_secret_format() {
    let code = ShareProfiles::new_room_code();
    let parts: Vec<&str> = code.split('-').collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], "SE");
    assert_eq!(parts[1], "R1");
    assert_eq!(parts[3].len(), 64);
}

#[test]
fn presence_hmac_covers_candidates() {
    let secret = random_bytes::<32>();
    let candidates = vec!["192.168.1.20:1234".to_string()];
    let payload = presence_payload(
        "direct",
        "lookup",
        "device-a",
        "pubkey",
        &candidates,
        42,
        "nonce",
    );
    let proof = hmac_proof(&secret, &payload);
    assert!(verify_hmac(&secret, &payload, &proof));

    let changed_payload = presence_payload(
        "direct",
        "lookup",
        "device-a",
        "pubkey",
        &["10.0.0.5:22".to_string()],
        42,
        "nonce",
    );
    assert!(!verify_hmac(&secret, &changed_payload, &proof));
}

#[test]
fn sanitize_strips_separators() {
    assert_eq!(sanitize_name("../e/t\\c:passwd"), "_e_t_c_passwd");
    assert_eq!(sanitize_name(""), "datei");
}

#[test]
fn ctrl_roundtrips() {
    let o = Ctrl::Offer {
        from: "A".into(),
        files: vec![FileMeta {
            name: "x".into(),
            size: 3,
        }],
    };
    let j = serde_json::to_vec(&o).unwrap();
    assert!(matches!(
        serde_json::from_slice::<Ctrl>(&j).unwrap(),
        Ctrl::Offer { .. }
    ));
}
