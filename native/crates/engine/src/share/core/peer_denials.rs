//! Rejected sessions of this process, in both directions (R3, A9; plan
//! 2026-11-06).
//!
//! Outgoing: a host rejected this device's hello. Further operations on the
//! same session key answer with the same rejection until a growing pause
//! has passed (first rejection: none, then 5 s, 15 s, 60 s, 120 s), instead
//! of opening a new QUIC connection with a full handshake per operation.
//! The pause ends early on a success, a configuration change of this device
//! and a fresh presence of that host. The host still checks every hello it
//! receives; this only spares hellos that would be rejected the same way.
//!
//! Incoming: devices this host rejected, so every surface can name them
//! with the reason and the way to allow them. Only the peer's TLS-bound
//! identity and the class are kept, never proof material.
//!
//! Both halves are bounded (`MAX_ENTRIES` each, oldest dropped first).
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::super::types::ShareAuthState;
use super::super::wire::{FsErrorKind, FsResponse, PeerHello};
use super::denial::{denial_of, peer_denied, SessionDenial};

const MAX_ENTRIES: usize = 64;
/// Pause after the n-th consecutive rejection (index n - 1, last repeats).
const PAUSES: [Duration; 5] = [
    Duration::ZERO,
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(60),
    Duration::from_secs(120),
];
/// A rejection older than this starts the count anew.
const FORGET_AFTER: Duration = Duration::from_secs(15 * 60);
/// Incoming rejections older than this are no longer shown.
const SHOW_INCOMING_SECS: i64 = 24 * 60 * 60;

struct Outgoing {
    local_node: String,
    node_id: String,
    device_id: String,
    device_name: String,
    denial: Option<SessionDenial>,
    message: String,
    failures: usize,
    last: Instant,
    until: Instant,
    at_unix: i64,
}

#[derive(Clone)]
struct Incoming {
    device_id: String,
    public_key: String,
    node_id: String,
    relation_kind: String,
    denial: SessionDenial,
    count: u64,
    first_unix: i64,
    last_unix: i64,
}

#[derive(Default)]
struct Book {
    outgoing: HashMap<String, Outgoing>,
    incoming: Vec<Incoming>,
}

fn book() -> std::sync::MutexGuard<'static, Book> {
    static BOOK: OnceLock<Mutex<Book>> = OnceLock::new();
    BOOK.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_unix() -> i64 {
    super::super::core::now_secs()
}

/// The host side of one outgoing session key.
pub(in crate::share) struct OutgoingPeer<'a> {
    /// This device's node id: several nodes of one process (tests) keep
    /// apart.
    pub(in crate::share) local_node: &'a str,
    pub(in crate::share) key: &'a str,
    pub(in crate::share) node_id: &'a str,
    pub(in crate::share) device_id: &'a str,
    pub(in crate::share) device_name: &'a str,
}

fn book_key(local_node: &str, key: &str) -> String {
    format!("{local_node}|{key}")
}

/// The remembered rejection of `key`, while its pause lasts.
pub(in crate::share) fn outgoing_paused(local_node: &str, key: &str) -> Option<io::Error> {
    let book = book();
    let entry = book.outgoing.get(&book_key(local_node, key))?;
    (Instant::now() < entry.until).then(|| peer_denied(entry.denial, entry.message.clone()))
}

/// Records a rejected hello of `peer`.
pub(in crate::share) fn record_outgoing(
    peer: OutgoingPeer<'_>,
    denial: Option<SessionDenial>,
    message: &str,
) {
    let now = Instant::now();
    let key = book_key(peer.local_node, peer.key);
    let mut book = book();
    let failures = book
        .outgoing
        .get(&key)
        .filter(|entry| now.saturating_duration_since(entry.last) < FORGET_AFTER)
        .map_or(0, |entry| entry.failures)
        .saturating_add(1);
    let pause = PAUSES[failures.min(PAUSES.len()) - 1];
    if !book.outgoing.contains_key(&key) && book.outgoing.len() >= MAX_ENTRIES {
        let oldest = book
            .outgoing
            .iter()
            .min_by_key(|(_, entry)| entry.last)
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            book.outgoing.remove(&oldest);
        }
    }
    book.outgoing.insert(
        key,
        Outgoing {
            local_node: peer.local_node.to_string(),
            node_id: peer.node_id.to_string(),
            device_id: peer.device_id.to_string(),
            device_name: peer.device_name.to_string(),
            denial,
            message: message.to_string(),
            failures,
            last: now,
            until: now + pause,
            at_unix: now_unix(),
        },
    );
}

/// Runs one outgoing hello of `key` to `presence`'s host unless a rejection
/// of it still pauses, and records its outcome.
pub(in crate::share) fn guarded<T>(
    local_node: &str,
    key: &str,
    presence: &super::super::types::PeerPresence,
    connect: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    if let Some(rejected) = outgoing_paused(local_node, key) {
        return Err(rejected);
    }
    let result = connect();
    match &result {
        Ok(_) => clear_outgoing(local_node, key),
        Err(error) => {
            if let Some(denial) = super::denial::peer_denial_of(error) {
                record_outgoing(
                    OutgoingPeer {
                        local_node,
                        key,
                        node_id: &presence.node_id,
                        device_id: &presence.device_id,
                        device_name: &presence.device_name,
                    },
                    denial,
                    &error.to_string(),
                );
            }
        }
    }
    result
}

/// A session of `key` was accepted.
pub(in crate::share) fn clear_outgoing(local_node: &str, key: &str) {
    book().outgoing.remove(&book_key(local_node, key));
}

/// A fresh presence of the host `node_id`: its next hello is tried at once.
pub(in crate::share) fn forget_outgoing_node(local_node: &str, node_id: &str) {
    let mut book = book();
    for entry in book.outgoing.values_mut() {
        if entry.local_node == local_node && entry.node_id == node_id {
            entry.until = entry.last;
        }
    }
}

/// This device's configuration changed (contacts, codes, rooms): every
/// paused key is tried at once again; the rejection stays visible until then.
pub(in crate::share) fn forget_outgoing_pauses(local_node: &str) {
    let mut book = book();
    for entry in book.outgoing.values_mut() {
        if entry.local_node == local_node {
            entry.until = entry.last;
        }
    }
}

/// Records a hello this host rejected.
pub(in crate::share) fn record_incoming(
    device_id: &str,
    public_key: &str,
    node_id: &str,
    relation_kind: &str,
    denial: SessionDenial,
) {
    let now = now_unix();
    let (device_id, public_key, node_id, relation_kind) = (
        truncated(device_id),
        truncated(public_key),
        truncated(node_id),
        truncated(relation_kind),
    );
    let mut book = book();
    if let Some(entry) = book.incoming.iter_mut().find(|entry| {
        entry.device_id == device_id
            && entry.public_key == public_key
            && entry.relation_kind == relation_kind
    }) {
        entry.node_id = node_id;
        entry.denial = denial;
        entry.count = entry.count.saturating_add(1);
        entry.last_unix = now;
        return;
    }
    if book.incoming.len() >= MAX_ENTRIES {
        let oldest = book
            .incoming
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.last_unix)
            .map(|(index, _)| index);
        if let Some(oldest) = oldest {
            book.incoming.remove(oldest);
        }
    }
    book.incoming.push(Incoming {
        device_id,
        public_key,
        node_id,
        relation_kind,
        denial,
        count: 1,
        first_unix: now,
        last_unix: now,
    });
}

/// The hello rejection of `error` (R3): with a class, the reason and the way
/// to the grant under this host's name, and the rejection recorded for this
/// host's surfaces; without one (a node id that does not match the TLS
/// identity, a locked state) the former generic text. The second value is
/// the host's log line, now naming the device.
pub(in crate::share) fn host_rejection(
    hello: &PeerHello,
    auth: &Arc<Mutex<ShareAuthState>>,
    error: &io::Error,
) -> (FsResponse, io::Error) {
    let denial = denial_of(error);
    let (host_name, device) = match auth.lock() {
        Ok(state) => (
            state.identity.device_name.clone(),
            known_name(&state, &hello.device_id),
        ),
        Err(_) => (String::new(), None),
    };
    if let Some(denial) = denial {
        record_incoming(
            &hello.device_id,
            &hello.public_key,
            &hello.node_id,
            &hello.relation_kind,
            denial,
        );
    }
    let msg = denial.map_or_else(
        || "Share-Anmeldung nicht autorisiert".to_string(),
        |denial| denial.message(&host_name),
    );
    let device = device.unwrap_or_else(|| truncated(&hello.device_id));
    (
        FsResponse::Err {
            kind: Some(FsErrorKind::PermissionDenied),
            msg,
            denial,
        },
        io::Error::new(error.kind(), format!("{error} (Gerät {device})")),
    )
}

/// The name this host knows a device by (its contact or grant).
fn known_name(state: &ShareAuthState, device_id: &str) -> Option<String> {
    state
        .direct_contacts
        .iter()
        .find(|contact| contact.remote_device_id.as_deref() == Some(device_id))
        .map(|contact| contact.display_name.clone())
        .or_else(|| {
            state
                .direct_grants
                .iter()
                .find(|grant| grant.device_id == device_id)
                .map(|grant| grant.device_name.clone())
        })
        .filter(|name| !name.trim().is_empty())
        .map(|name| truncated(&name))
}

/// The host now accepted a session of this identity.
pub(in crate::share) fn clear_incoming(device_id: &str, public_key: &str) {
    book()
        .incoming
        .retain(|entry| entry.device_id != device_id || entry.public_key != public_key);
}

fn truncated(value: &str) -> String {
    let mut end = value.len().min(256);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

/// One remembered rejection, as every surface shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::share) struct DenialRow {
    /// `outgoing`: the other device rejected this one; `incoming`: this
    /// device rejected the other one.
    pub(in crate::share) direction: &'static str,
    pub(in crate::share) device_id: String,
    pub(in crate::share) public_key: Option<String>,
    pub(in crate::share) node_id: String,
    pub(in crate::share) device_name: Option<String>,
    pub(in crate::share) relation_kind: Option<String>,
    pub(in crate::share) denial: Option<SessionDenial>,
    pub(in crate::share) message: Option<String>,
    pub(in crate::share) count: u64,
    pub(in crate::share) last_unix: i64,
    pub(in crate::share) first_unix: i64,
    /// Seconds until this device tries the host again (outgoing only).
    pub(in crate::share) retry_in_secs: Option<u64>,
}

/// Every remembered rejection of the node `local_node` as client and of
/// this process as host; incoming ones of the last 24 hours.
pub(in crate::share) fn rows(local_node: &str) -> Vec<DenialRow> {
    let now = Instant::now();
    let unix = now_unix();
    let book = book();
    let mut rows: Vec<DenialRow> = book
        .outgoing
        .values()
        .filter(|entry| entry.local_node == local_node)
        .map(|entry| DenialRow {
            direction: "outgoing",
            device_id: entry.device_id.clone(),
            public_key: None,
            node_id: entry.node_id.clone(),
            device_name: Some(entry.device_name.clone()).filter(|name| !name.is_empty()),
            relation_kind: None,
            denial: entry.denial,
            message: Some(entry.message.clone()),
            count: entry.failures as u64,
            last_unix: entry.at_unix,
            first_unix: entry.at_unix,
            retry_in_secs: Some(entry.until.saturating_duration_since(now).as_secs()),
        })
        .collect();
    rows.extend(
        book.incoming
            .iter()
            .filter(|entry| unix.saturating_sub(entry.last_unix) <= SHOW_INCOMING_SECS)
            .map(|entry| DenialRow {
                direction: "incoming",
                device_id: entry.device_id.clone(),
                public_key: Some(entry.public_key.clone()),
                node_id: entry.node_id.clone(),
                device_name: None,
                relation_kind: Some(entry.relation_kind.clone()),
                denial: Some(entry.denial),
                message: None,
                count: entry.count,
                last_unix: entry.last_unix,
                first_unix: entry.first_unix,
                retry_in_secs: None,
            }),
    );
    rows.sort_by(|left, right| right.last_unix.cmp(&left.last_unix));
    rows
}

#[cfg(test)]
#[path = "peer_denials_tests.rs"]
mod tests;
