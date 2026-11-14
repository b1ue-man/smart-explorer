//! Private LAN identifiers and authenticated advisory facts. A legacy
//! sighting remains a dial hint; it is never an uplink authorization.
use std::net::IpAddr;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::core::{iroh_signature, verify_iroh_signature};
use super::lan_presence_match::LanSighting;

#[path = "lan_privacy_seal.rs"]
mod seal;
pub use seal::{open_proof, seal_proof, SealedLanProof, MAX_SEALED_BYTES, SEAL_NONCE_BYTES};

pub const ID_EPOCH_SECS: i64 = 15 * 60;
pub const PROOF_LIFETIME_SECS: i64 = 150;
pub const MAX_ADDRESSES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanProof {
    pub epoch: i64,
    pub expires_at: i64,
    pub addresses: Vec<IpAddr>,
    pub signature: String,
    /// `v=3`: addresses and signature sealed under the announcer's Direct
    /// secret. A received sealed proof carries empty `addresses` and
    /// `signature` until `open_proof` restores them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<SealedLanProof>,
}

pub fn rotating_id(node: &str, secret: &[u8], epoch: i64) -> Option<String> {
    if secret.len() != 32 || node.is_empty() || epoch < 0 {
        return None;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).ok()?;
    mac.update(b"smart-explorer/lan-private-id/v2\0");
    mac.update(&(node.len() as u64).to_be_bytes());
    mac.update(node.as_bytes());
    mac.update(&epoch.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    Some(
        digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

pub fn make_proof(
    sighting: &LanSighting,
    epoch: i64,
    expires_at: i64,
    signing_key: &iroh::SecretKey,
) -> LanProof {
    let mut proof = LanProof {
        epoch,
        expires_at,
        addresses: sighting.addrs.clone(),
        signature: String::new(),
        sealed: None,
    };
    proof.addresses.sort();
    proof.addresses.dedup();
    proof.addresses.truncate(MAX_ADDRESSES);
    proof.signature = iroh_signature(signing_key, &payload(sighting, &proof));
    proof
}

/// The sighting cut to the addresses its signed list names (E26). Addresses
/// outside the signed list are dropped and the rest stay; nothing is added,
/// because the signature covers only the signed list. `None` when the proof,
/// the lookup id, the epoch, the lifetime or the signature fail, or when no
/// signed address is left.
pub fn accepted_sighting(
    sighting: &LanSighting,
    proof: &LanProof,
    pinned_node: &str,
    secret: &[u8],
    now: i64,
) -> Option<LanSighting> {
    let epoch = now.div_euclid(ID_EPOCH_SECS);
    if proof.epoch < epoch.saturating_sub(1)
        || proof.epoch > epoch.saturating_add(1)
        || proof.expires_at < now
        || proof.expires_at > now.saturating_add(PROOF_LIFETIME_SECS + 30)
        || proof.addresses.is_empty()
        || proof.addresses.len() > MAX_ADDRESSES
        || proof
            .addresses
            .iter()
            .any(|ip| ip.is_loopback() || ip.is_unspecified() || ip.is_multicast())
        || rotating_id(pinned_node, secret, proof.epoch).as_deref() != Some(sighting.id.as_str())
    {
        return None;
    }
    let addrs: Vec<IpAddr> = sighting
        .addrs
        .iter()
        .filter(|ip| proof.addresses.contains(ip))
        .copied()
        .collect();
    if addrs.is_empty()
        || !verify_iroh_signature(pinned_node, &payload(sighting, proof), &proof.signature)
    {
        return None;
    }
    Some(LanSighting {
        addrs,
        ..sighting.clone()
    })
}

/// The sighting of a received proof of either format (E26): a sealed (`v=3`)
/// proof is opened with `secret` first, then [`accepted_sighting`] applies.
pub fn accepted_any_sighting(
    sighting: &LanSighting,
    proof: &LanProof,
    pinned_node: &str,
    secret: &[u8],
    now: i64,
) -> Option<LanSighting> {
    if proof.sealed.is_none() {
        return accepted_sighting(sighting, proof, pinned_node, secret, now);
    }
    let opened = open_proof(sighting, proof, secret)?;
    accepted_sighting(sighting, &opened, pinned_node, secret, now)
}

/// `true` when [`accepted_sighting`] accepts the sighting. Addresses outside the
/// signed list are dropped, neither refused as a whole nor adopted (E26).
pub fn verify_sighting(
    sighting: &LanSighting,
    proof: &LanProof,
    pinned_node: &str,
    secret: &[u8],
    now: i64,
) -> bool {
    accepted_sighting(sighting, proof, pinned_node, secret, now).is_some()
}

/// Verifies a received proof of either format: a sealed (`v=3`) proof is
/// opened with `secret` first, then the same signature check applies.
pub fn verify_any_sighting(
    sighting: &LanSighting,
    proof: &LanProof,
    pinned_node: &str,
    secret: &[u8],
    now: i64,
) -> bool {
    accepted_any_sighting(sighting, proof, pinned_node, secret, now).is_some()
}

fn payload(sighting: &LanSighting, proof: &LanProof) -> Vec<u8> {
    let mut bytes = b"smart-explorer/lan-advisory/v2\0".to_vec();
    bytes.extend_from_slice(&(sighting.id.len() as u64).to_be_bytes());
    bytes.extend_from_slice(sighting.id.as_bytes());
    bytes.extend_from_slice(&proof.epoch.to_be_bytes());
    bytes.extend_from_slice(&proof.expires_at.to_be_bytes());
    bytes.extend_from_slice(&sighting.p4.to_be_bytes());
    bytes.extend_from_slice(&sighting.p6.to_be_bytes());
    bytes.push(u8::from(sighting.uplink));
    bytes.extend_from_slice(&(proof.addresses.len() as u64).to_be_bytes());
    for address in &proof.addresses {
        match address {
            IpAddr::V4(ip) => {
                bytes.push(4);
                bytes.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                bytes.push(6);
                bytes.extend_from_slice(&ip.octets());
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_lan_ids_rotate_and_forged_advisories_are_not_uplink_facts() {
        let key = iroh::SecretKey::from_bytes(&[7; 32]);
        let node = key.public().to_string();
        let secret = [9; 32];
        let now = 1800;
        let epoch = now / ID_EPOCH_SECS;
        let mut sighting = LanSighting {
            id: rotating_id(&node, &secret, epoch).unwrap(),
            addrs: vec!["192.168.4.2".parse().unwrap()],
            p4: 51820,
            p6: 0,
            uplink: false,
            seen_at: now,
        };
        assert_ne!(sighting.id, rotating_id(&node, &secret, epoch + 1).unwrap());
        assert_ne!(sighting.id, rotating_id(&node, &[8; 32], epoch).unwrap());
        let proof = make_proof(&sighting, epoch, now + PROOF_LIFETIME_SECS, &key);
        assert!(verify_sighting(&sighting, &proof, &node, &secret, now));
        sighting.uplink = true;
        assert!(!verify_sighting(&sighting, &proof, &node, &secret, now));
        sighting.uplink = false;
        sighting.addrs = vec!["192.168.5.2".parse().unwrap()];
        assert!(!verify_sighting(&sighting, &proof, &node, &secret, now));
        sighting.addrs = proof.addresses.clone();
        assert!(!verify_sighting(
            &sighting,
            &proof,
            &node,
            &secret,
            now + PROOF_LIFETIME_SECS + 1
        ));
    }
}

#[cfg(test)]
#[path = "lan_privacy_task_tests.rs"]
mod task_tests;

#[cfg(test)]
#[path = "lan_kuerzung_task_tests.rs"]
mod kuerzung_task_tests;
