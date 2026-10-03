//! Bounded replay admission and immediate in-memory signature memory.
use std::collections::HashSet;

use crate::share::types::PeerPresence;

/// Reject fresh messages at capacity; authenticated evidence is never evicted.
const MAX_REPLAY_KEYS: usize = 4096;

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

pub(crate) fn remember_replay(seen: &mut HashSet<String>, key: String, now: i64) -> bool {
    if seen.len() >= MAX_REPLAY_KEYS {
        seen.retain(|entry| replay_expiry(entry) >= now);
    }
    if seen.len() >= MAX_REPLAY_KEYS {
        return false;
    }
    seen.insert(key)
}


fn signature_marker(presence: &PeerPresence) -> String {
    replay_key(i64::MAX, &format!("signed:{}:{}", presence.public_key, presence.node_id))
}

pub(super) fn signature_seen(seen: &HashSet<String>, presence: &PeerPresence) -> bool {
    seen.contains(&signature_marker(presence))
}

/// A signature verified before this call stays remembered even while its
/// first daemon event is still queued. Known peers also persist that fact in
/// their relation flags; this bounded memory covers new incoming requests.
pub(super) fn remember_presence(
    seen: &mut HashSet<String>, key: String, now: i64, presence: &PeerPresence,
) -> bool {
    if presence.is_signed() && !signature_seen(seen, presence)
        && !remember_replay(seen, signature_marker(presence), now)
    {
        return false;
    }
    remember_replay(seen, key, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_replay_capacity_rejects_new_without_forgetting_old() {
        let mut seen = HashSet::new();
        for index in 0..MAX_REPLAY_KEYS {
            assert!(remember_replay(&mut seen, replay_key(20, &index.to_string()), 10));
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
            kind: "direct".into(), relation_id: "lookup".into(), device_id: "peer".into(),
            device_name: "Peer".into(), public_key: "key".into(), fingerprint: "fp".into(),
            node_id: "key".into(), relay_url: String::new(), candidates: Vec::new(),
            expires_at: 20, nonce: "already-verified.ps1.signature".into(), proof: String::new(),
        };
        assert!(!signature_seen(&seen, &presence));
        assert!(remember_presence(&mut seen, replay_key(20, "message"), 10, &presence));
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
            assert!(remember_replay(&mut seen, replay_key(20, &index.to_string()), 10));
        }
        assert!(remember_replay(&mut seen, replay_key(30, "fresh"), 21));
        assert!(seen.contains("session:authenticated-nonce"));
        assert_eq!(seen.len(), 2);
    }
}
