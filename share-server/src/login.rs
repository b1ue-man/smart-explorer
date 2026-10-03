//! Key login of a signaling connection (FC4, `key_login_v1`). The server
//! sends 16 fresh random bytes; the client signs a domain-separated SHA-256
//! of them together with the device id and public key of its Hello using its
//! Iroh key; the server verifies strictly (`PublicKey::verify`). The nonce
//! lives only for this connection, so a recorded answer is worthless.
//! Failures are reported uniformly, without saying which check failed.

use iroh_base::{PublicKey, Signature};
use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};

pub(super) const CAPABILITY: &str = "key_login_v1";
const LOGIN_DOMAIN: &[u8] = b"se-signal-login-v1\0";
pub(super) const NONCE_BYTES: usize = 16;
/// The same text for every failed login.
pub(super) const LOGIN_FAILED: &str = "key login failed";

pub(super) fn nonce() -> Option<[u8; NONCE_BYTES]> {
    let mut nonce = [0u8; NONCE_BYTES];
    SystemRandom::new().fill(&mut nonce).ok()?;
    Some(nonce)
}

/// SHA-256 over the domain and the length-prefixed nonce, device id and
/// public key; the app computes the same (`signal_handshake::login_digest`).
pub(super) fn digest(nonce: &[u8], device_id: &str, public_key: &str) -> [u8; 32] {
    let mut context = digest::Context::new(&digest::SHA256);
    context.update(LOGIN_DOMAIN);
    for field in [nonce, device_id.as_bytes(), public_key.as_bytes()] {
        let length = u32::try_from(field.len()).unwrap_or(u32::MAX);
        context.update(&length.to_be_bytes());
        context.update(field);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(context.finish().as_ref());
    out
}

/// The key the client proved, or `None`.
pub(super) fn verify(
    nonce: &[u8],
    device_id: &str,
    public_key: &str,
    signature_hex: &str,
) -> Option<PublicKey> {
    if signature_hex.len() != Signature::LENGTH * 2 { return None; }
    let key = parse_key(public_key)?;
    let bytes: [u8; Signature::LENGTH] = hex_decode(signature_hex)?.try_into().ok()?;
    let signature = Signature::from_bytes(&bytes);
    key.verify(&digest(nonce, device_id, public_key), &signature)
        .ok()?;
    Some(key)
}

/// An Iroh public key as the app writes it (64 hex digits).
pub(super) fn parse_key(public_key: &str) -> Option<PublicKey> {
    public_key.parse::<PublicKey>().ok()
}

pub(super) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(digest::digest(&digest::SHA256, bytes).as_ref());
    out
}

pub(super) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

pub(super) fn hex_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.as_bytes();
    if text.len() % 2 != 0 {
        return None;
    }
    text.chunks(2)
        .map(|pair| Some((hex_value(pair[0])? << 4) | hex_value(pair[1])?))
        .collect()
}

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{digest, hex, hex_decode, nonce, verify};

    /// Shared test vector with the app (`signal_handshake.rs`).
    const LOGIN_DIGEST_VECTOR: &str =
        "7cfb3e8edf96d9906d5c895e201146e639b1a74b2ab473401db0614c85e09bac";

    #[test]
    fn review_task_login_digest_matches_the_app_vector() {
        let nonce: Vec<u8> = (0u8..16).collect();
        assert_eq!(hex(&digest(&nonce, "device-a", "pk-a")), LOGIN_DIGEST_VECTOR);
    }

    #[test]
    fn review_task_only_the_signing_key_passes_the_login() {
        let secret = iroh_base::SecretKey::from_bytes(&[3; 32]);
        let public_key = secret.public().to_string();
        let nonce = nonce().expect("random nonce");
        let signature = hex(&secret
            .sign(&digest(&nonce, "device-a", &public_key))
            .to_bytes());
        assert_eq!(
            verify(&nonce, "device-a", &public_key, &signature),
            Some(secret.public())
        );
        // Another device id, nonce or key: refused.
        assert_eq!(verify(&nonce, "device-b", &public_key, &signature), None);
        assert_eq!(verify(&[0; 16], "device-a", &public_key, &signature), None);
        let other = iroh_base::SecretKey::from_bytes(&[4; 32]).public().to_string();
        assert_eq!(verify(&nonce, "device-a", &other, &signature), None);
        assert_eq!(verify(&nonce, "device-a", &public_key, "zz"), None);
        assert_eq!(hex_decode("0aFF"), Some(vec![0x0a, 0xff]));
        assert_eq!(hex_decode("abc"), None);
    }
}
