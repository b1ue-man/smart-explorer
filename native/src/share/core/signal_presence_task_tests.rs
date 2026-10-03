use super::*;

fn signed(random: String) -> PeerPresence {
    let key = iroh::SecretKey::from_bytes(&[6; 32]);
    let public = key.public().to_string();
    let mut presence = PeerPresence {
        kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: "peer".into(),
        device_name: "Trusted peer".into(),
        public_key: public.clone(),
        fingerprint: public_fingerprint(public.as_bytes()),
        node_id: public,
        relay_url: "https://relay.example/".into(),
        candidates: vec!["127.0.0.1:1234".into()],
        expires_at: 100,
        nonce: String::new(),
        proof: String::new(),
    };
    let signature = iroh_signature(&key, &signature_payload(&presence, &random));
    presence.nonce = format!("{random}{SIGNATURE_MARKER}{signature}");
    presence
}

#[test]
fn review_task_presence_signature_binds_names_fingerprint_node_and_routes() {
    let original = signed("random".into());
    assert_eq!(original.signature_state(), PresenceSignature::Valid);
    for index in 0..5 {
        let mut forged = original.clone();
        match index {
            0 => forged.device_name = "Forged".into(),
            1 => forged.fingerprint = "fake".into(),
            2 => forged.node_id = "another-node".into(),
            3 => forged.relay_url = "https://other.example/".into(),
            _ => forged.candidates.push("127.0.0.1:4321".into()),
        }
        assert_eq!(forged.signature_state(), PresenceSignature::Invalid);
    }
    let mut unsigned = original;
    unsigned.nonce = "legacy".into();
    unsigned.fingerprint = "untrusted-wire-text".into();
    let expected = public_fingerprint(unsigned.public_key.as_bytes());
    assert_eq!(unsigned.with_local_fingerprint().fingerprint, expected);
}

#[test]
fn review_task_legacy_decision_nonce_binds_recipient_and_value_within_wire_limit() {
    for accepted in [true, false] {
        let presence = signed(format!("{}ABCDEFGH", decision_context("local", accepted)));
        assert!(presence.nonce.len() <= 128);
        assert_eq!(presence.nonce.len(), 126);
        assert_eq!(presence.signature_state(), PresenceSignature::Valid);
        assert!(presence.matches_legacy_decision("local", accepted));
        assert!(!presence.matches_legacy_decision("another-device", accepted));
        assert!(!presence.matches_legacy_decision("local", !accepted));
    }
    assert!(!signed("random".into()).matches_legacy_decision("local", true));
}
