//! The remote keys the Share endpoint refuses in the TLS handshake (M6,
//! decision E32; plan 2026-11-08).
//!
//! Iroh registers every connection whose handshake completed before the
//! accept loop sees it, and the registration sends this device's direct
//! addresses to the peer (`ADD_ADDRESS`) and answers its `REACH_OUT` with
//! probes from this device's sockets (`vendor/SMART_EXPLORER_ROUTE_POLICY.md`,
//! section "M6"). The accept loop already refuses a removed device that
//! arrives over the relay before the handshake; one that dials an address it
//! still knows would get through the handshake. The vendor gate
//! (`iroh::endpoint::RemoteKeyGate`) closes that: the handshake of a refused
//! key fails, in both directions, before any own address leaves.
//!
//! Refused are exactly the identities the accept loop refuses early
//! (`node_accept.rs`, `!known && is_withdrawn`): the user removed them (the
//! withdrawn node ids of the Direct-code renewal,
//! `code_renewal_registry::is_withdrawn`) and no relation admits them again
//! (an accepted grant or contact, an admitted member of an auto-join room), so
//! a removed Direct contact that is still a room member keeps its room.
//! Unknown and denied devices still complete the handshake: the host checks
//! their hello and every surface names them with the way to allow them
//! (`peer_denials.rs`). The gate reads the registry and the relations on
//! every handshake, so a removal applies to the next handshake without
//! restarting the endpoint.
use std::sync::{Arc, Mutex, TryLockError};

use iroh::endpoint::RemoteKeyGate;

use crate::share::identity::code_renewal_registry as registry;
use crate::share::types::{DirectAccessState, DirectGrantState, ShareAuthState};

/// The gate of the Share endpoint whose relations live in `auth`.
pub(in crate::share) fn remote_key_gate(auth: Arc<Mutex<ShareAuthState>>) -> RemoteKeyGate {
    RemoteKeyGate::new(move |remote| refuses(&auth, &remote.to_string()))
}

/// Whether the handshake with the endpoint `node_id` (its text form) fails.
pub(in crate::share) fn refuses(auth: &Mutex<ShareAuthState>, node_id: &str) -> bool {
    if !registry::is_withdrawn(node_id) {
        return false;
    }
    // The verifier runs on the connection driver. A thread that holds the
    // state while it waits for its own connect would wait for this very
    // handshake, so the gate never waits for the lock: a busy or poisoned
    // state refuses the removed key, as the gate did before this check.
    match auth.try_lock() {
        Ok(auth) => !known(&auth, node_id),
        Err(TryLockError::WouldBlock) | Err(TryLockError::Poisoned(_)) => true,
    }
}

/// Whether a relation in `auth` admits the endpoint `remote`. The same rule
/// as `ShareIrohNode::known_endpoint` (`node_accept.rs`), which keeps a
/// removed key that is known again out of the early refusal.
pub(in crate::share) fn known(auth: &ShareAuthState, remote: &str) -> bool {
    let matches = |key: &str, node: &str| {
        !remote.is_empty()
            && ((!node.is_empty() && node == remote) || (node.is_empty() && key == remote))
    };
    auth.direct_grants.iter().any(|grant| {
        grant.state == DirectGrantState::Accepted && matches(&grant.public_key, &grant.node_id)
    }) || auth.direct_contacts.iter().any(|contact| {
        contact.access_state == DirectAccessState::Accepted
            && matches(
                contact
                    .remote_public_key
                    .as_deref()
                    .or(contact.accepted_public_key.as_deref())
                    .unwrap_or_default(),
                &contact.expected_node_id,
            )
    }) || auth.rooms.iter().filter(|room| room.auto_join).any(|room| {
        room.members
            .iter()
            .any(|member| member.is_admitted() && matches(&member.public_key, &member.node_id))
    })
}
