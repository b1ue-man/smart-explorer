//! Pure FC1 rights projection; no preference I/O in the worker snapshot.
use crate::share::{DirectContact, DirectGrantState, DirectPeerIdentity, ShareProfiles};
use serde_json::{json, Value};

pub(super) fn grants(profiles: &ShareProfiles) -> Vec<Value> {
    profiles.direct_grants.iter().map(|grant| {
        let peer = DirectPeerIdentity {
            device_id: grant.device_id.clone(), device_name: grant.device_name.clone(),
            public_key: grant.public_key.clone(), node_id: grant.node_id.clone(), fingerprint: grant.fingerprint.clone(),
        };
        let active = grant.state == DirectGrantState::Accepted && profiles.removed_direct_peer(&peer).is_none();
        json!({
            "deviceId": grant.device_id, "name": grant.device_name,
            "publicKey": grant.public_key, "nodeId": grant.node_id, "fingerprint": grant.fingerprint,
            "state": grant.state, "write": grant.write, "active": active,
            "canSetWrite": active || grant.write,
        })
    }).collect()
}

pub(super) fn contact_write(profiles: &ShareProfiles, contact: &DirectContact) -> Option<bool> {
    let peer = ShareProfiles::contact_remote_identity(contact)?;
    let mut matches = profiles.direct_grants.iter().filter(|grant| {
        peer.device_id == grant.device_id
            && peer.public_key == grant.public_key
            && peer.fingerprint == grant.fingerprint
            && (peer.node_id.is_empty() || peer.node_id == grant.node_id)
    });
    let grant = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(grant.write)
}

pub(super) fn connections(profiles: &ShareProfiles) -> Value {
    let rooms = profiles
        .rooms
        .iter()
        .map(|room| (room.id.clone(), json!(room.exports.shared_connections)))
        .collect::<serde_json::Map<_, _>>();
    json!({"direct": profiles.default_direct_exports.shared_connections, "rooms": rooms})
}
