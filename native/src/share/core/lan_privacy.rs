//! Private LAN identifiers and authenticated advisory facts. A legacy
//! sighting remains a dial hint; it is never an uplink authorization.
use std::net::IpAddr;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::core::{iroh_signature, verify_iroh_signature};
use super::lan_presence_match::LanSighting;

pub(crate) const ID_EPOCH_SECS: i64 = 15 * 60;
pub(crate) const PROOF_LIFETIME_SECS: i64 = 150;
pub(crate) const MAX_ADDRESSES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanProof {
    pub epoch: i64,
    pub expires_at: i64,
    pub addresses: Vec<IpAddr>,
    pub signature: String,
}

pub(crate) fn rotating_id(node: &str, secret: &[u8], epoch: i64) -> Option<String> {
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

pub(crate) fn make_proof(
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
    };
    proof.addresses.sort();
    proof.addresses.dedup();
    proof.addresses.truncate(MAX_ADDRESSES);
    proof.signature = iroh_signature(signing_key, &payload(sighting, &proof));
    proof
}

pub(crate) fn verify_sighting(
    sighting: &LanSighting,
    proof: &LanProof,
    pinned_node: &str,
    secret: &[u8],
    now: i64,
) -> bool {
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
        || sighting.addrs.is_empty()
        || sighting
            .addrs
            .iter()
            .any(|ip| !proof.addresses.contains(ip))
        || rotating_id(pinned_node, secret, proof.epoch).as_deref() != Some(sighting.id.as_str())
    {
        return false;
    }
    verify_iroh_signature(pinned_node, &payload(sighting, proof), &proof.signature)
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
