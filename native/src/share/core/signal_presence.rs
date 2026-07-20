use std::io;

use super::backend::ShareIrohNode;
use super::core::{eio, hmac_proof, now_secs, presence_payload, random_token};
use super::identity::ShareIdentity;
use super::types::PeerPresence;

pub(super) fn build_presence(
    kind: &str,
    relation_id: &str,
    identity: &ShareIdentity,
    secret: &[u8],
    iroh: &ShareIrohNode,
) -> io::Result<PeerPresence> {
    // Sign one coherent Iroh address snapshot so relay and direct routes can
    // never come from different network revisions.
    let routes = iroh.published_routes();
    let candidates = routes.candidates;
    let relay_url = routes.relay_url;
    let expires_at = now_secs() + 300;
    let nonce = random_token(12).map_err(eio)?;
    let payload = presence_payload(
        kind,
        relation_id,
        &identity.device_id,
        &identity.public_key,
        &identity.node_id,
        &relay_url,
        &candidates,
        expires_at,
        &nonce,
    );
    Ok(PeerPresence {
        kind: kind.to_string(),
        relation_id: relation_id.to_string(),
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
        node_id: identity.node_id.clone(),
        relay_url,
        candidates,
        expires_at,
        nonce,
        proof: hmac_proof(secret, &payload),
    })
}
