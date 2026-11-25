//! Peer addresses at connection setup and for direct retries (user report
//! 2026-11-25: „das aktuallisieren der ip adresse muss automatisch laufen
//! beim verbindungsaufbau“).
//!
//! Every outgoing connection of the node (files, exec, screen, TCP tunnel,
//! device commands) dials through [`ShareIrohNode::dial_addr`]: once per
//! peer and signal session it lets the signal worker fetch the peer's
//! current presence from the Share server (`peer_address_book.rs`, at most
//! `REFRESH_WAIT`), then it dials the newest verified presence of the same
//! relation and device. The cached presence keeps its LAN sightings: Iroh
//! sends the handshake to every address of a dial at once, so a stale extra
//! address costs one unanswered packet and never delays a fresh one.
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use iroh::EndpointAddr;

use crate::share::lan_presence_match::effective_presence;
use crate::share::node::ShareIrohNode;
use crate::share::peer_address_book::{PeerAddressBook, REFRESH_WAIT};
use crate::share::route_retry_port::{DirectRetryPort, NodeDirectRetry};
use crate::share::session::endpoint_addr;
use crate::share::signal_presence::PresenceSignature;
use crate::share::types::{DirectAccessState, PeerEndpoint, PeerPresence, ShareAuthState};

/// Published peer addresses one direct retry uses.
const MAX_PEER_ADDRS: usize = 16;

impl ShareIrohNode {
    /// The address to dial for `endpoint`, with the peer's addresses as
    /// published now (see the module documentation).
    pub(in crate::share) async fn dial_addr(
        &self,
        endpoint: &PeerEndpoint,
    ) -> io::Result<EndpointAddr> {
        let book = &self.peer_addresses;
        if let Some(round) = book.request(&endpoint.presence.node_id) {
            book.wait_round(round, REFRESH_WAIT).await;
        }
        let fresh = book.fresh(&endpoint.presence);
        let presence = merged_dial_presence(&endpoint.presence, fresh.as_ref());
        let local = self.routes.current(&self.endpoint);
        endpoint_addr(&presence, &local, &self.transport_options)
    }

    /// The port through which the route actors of this node's connections
    /// keep trying the direct path (`route_monitor_retry.rs`).
    pub(in crate::share) fn direct_retry_port(&self) -> Arc<dyn DirectRetryPort> {
        Arc::new(NodeDirectRetry {
            endpoint: self.endpoint.clone(),
            auth: self.auth.clone(),
            book: self.peer_addresses.clone(),
        })
    }
}

/// The presence to dial: `cached` (the relation's current routing evidence,
/// LAN sightings included) replaced by `fresh`, a verified server presence of
/// the same relation and device that is newer or the only server evidence;
/// cached addresses missing from it are kept behind the fresh ones.
pub(in crate::share) fn merged_dial_presence(
    cached: &PeerPresence,
    fresh: Option<&PeerPresence>,
) -> PeerPresence {
    let Some(fresh) = fresh.filter(|fresh| {
        same_device(cached, fresh)
            && fresh.nonce != cached.nonce
            && (cached.relay_url.is_empty() || fresh.expires_at >= cached.expires_at)
    }) else {
        return cached.clone();
    };
    let mut merged = fresh.clone();
    for candidate in &cached.candidates {
        if !merged.candidates.contains(candidate) {
            merged.candidates.push(candidate.clone());
        }
    }
    if merged.relay_url.is_empty() {
        merged.relay_url = cached.relay_url.clone();
    }
    merged
}

fn same_device(left: &PeerPresence, right: &PeerPresence) -> bool {
    left.kind == right.kind
        && left.relation_id == right.relation_id
        && left.device_id == right.device_id
        && left.public_key == right.public_key
        && left.node_id == right.node_id
}

/// The direct addresses `remote` (an Iroh node id) published last: of each
/// accepted Direct contact with that node its routing evidence updated by
/// the newest verified server presence, and of each room member with that
/// node a device-signed presence. Sorted, without duplicates, at most 16.
pub(in crate::share) fn peer_addresses(
    state: &ShareAuthState,
    book: &PeerAddressBook,
    remote: &str,
    now: i64,
) -> Vec<SocketAddr> {
    let mut presences = Vec::new();
    for contact in state.direct_contacts.iter().filter(|contact| {
        contact.access_state == DirectAccessState::Accepted
            && (contact.expected_node_id == remote
                || contact.remote_public_key.as_deref() == Some(remote)
                || contact
                    .presence
                    .as_ref()
                    .is_some_and(|presence| presence.node_id == remote))
    }) {
        let fresh = book.fresh_for("direct", &contact.lookup_id, remote);
        match effective_presence(contact, now) {
            Some(cached) => presences.push(merged_dial_presence(&cached, fresh.as_ref())),
            None => presences.extend(fresh),
        }
    }
    let members = state
        .rooms
        .iter()
        .filter(|room| room.auto_join)
        .flat_map(|room| &room.members)
        .filter(|member| !member.blocked);
    for presence in members.filter_map(|member| member.presence.as_ref()) {
        if presence.node_id == remote
            && presence.is_current_at(now)
            && presence.signature_state() == PresenceSignature::Valid
        {
            presences.push(merged_dial_presence(
                presence,
                book.fresh(presence).as_ref(),
            ));
        }
    }
    let mut addrs: Vec<SocketAddr> = presences
        .iter()
        .flat_map(|presence| &presence.candidates)
        .filter_map(|candidate| crate::net::parse_candidate(candidate))
        .collect();
    addrs.sort();
    addrs.dedup();
    addrs.truncate(MAX_PEER_ADDRS);
    addrs
}

#[cfg(test)]
#[path = "peer_dial_tests.rs"]
mod tests;
