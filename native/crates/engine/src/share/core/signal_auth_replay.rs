//! Bounded replay admission and immediate in-memory signature memory.
use std::collections::HashSet;

use crate::share::types::{DirectContact, PeerPresence, RoomProfile, ShareAuthState};

/// Reject fresh messages at capacity: replay keys carry their expiry and are
/// only dropped once it passed.
const MAX_REPLAY_KEYS: usize = 4096;

/// Signature markers have their own bound and never count against the replay
/// keys (r-entdeckung-1). At capacity the least recently verified marker of a
/// key that no contact and no room member pins is evicted first; a pinned key
/// keeps its downgrade protection durably in `relation.signed_presence`.
const MAX_SIGNATURE_MARKERS: usize = 1024;

/// `signed|<last verification, 20 digits>|<public key>:<node id>`. The prefix
/// is not a number, so expiry pruning keeps markers like other foreign formats.
const MARKER_PREFIX: &str = "signed|";

/// Replay key carrying the signed expiry, so pruning drops exactly the
/// expired keys instead of flushing the whole set (S16).
pub(super) fn replay_key(expires_at: i64, key: &str) -> String {
    format!("{:020}|{key}", expires_at.max(0))
}

fn replay_expiry(entry: &str) -> i64 {
    entry
        .split_once('|')
        .and_then(|(expiry, _)| expiry.parse().ok())
        // Other authorization paths may share the set. An unrecognized
        // nonce format is not evidence that its replay protection expired.
        .unwrap_or(i64::MAX)
}

fn is_marker(entry: &str) -> bool {
    entry.starts_with(MARKER_PREFIX)
}

fn replay_count(seen: &HashSet<String>) -> usize {
    seen.iter().filter(|entry| !is_marker(entry)).count()
}

pub fn remember_replay(seen: &mut HashSet<String>, key: String, now: i64) -> bool {
    if replay_count(seen) >= MAX_REPLAY_KEYS {
        seen.retain(|entry| is_marker(entry) || replay_expiry(entry) >= now);
    }
    if replay_count(seen) >= MAX_REPLAY_KEYS {
        return false;
    }
    seen.insert(key)
}

fn marker_identity(presence: &PeerPresence) -> String {
    format!("{}:{}", presence.public_key, presence.node_id)
}

/// `(last verification, identity)` of a marker; an unreadable marker sorts
/// first so it is evicted before any readable one.
fn parse_marker(entry: &str) -> Option<(i64, &str)> {
    let rest = entry.strip_prefix(MARKER_PREFIX)?;
    match rest.split_once('|') {
        Some((seen_at, identity)) => Some((seen_at.parse().unwrap_or(i64::MIN), identity)),
        None => Some((i64::MIN, "")),
    }
}

pub(super) fn signature_seen(seen: &HashSet<String>, presence: &PeerPresence) -> bool {
    let identity = marker_identity(presence);
    seen.iter()
        .filter_map(|entry| parse_marker(entry))
        .any(|(_, known)| known == identity)
}

/// Public keys that a direct contact or a room member pins.
fn pinned_keys(contacts: &[DirectContact], rooms: &[RoomProfile]) -> HashSet<String> {
    let mut keys = HashSet::new();
    for contact in contacts {
        keys.extend(contact.remote_public_key.iter().cloned());
        keys.extend(contact.accepted_public_key.iter().cloned());
    }
    for room in rooms {
        keys.extend(
            room.members
                .iter()
                .filter(|member| !member.public_key.is_empty())
                .map(|member| member.public_key.clone()),
        );
    }
    keys
}

/// Records (or refreshes) the marker of a verified signature and keeps the
/// marker count below its bound. Never fails: a full marker memory evicts.
fn remember_signature<F>(seen: &mut HashSet<String>, presence: &PeerPresence, now: i64, pinned: F)
where
    F: FnOnce() -> HashSet<String>,
{
    let identity = marker_identity(presence);
    seen.retain(|entry| parse_marker(entry).is_none_or(|(_, known)| known != identity));
    let mut markers: Vec<(i64, String)> = seen
        .iter()
        .filter_map(|entry| parse_marker(entry).map(|(seen_at, _)| (seen_at, entry.clone())))
        .collect();
    if markers.len() >= MAX_SIGNATURE_MARKERS {
        let pinned = pinned();
        let is_pinned = |entry: &str| {
            parse_marker(entry)
                .and_then(|(_, known)| known.split_once(':'))
                .is_some_and(|(key, _)| pinned.contains(key))
        };
        // Unpinned before pinned, then least recently verified first.
        markers.sort_by(|left, right| {
            (is_pinned(&left.1), left.0, &left.1).cmp(&(is_pinned(&right.1), right.0, &right.1))
        });
        let excess = markers.len() + 1 - MAX_SIGNATURE_MARKERS;
        for (_, entry) in markers.into_iter().take(excess) {
            seen.remove(&entry);
        }
    }
    seen.insert(format!("{MARKER_PREFIX}{:020}|{identity}", now.max(0)));
}

fn remember_presence_in<F>(
    seen: &mut HashSet<String>,
    key: String,
    now: i64,
    presence: &PeerPresence,
    pinned: F,
) -> bool
where
    F: FnOnce() -> HashSet<String>,
{
    if presence.is_signed() {
        remember_signature(seen, presence, now, pinned);
    }
    remember_replay(seen, key, now)
}

/// A signature verified before this call stays remembered even while its
/// first daemon event is still queued. Known peers also persist that fact in
/// their relation flags; this bounded memory covers new incoming requests.
pub(super) fn remember_presence(
    state: &mut ShareAuthState,
    key: String,
    now: i64,
    presence: &PeerPresence,
) -> bool {
    let ShareAuthState {
        seen_nonces,
        direct_contacts,
        rooms,
        ..
    } = state;
    remember_presence_in(seen_nonces, key, now, presence, || {
        pinned_keys(direct_contacts, rooms)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_replay_capacity_rejects_new_without_forgetting_old() {
        let mut seen = HashSet::new();
        for index in 0..MAX_REPLAY_KEYS {
            assert!(remember_replay(
                &mut seen,
                replay_key(20, &index.to_string()),
                10
            ));
        }
        assert!(!remember_replay(&mut seen, replay_key(20, "new"), 10));
        assert!(seen.contains(&replay_key(20, "0")));
        assert!(seen.contains(&replay_key(20, &(MAX_REPLAY_KEYS - 1).to_string())));
        assert!(remember_replay(&mut seen, replay_key(30, "new"), 21));
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn review_task_first_signature_is_remembered_before_daemon_event() {
        let mut seen = HashSet::new();
        let mut presence = PeerPresence {
            kind: "direct".into(),
            relation_id: "lookup".into(),
            device_id: "peer".into(),
            device_name: "Peer".into(),
            public_key: "key".into(),
            fingerprint: "fp".into(),
            node_id: "key".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: 20,
            nonce: "already-verified.ps1.signature".into(),
            proof: String::new(),
        };
        assert!(!signature_seen(&seen, &presence));
        assert!(remember_presence_in(
            &mut seen,
            replay_key(20, "message"),
            10,
            &presence,
            HashSet::new
        ));
        presence.device_id = "different-id".into();
        presence.nonce = "unsigned".into();
        assert!(signature_seen(&seen, &presence));
        seen.retain(|entry| replay_expiry(entry) >= 21);
        assert!(signature_seen(&seen, &presence));
    }

    #[test]
    fn review_task_replay_pruning_retains_other_authorization_nonce_formats() {
        let mut seen = HashSet::from(["session:authenticated-nonce".to_string()]);
        for index in 1..MAX_REPLAY_KEYS {
            assert!(remember_replay(
                &mut seen,
                replay_key(20, &index.to_string()),
                10
            ));
        }
        assert!(remember_replay(&mut seen, replay_key(30, "fresh"), 21));
        assert!(seen.contains("session:authenticated-nonce"));
        assert_eq!(seen.len(), 2);
    }
}

#[cfg(test)]
#[path = "signal_auth_replay_task_tests.rs"]
mod task_tests;
