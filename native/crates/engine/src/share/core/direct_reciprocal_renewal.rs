//! E17 glue of the reciprocal Direct stream: which data an exchange carries
//! while this device's Direct code is renewed, and what a completed exchange
//! confirms. The message „Beziehungsdaten erneuert“ is the offer to a
//! kind-5 hello (`direct_reciprocal_wire.rs`); its confirmation is the
//! initiator's commit, sent only after the new data are durable.
use crate::share::direct_reciprocal::DirectRelationMaterial;
use crate::share::direct_reciprocal_session::DirectRepairReceiver;
use crate::share::identity::code_renewal_registry as registry;
use crate::share::identity_store::code_renewal_store;
use crate::share::session::IncomingSession;
use crate::share::types::PeerPresence;

/// Initiator: the data this device sends to `peer`. A peer that still waits
/// for this device's renewal gets exactly the retiring data it holds (a push
/// never carries anything new); everyone else gets the current data.
/// Returns whether the current data are sent.
pub(super) fn outgoing_material(
    local_node: &str,
    peer: &PeerPresence,
    current: DirectRelationMaterial,
) -> (DirectRelationMaterial, bool) {
    let pending = registry::pending_material_for(
        local_node,
        &peer.device_id,
        &peer.public_key,
        &peer.node_id,
    );
    match pending.and_then(|(lookup_id, secret)| {
        DirectRelationMaterial::new(lookup_id, secret.to_vec()).ok()
    }) {
        Some(retiring) => (retiring, false),
        None => (current, true),
    }
}

/// Initiator: the host announced a renewal in its signed presence, so the
/// hello asks for renewed data (kind 5).
pub(super) fn asks_for_renewal(peer: &PeerPresence) -> bool {
    peer.announces_code_renewal()
}

/// Receiver: a session on a retiring lookup id keeps its data unless the
/// hello asks for renewed ones.
pub(super) fn with_retiring(
    receiver: DirectRepairReceiver,
    local_node: &str,
    session: &IncomingSession,
) -> DirectRepairReceiver {
    let Some(lookup_id) = session.direct_relation_id() else {
        return receiver;
    };
    let Some(secret) = registry::retiring_secret(local_node, lookup_id) else {
        return receiver;
    };
    match DirectRelationMaterial::new(lookup_id, secret.to_vec()) {
        Ok(material) => receiver.with_retiring_material(material),
        Err(_) => receiver,
    }
}

/// After a completed exchange: the peer now holds `current_lookup_id`
/// (confirmation) or kept the retiring `used_lookup_id` without renewal
/// support (older peer). Runs the durable bookkeeping off the async task;
/// a no-op unless a renewal waits for this peer.
pub(super) fn record_outcome(
    local_node: &str,
    peer: (&str, &str, &str),
    holds_current: bool,
    current_lookup_id: &str,
    used_lookup_id: &str,
) {
    let (device_id, public_key, peer_node) = peer;
    if !registry::is_pending(local_node, device_id, public_key) {
        return;
    }
    let local_node = local_node.to_string();
    let device_id = device_id.to_string();
    let public_key = public_key.to_string();
    let peer_node = peer_node.to_string();
    let current = current_lookup_id.to_string();
    let used = used_lookup_id.to_string();
    // Detached: the exchange is complete either way; a failure shows in
    // the renewal status and the next exchange confirms again.
    drop(tokio::task::spawn_blocking(move || {
        if holds_current {
            code_renewal_store::confirm_peer(
                &local_node,
                &device_id,
                &public_key,
                &peer_node,
                &current,
            );
        } else {
            code_renewal_store::mark_legacy(
                &local_node,
                &device_id,
                &public_key,
                &peer_node,
                &used,
            );
        }
    }));
}
