//! Sealed LAN advisories (format `v=3`, r-entdeckung-2/-3): the address list
//! and the device signature travel encrypted under a key derived from the
//! announcer's Direct secret. A listener without that secret sees neither the
//! addresses of all interfaces nor a signature it could test against known
//! node ids; a contact opens the seal and verifies the very same signature as
//! for `v=2`.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, Key, KeyInit, Nonce,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::{LanProof, LanSighting, MAX_ADDRESSES};

/// Upper bound of one sealed advisory: 32 IPv6 addresses (17 bytes each), a
/// signature of at most 128 characters, count byte and tag fit well below.
pub const MAX_SEALED_BYTES: usize = 1024;
/// Longest signature text accepted inside a seal (same bound as `v=2`).
const MAX_SIGNATURE_CHARS: usize = 128;
const SEAL_DOMAIN: &[u8] = b"smart-explorer/lan-seal/v3\0";
pub const SEAL_NONCE_BYTES: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedLanProof {
    pub nonce: [u8; SEAL_NONCE_BYTES],
    pub ciphertext: Vec<u8>,
}

/// Binds the seal to the clear fields of the record it travels in.
fn associated_data(sighting: &LanSighting, epoch: i64, expires_at: i64) -> Vec<u8> {
    let mut aad = SEAL_DOMAIN.to_vec();
    aad.extend_from_slice(&(sighting.id.len() as u64).to_be_bytes());
    aad.extend_from_slice(sighting.id.as_bytes());
    aad.extend_from_slice(&epoch.to_be_bytes());
    aad.extend_from_slice(&expires_at.to_be_bytes());
    aad
}

fn seal_key(secret: &[u8], aad: &[u8]) -> Option<Zeroizing<[u8; 32]>> {
    if secret.len() != 32 {
        return None;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).ok()?;
    mac.update(aad);
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&mac.finalize().into_bytes());
    Some(key)
}

fn encode_plain(proof: &LanProof) -> Option<Zeroizing<Vec<u8>>> {
    if proof.addresses.is_empty()
        || proof.addresses.len() > MAX_ADDRESSES
        || proof.signature.is_empty()
        || proof.signature.len() > MAX_SIGNATURE_CHARS
    {
        return None;
    }
    let mut plain = Zeroizing::new(Vec::with_capacity(1 + proof.addresses.len() * 17 + 128));
    plain.push(u8::try_from(proof.addresses.len()).ok()?);
    for address in &proof.addresses {
        match address {
            IpAddr::V4(ip) => {
                plain.push(4);
                plain.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                plain.push(6);
                plain.extend_from_slice(&ip.octets());
            }
        }
    }
    plain.extend_from_slice(proof.signature.as_bytes());
    Some(plain)
}

fn decode_plain(plain: &[u8]) -> Option<(Vec<IpAddr>, String)> {
    let (&count, mut rest) = plain.split_first()?;
    let count = usize::from(count);
    if count == 0 || count > MAX_ADDRESSES {
        return None;
    }
    let mut addresses = Vec::with_capacity(count);
    for _ in 0..count {
        let (&tag, tail) = rest.split_first()?;
        let (address, tail) = match tag {
            4 if tail.len() >= 4 => {
                let (octets, tail) = tail.split_at(4);
                let octets: [u8; 4] = octets.try_into().ok()?;
                (IpAddr::V4(Ipv4Addr::from(octets)), tail)
            }
            6 if tail.len() >= 16 => {
                let (octets, tail) = tail.split_at(16);
                let octets: [u8; 16] = octets.try_into().ok()?;
                (IpAddr::V6(Ipv6Addr::from(octets)), tail)
            }
            _ => return None,
        };
        addresses.push(address);
        rest = tail;
    }
    if rest.is_empty() || rest.len() > MAX_SIGNATURE_CHARS {
        return None;
    }
    let signature = std::str::from_utf8(rest).ok()?.to_string();
    Some((addresses, signature))
}

/// Seals the address list and the signature of `proof` for the holders of
/// the announcer's Direct `secret`. `None` when the secret, the randomness or
/// the size is unusable; the caller then announces nothing sealed.
pub fn seal_proof(
    sighting: &LanSighting,
    proof: &LanProof,
    secret: &[u8],
) -> Option<SealedLanProof> {
    let aad = associated_data(sighting, proof.epoch, proof.expires_at);
    let key = seal_key(secret, &aad)?;
    let plain = encode_plain(proof)?;
    let mut nonce = [0u8; SEAL_NONCE_BYTES];
    getrandom::getrandom(&mut nonce).ok()?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key[..]));
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain.as_slice(),
                aad: &aad,
            },
        )
        .ok()?;
    (ciphertext.len() <= MAX_SEALED_BYTES).then_some(SealedLanProof { nonce, ciphertext })
}

/// Opens a sealed proof with the announcer's Direct `secret` into the plain
/// `v=2` proof whose signature the caller then verifies. `None` for a proof
/// without seal, a wrong secret, a changed clear field or a malformed seal.
pub fn open_proof(sighting: &LanSighting, proof: &LanProof, secret: &[u8]) -> Option<LanProof> {
    let sealed = proof.sealed.as_ref()?;
    if sealed.ciphertext.len() > MAX_SEALED_BYTES {
        return None;
    }
    let aad = associated_data(sighting, proof.epoch, proof.expires_at);
    let key = seal_key(secret, &aad)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key[..]));
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: &aad,
                },
            )
            .ok()?,
    );
    let (addresses, signature) = decode_plain(&plain)?;
    Some(LanProof {
        epoch: proof.epoch,
        expires_at: proof.expires_at,
        addresses,
        signature,
        sealed: None,
    })
}
