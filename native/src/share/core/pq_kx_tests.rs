use std::sync::Arc;

use ring::signature::KeyPair;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::AlwaysResolvesServerRawPublicKeys;
use rustls::sign::CertifiedKey;
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, ServerConfig, ServerConnection,
    SignatureScheme,
};

use super::*;

#[test]
fn luecken_task_hybrid_group_agrees_on_one_64_byte_secret() {
    let client = X25519MLKEM768.start().unwrap();
    assert_eq!(client.group(), NamedGroup::X25519MLKEM768);
    assert_eq!(
        client.pub_key().len(),
        MLKEM768_ENCAPSULATION_KEY_LEN + X25519_LEN
    );
    let server = X25519MLKEM768.start_and_complete(client.pub_key()).unwrap();
    assert_eq!(server.group, NamedGroup::X25519MLKEM768);
    assert_eq!(server.pub_key.len(), MLKEM768_CIPHERTEXT_LEN + X25519_LEN);
    let client_secret = client.complete(&server.pub_key).unwrap();
    assert_eq!(client_secret.secret_bytes().len(), HYBRID_SECRET_LEN);
    assert_eq!(client_secret.secret_bytes(), server.secret.secret_bytes());
}

#[test]
fn luecken_task_hybrid_share_carries_a_classical_x25519_part_for_old_peers() {
    let client = X25519MLKEM768.start().unwrap();
    let (group, share) = client.hybrid_component().unwrap();
    assert_eq!(group, NamedGroup::X25519);
    assert_eq!(share.len(), X25519_LEN);
    assert_eq!(&client.pub_key()[MLKEM768_ENCAPSULATION_KEY_LEN..], share);
    let old_server = rustls::crypto::ring::kx_group::X25519
        .start_and_complete(share)
        .unwrap();
    let client_secret = client
        .complete_hybrid_component(&old_server.pub_key)
        .unwrap();
    assert_eq!(
        client_secret.secret_bytes(),
        old_server.secret.secret_bytes()
    );
}

#[test]
fn luecken_task_malformed_hybrid_shares_are_rejected() {
    assert!(X25519MLKEM768.start_and_complete(&[0u8; 31]).is_err());
    let client = X25519MLKEM768.start().unwrap();
    assert!(client.complete(&[7u8; 40]).is_err());
}

#[test]
fn luecken_task_share_provider_prefers_the_hybrid_and_keeps_ring_defaults() {
    let names: Vec<NamedGroup> = share_crypto_provider()
        .kx_groups
        .iter()
        .map(|group| group.name())
        .collect();
    assert_eq!(
        names,
        vec![
            NamedGroup::X25519MLKEM768,
            NamedGroup::X25519,
            NamedGroup::secp256r1,
            NamedGroup::secp384r1,
        ]
    );
    assert!(!X25519MLKEM768.usable_for_version(ProtocolVersion::TLSv1_2));
}

#[test]
fn luecken_task_updated_peers_negotiate_x25519mlkem768_in_a_real_tls13_handshake() {
    let provider = share_crypto_provider();
    let (client, server) = handshake(provider.clone(), provider);
    assert_eq!(
        client
            .negotiated_key_exchange_group()
            .map(|group| group.name()),
        Some(NamedGroup::X25519MLKEM768)
    );
    assert_eq!(
        server
            .negotiated_key_exchange_group()
            .map(|group| group.name()),
        Some(NamedGroup::X25519MLKEM768)
    );
}

#[test]
fn luecken_task_updated_client_and_old_host_fall_back_to_x25519() {
    let old = Arc::new(rustls::crypto::ring::default_provider());
    let (client, server) = handshake(share_crypto_provider(), old.clone());
    assert_eq!(
        client
            .negotiated_key_exchange_group()
            .map(|group| group.name()),
        Some(NamedGroup::X25519)
    );
    assert_eq!(
        server
            .negotiated_key_exchange_group()
            .map(|group| group.name()),
        Some(NamedGroup::X25519)
    );
    let (client, _) = handshake(old, share_crypto_provider());
    assert_eq!(
        client
            .negotiated_key_exchange_group()
            .map(|group| group.name()),
        Some(NamedGroup::X25519)
    );
}

/// One complete in-memory TLS 1.3 handshake with Ed25519 raw public keys, the
/// way iroh authenticates Share endpoints (RFC 7250).
fn handshake(
    client_provider: Arc<CryptoProvider>,
    server_provider: Arc<CryptoProvider>,
) -> (ClientConnection, ServerConnection) {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let spki = rustls::sign::public_key_to_spki(
        &rustls::pki_types::alg_id::ED25519,
        pair.public_key().as_ref(),
    );
    let signing = server_provider
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            pkcs8.as_ref().to_vec(),
        )))
        .unwrap();
    let certified = CertifiedKey::new(vec![CertificateDer::from(spki.to_vec())], signing);
    let server_config = ServerConfig::builder_with_provider(server_provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(AlwaysResolvesServerRawPublicKeys::new(Arc::new(
            certified,
        ))));
    let client_config = ClientConfig::builder_with_provider(client_provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptRawKey))
        .with_no_client_auth();
    let mut client = ClientConnection::new(
        Arc::new(client_config),
        ServerName::try_from("peer.invalid").unwrap(),
    )
    .unwrap();
    let mut server = ServerConnection::new(Arc::new(server_config)).unwrap();
    for _ in 0..16 {
        if !client.is_handshaking() && !server.is_handshaking() {
            break;
        }
        let mut to_server = Vec::new();
        client.write_tls(&mut to_server).unwrap();
        let mut reader = to_server.as_slice();
        while !reader.is_empty() {
            server.read_tls(&mut reader).unwrap();
            server.process_new_packets().unwrap();
        }
        let mut to_client = Vec::new();
        server.write_tls(&mut to_client).unwrap();
        let mut reader = to_client.as_slice();
        while !reader.is_empty() {
            client.read_tls(&mut reader).unwrap();
            client.process_new_packets().unwrap();
        }
    }
    assert!(!client.is_handshaking() && !server.is_handshaking());
    (client, server)
}

/// Test-only verifier: the key exchange is under test, not peer identity.
#[derive(Debug)]
struct AcceptRawKey;

impl ServerCertVerifier for AcceptRawKey {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is not offered".into()))
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

// --- RFC 10024 conformance, built from primitives -------------------------
//
// These tests never call the code under test for the peer role they assemble:
// the other side is built by hand from `ml-kem` and ring's X25519 so a wrong
// order of the ML-KEM and X25519 parts, a wrong length or a missing key check
// cannot cancel out between the client and the server half of `pq_kx.rs`.

/// An X25519 key pair made directly with ring (independent of rustls).
fn ring_x25519_pair() -> (ring::agreement::EphemeralPrivateKey, Vec<u8>) {
    let rng = ring::rand::SystemRandom::new();
    let private =
        ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::X25519, &rng).unwrap();
    let public = private.compute_public_key().unwrap().as_ref().to_vec();
    (private, public)
}

/// An X25519 agreement made directly with ring.
fn ring_x25519_agree(private: ring::agreement::EphemeralPrivateKey, peer: &[u8]) -> Vec<u8> {
    ring::agreement::agree_ephemeral(
        private,
        &ring::agreement::UnparsedPublicKey::new(&ring::agreement::X25519, peer),
        |secret| secret.to_vec(),
    )
    .unwrap()
}

/// A freshly generated, valid ML-KEM-768 encapsulation key (1184 bytes).
fn fresh_encapsulation_key_bytes() -> Vec<u8> {
    let (_decapsulation, encapsulation) = MlKem768::generate_keypair_from_rng(&mut rand::rng());
    encapsulation.to_bytes().as_slice().to_vec()
}

fn is_invalid_key_share<T>(result: Result<T, Error>) -> bool {
    matches!(
        result,
        Err(Error::PeerMisbehaved(PeerMisbehaved::InvalidKeyShare))
    )
}

#[test]
fn luecken_task_group_is_code_point_0x11ec_with_the_sizes_of_rfc_10024() {
    assert_eq!(u16::from(NamedGroup::X25519MLKEM768), 0x11ec);
    assert_eq!(X25519MLKEM768.name(), NamedGroup::X25519MLKEM768);
    assert_eq!(MLKEM768_ENCAPSULATION_KEY_LEN + X25519_LEN, 1216);
    assert_eq!(MLKEM768_CIPHERTEXT_LEN + X25519_LEN, 1120);
    assert_eq!(HYBRID_SECRET_LEN, 64);
    assert!(X25519MLKEM768.usable_for_version(ProtocolVersion::TLSv1_3));
    assert!(!X25519MLKEM768.usable_for_version(ProtocolVersion::TLSv1_2));
}

#[test]
fn luecken_task_client_secret_is_the_ml_kem_secret_then_the_x25519_secret() {
    let client = X25519MLKEM768.start().unwrap();
    let share = client.pub_key().to_vec();
    assert_eq!(share.len(), 1216);
    let (encapsulation_bytes, client_x25519) = share.split_at(1184);
    // The server half, by hand: ML-KEM encapsulation and a ring X25519 agreement.
    let encapsulation = EncapsulationKey::<MlKem768>::new_from_slice(encapsulation_bytes).unwrap();
    let (ciphertext, pq_secret) = encapsulation.encapsulate_with_rng(&mut rand::rng());
    let (server_private, server_x25519) = ring_x25519_pair();
    let x25519_secret = ring_x25519_agree(server_private, client_x25519);
    let mut server_share = Vec::new();
    server_share.extend_from_slice(ciphertext.as_slice());
    server_share.extend_from_slice(&server_x25519);
    assert_eq!(server_share.len(), 1120);
    let mut expected = Vec::new();
    expected.extend_from_slice(pq_secret.as_slice());
    expected.extend_from_slice(&x25519_secret);
    assert_eq!(expected.len(), 64);
    assert_ne!(pq_secret.as_slice(), x25519_secret.as_slice());
    let secret = client.complete(&server_share).unwrap();
    assert_eq!(secret.secret_bytes(), expected.as_slice());
}

#[test]
fn luecken_task_server_share_and_secret_follow_the_rfc_10024_order() {
    // The client half, by hand: ML-KEM key pair and a ring X25519 key pair.
    let (decapsulation, encapsulation) = MlKem768::generate_keypair_from_rng(&mut rand::rng());
    let (client_private, client_x25519) = ring_x25519_pair();
    let mut client_share = Vec::new();
    client_share.extend_from_slice(encapsulation.to_bytes().as_slice());
    client_share.extend_from_slice(&client_x25519);
    assert_eq!(client_share.len(), 1216);
    let done = X25519MLKEM768.start_and_complete(&client_share).unwrap();
    assert_eq!(done.group, NamedGroup::X25519MLKEM768);
    assert_eq!(done.pub_key.len(), 1120);
    let (ciphertext_bytes, server_x25519) = done.pub_key.split_at(1088);
    let ciphertext = ml_kem::Ciphertext::<MlKem768>::try_from(ciphertext_bytes).unwrap();
    let pq_secret = decapsulation.decapsulate(&ciphertext);
    let x25519_secret = ring_x25519_agree(client_private, server_x25519);
    let mut expected = Vec::new();
    expected.extend_from_slice(pq_secret.as_slice());
    expected.extend_from_slice(&x25519_secret);
    assert_eq!(done.secret.secret_bytes(), expected.as_slice());
}

#[test]
fn luecken_task_wrong_share_lengths_are_rejected_as_invalid_key_share() {
    let mut valid_client_share = fresh_encapsulation_key_bytes();
    valid_client_share.extend_from_slice(&ring_x25519_pair().1);
    assert_eq!(valid_client_share.len(), 1216);
    for length in [0usize, 1, 31, 32, 1183, 1184, 1215, 1217, 1248, 2400] {
        let mut share = valid_client_share.clone();
        share.resize(length, 0x55);
        assert!(
            is_invalid_key_share(X25519MLKEM768.start_and_complete(&share)),
            "client share of {length} bytes must be rejected"
        );
    }
    for length in [0usize, 1, 32, 1087, 1088, 1119, 1121, 1152, 2000] {
        let client = X25519MLKEM768.start().unwrap();
        assert!(
            is_invalid_key_share(client.complete(&vec![0x22; length])),
            "server share of {length} bytes must be rejected"
        );
    }
}

#[test]
fn luecken_task_invalid_encapsulation_keys_fail_the_fips_203_check() {
    let (_private, x25519) = ring_x25519_pair();
    // Every 12-bit value is 4095, above q = 3329.
    let mut share = vec![0xff; 1184];
    share.extend_from_slice(&x25519);
    assert!(is_invalid_key_share(
        X25519MLKEM768.start_and_complete(&share)
    ));
    // Coefficient 0 sits in byte 0 and the low nibble of byte 1.
    let with_first_coefficient = |low: u8| {
        let mut key = fresh_encapsulation_key_bytes();
        key[0] = low;
        key[1] = (key[1] & 0xf0) | 0x0d;
        key.extend_from_slice(&x25519);
        key
    };
    // 0xd01 = 3329 = q is not a canonical coefficient (FIPS 203 section 7.2).
    assert!(is_invalid_key_share(
        X25519MLKEM768.start_and_complete(&with_first_coefficient(0x01))
    ));
    // 0xd00 = 3328 = q - 1 is canonical and accepted.
    let accepted = X25519MLKEM768
        .start_and_complete(&with_first_coefficient(0x00))
        .unwrap();
    assert_eq!(accepted.pub_key.len(), 1120);
}

#[test]
fn luecken_task_all_zero_x25519_points_are_rejected_on_both_sides() {
    // Server side: valid ML-KEM key, X25519 value is the all-zero point.
    let mut client_share = fresh_encapsulation_key_bytes();
    client_share.extend_from_slice(&[0u8; 32]);
    assert!(is_invalid_key_share(
        X25519MLKEM768.start_and_complete(&client_share)
    ));
    // Client side: any 1088 bytes decapsulate (implicit rejection), the
    // all-zero X25519 value must still fail the whole exchange.
    let mut server_share = vec![0x42; 1088];
    server_share.extend_from_slice(&[0u8; 32]);
    let client = X25519MLKEM768.start().unwrap();
    assert!(is_invalid_key_share(client.complete(&server_share)));
    // The separate X25519 component offered to old peers is checked the same way.
    let client = X25519MLKEM768.start().unwrap();
    assert!(is_invalid_key_share(
        client.complete_hybrid_component(&[0u8; 32])
    ));
}

#[test]
fn luecken_task_tampered_ml_kem_ciphertext_changes_the_secret_without_panicking() {
    let client = X25519MLKEM768.start().unwrap();
    let server = X25519MLKEM768.start_and_complete(client.pub_key()).unwrap();
    let mut tampered = server.pub_key.clone();
    tampered[0] ^= 0x01;
    let secret = client.complete(&tampered).unwrap();
    assert_eq!(secret.secret_bytes().len(), 64);
    assert_ne!(secret.secret_bytes(), server.secret.secret_bytes());
}
