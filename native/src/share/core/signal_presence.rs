//! Presences: building them and the device signature they carry (B03).
//!
//! Every presence keeps its v1 HMAC under the relation secret, so old peers
//! verify it unchanged. New devices also sign it with their Iroh key. The
//! signature travels inside the nonce (`"<random>.ps1.<signature>"`) because
//! servers rewrite presence objects and drop unknown fields; the HMAC covers
//! the whole nonce. The signed payload is length-prefixed (injective, S55)
//! and also covers `device_name` and `fingerprint` (S06, S26).
use std::io;

use super::backend::ShareIrohNode;
use super::core::{
    eio, hmac_proof, iroh_signature, now_secs, presence_payload, public_fingerprint, random_token,
    sha256_b64, verify_iroh_signature,
};
use super::identity::ShareIdentity;
use super::types::PeerPresence;

/// Separator of the signature inside the nonce; absent from base64url, so the
/// split is unambiguous.
const SIGNATURE_MARKER: &str = ".ps1.";
const SIGNATURE_DOMAIN: &[u8] = b"smart-explorer/share/presence-signature/v1";

/// What the device signature of a presence proves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresenceSignature {
    /// No signature (older device or forged by another code holder).
    Unsigned,
    /// Signed by the key the presence names, with matching node and
    /// fingerprint.
    Valid,
    /// Carries a signature that does not verify: always rejected.
    Invalid,
}

impl PeerPresence {
    /// Whether the nonce carries a device signature (not whether it verifies).
    pub fn is_signed(&self) -> bool {
        self.nonce.contains(SIGNATURE_MARKER)
    }

    pub(crate) fn signature_state(&self) -> PresenceSignature {
        let Some((random, signature)) = self.nonce.split_once(SIGNATURE_MARKER) else {
            return PresenceSignature::Unsigned;
        };
        let bound = !random.is_empty()
            && !signature.contains(SIGNATURE_MARKER)
            && self.node_id == self.public_key
            && self.fingerprint == public_fingerprint(self.public_key.as_bytes());
        if bound
            && verify_iroh_signature(
                &self.public_key,
                &signature_payload(self, random),
                signature,
            )
        {
            PresenceSignature::Valid
        } else {
            PresenceSignature::Invalid
        }
    }

    /// The fingerprint derived from the authenticated key replaces the wire
    /// value, which an unsigned presence does not authenticate (S26).
    pub(crate) fn with_local_fingerprint(mut self) -> Self {
        self.fingerprint = public_fingerprint(self.public_key.as_bytes());
        self
    }

    /// New legacy answers bind both unsigned envelope fields into the signed
    /// nonce. Unsigned old peers retain the pending-only compatibility path;
    /// a signed ordinary presence never authorizes an accept/reject envelope.
    pub(crate) fn matches_legacy_decision(&self, requester_device_id: &str, accepted: bool) -> bool {
        let Some((context, _)) = self.nonce.split_once(SIGNATURE_MARKER) else {
            return true;
        };
        let expected = decision_context(requester_device_id, accepted);
        context.strip_prefix(&expected).is_some_and(|random| {
            random.len() == 8
                && random.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
    }
}

fn signature_payload(presence: &PeerPresence, nonce_random: &str) -> Vec<u8> {
    let mut candidates = presence.candidates.clone();
    candidates.sort();
    let mut payload = SIGNATURE_DOMAIN.to_vec();
    for field in [
        presence.kind.as_str(),
        presence.relation_id.as_str(),
        presence.device_id.as_str(),
        presence.device_name.as_str(),
        presence.public_key.as_str(),
        presence.fingerprint.as_str(),
        presence.node_id.as_str(),
        presence.relay_url.as_str(),
    ] {
        push_field(&mut payload, field.as_bytes());
    }
    push_len(&mut payload, candidates.len());
    for candidate in &candidates {
        push_field(&mut payload, candidate.as_bytes());
    }
    payload.extend_from_slice(&presence.expires_at.to_be_bytes());
    push_field(&mut payload, nonce_random.as_bytes());
    payload
}

fn push_field(payload: &mut Vec<u8>, bytes: &[u8]) {
    push_len(payload, bytes.len());
    payload.extend_from_slice(bytes);
}

fn push_len(payload: &mut Vec<u8>, len: usize) {
    // Every field is bounded far below 4 GiB by the presence limits.
    payload.extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes());
}

pub(super) fn build_presence(
    kind: &str,
    relation_id: &str,
    identity: &ShareIdentity,
    secret: &[u8],
    iroh: &ShareIrohNode,
) -> io::Result<PeerPresence> {
    let random = random_token(12).map_err(eio)?;
    build_presence_with_nonce(kind, relation_id, identity, secret, iroh, random)
}

/// Sign a legacy decision through the existing opaque nonce field. The
/// recipient digest, decision and 48-bit random salt use 35 bytes; Ed25519's
/// marker/signature add 91, staying at 126 bytes for old server nonce limits.
pub(crate) fn build_direct_decision_presence(
    relation_id: &str,
    requester_device_id: &str,
    accepted: bool,
    identity: &ShareIdentity,
    secret: &[u8],
    iroh: &ShareIrohNode,
) -> io::Result<PeerPresence> {
    let random = format!(
        "{}{}",
        decision_context(requester_device_id, accepted),
        random_token(6).map_err(eio)?,
    );
    build_presence_with_nonce("direct", relation_id, identity, secret, iroh, random)
}

fn decision_context(requester_device_id: &str, accepted: bool) -> String {
    let payload = format!("smart-explorer/share/legacy-decision-recipient/v1:{requester_device_id}");
    let recipient = sha256_b64(payload.as_bytes());
    format!("d1{}.{}.", if accepted { 't' } else { 'f' }, &recipient[..22])
}

fn build_presence_with_nonce(
    kind: &str,
    relation_id: &str,
    identity: &ShareIdentity,
    secret: &[u8],
    iroh: &ShareIrohNode,
    random: String,
) -> io::Result<PeerPresence> {
    // Sign one coherent Iroh address snapshot so relay and direct routes can
    // never come from different network revisions.
    let routes = iroh.published_routes();
    let mut presence = PeerPresence {
        kind: kind.to_string(),
        relation_id: relation_id.to_string(),
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
        node_id: identity.node_id.clone(),
        relay_url: routes.relay_url,
        candidates: routes.candidates,
        expires_at: now_secs() + 300,
        nonce: String::new(),
        proof: String::new(),
    };
    let signature = iroh_signature(
        &identity.iroh_secret,
        &signature_payload(&presence, &random),
    );
    presence.nonce = format!("{random}{SIGNATURE_MARKER}{signature}");
    let payload = presence_payload(
        kind,
        relation_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    presence.proof = hmac_proof(secret, &payload);
    Ok(presence)
}

#[cfg(test)]
#[path = "signal_presence_task_tests.rs"]
mod task_tests;
