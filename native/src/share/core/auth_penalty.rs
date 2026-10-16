//! Penalties for failed Share authentication, per remote EndpointId.
//!
//! The model and numbers are OpenSSH's `PerSourcePenalties` defaults
//! (sshd_config(5), OpenSSH 9.8 and later): a failed authentication adds
//! 5 s, a connection that never attempts authentication adds 1 s, a source
//! is refused once its accumulated penalty reaches 15 s and then until the
//! penalty has run out, a penalty never exceeds 600 s, and at most 65 536
//! sources are tracked; beyond that, new sources are admitted untracked
//! ("permissive" overflow) instead of being refused.
//! Iroh authenticates the EndpointId in TLS before any Share frame, so the
//! key here is the cryptographic peer identity, not an IP address that a
//! relay or NAT shares between many peers.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const AUTH_FAILURE: Duration = Duration::from_secs(5);
const NO_AUTHENTICATION: Duration = Duration::from_secs(1);
const MIN_PENALTY: Duration = Duration::from_secs(15);
const MAX_PENALTY: Duration = Duration::from_secs(600);
const MAX_SOURCES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PenaltyKind {
    /// Identity or relation proof did not verify.
    AuthFailure,
    /// The connection ended or timed out before it attempted to authenticate.
    NoAuthentication,
}

impl PenaltyKind {
    fn duration(self) -> Duration {
        match self {
            Self::AuthFailure => AUTH_FAILURE,
            Self::NoAuthentication => NO_AUTHENTICATION,
        }
    }
}

/// Whether a penalty of `source` turns its connection away: every Exec
/// connection, and file sessions of unknown endpoints only. File sessions of
/// paired devices reconnect on their own (network change, sleep) and are
/// never held back.
pub(crate) fn refusal_applies(exec: bool, known: bool) -> bool {
    exec || !known
}

#[derive(Clone, Copy)]
struct Penalty {
    /// Instant until which the accumulated penalty runs.
    until: Instant,
    /// Reached the 15 s threshold: refused until `until`.
    active: bool,
}

#[derive(Default)]
pub(crate) struct AuthPenalties {
    sources: Mutex<HashMap<String, Penalty>>,
}

impl AuthPenalties {
    /// Remaining refusal time of `source`, when it is currently refused.
    pub(crate) fn refusal(&self, source: &str, now: Instant) -> Option<Duration> {
        let sources = self
            .sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let penalty = sources.get(source)?;
        if !penalty.active {
            return None;
        }
        penalty
            .until
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
    }

    /// Adds one penalty of `kind` to `source`.
    pub(crate) fn record(&self, source: &str, kind: PenaltyKind, now: Instant) {
        if source.is_empty() || source.len() > 128 {
            return;
        }
        let mut sources = self
            .sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !sources.contains_key(source) && sources.len() >= MAX_SOURCES {
            sources.retain(|_, penalty| penalty.until > now);
            if sources.len() >= MAX_SOURCES {
                return;
            }
        }
        let current = sources
            .get(source)
            .copied()
            .filter(|penalty| penalty.until > now);
        let base = current.map_or(now, |penalty| penalty.until);
        let until = (base + kind.duration()).min(now + MAX_PENALTY);
        let active = current.is_some_and(|penalty| penalty.active)
            || until.saturating_duration_since(now) >= MIN_PENALTY;
        sources.insert(source.to_string(), Penalty { until, active });
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.sources.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luecken_task_three_failed_authentications_refuse_for_fifteen_seconds() {
        let penalties = AuthPenalties::default();
        let start = Instant::now();
        penalties.record("peer", PenaltyKind::AuthFailure, start);
        penalties.record("peer", PenaltyKind::AuthFailure, start);
        assert_eq!(penalties.refusal("peer", start), None);
        penalties.record("peer", PenaltyKind::AuthFailure, start);
        assert_eq!(penalties.refusal("peer", start), Some(MIN_PENALTY));
        assert_eq!(penalties.refusal("other", start), None);
        // Once enforced, the refusal lasts until the penalty has run out.
        assert_eq!(
            penalties.refusal("peer", start + Duration::from_secs(10)),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            penalties.refusal("peer", start + Duration::from_secs(15)),
            None
        );
        assert_eq!(
            penalties.refusal("peer", start + Duration::from_secs(16)),
            None
        );
    }

    #[test]
    fn luecken_task_file_sessions_of_paired_devices_are_never_turned_away() {
        assert!(!refusal_applies(false, true));
        assert!(refusal_applies(false, false));
        assert!(refusal_applies(true, true));
        assert!(refusal_applies(true, false));
    }

    #[test]
    fn luecken_task_penalties_are_capped_and_decay() {
        let penalties = AuthPenalties::default();
        let start = Instant::now();
        for _ in 0..1000 {
            penalties.record("peer", PenaltyKind::AuthFailure, start);
        }
        assert_eq!(penalties.refusal("peer", start), Some(MAX_PENALTY));
        let later = start + MAX_PENALTY + Duration::from_secs(1);
        assert_eq!(penalties.refusal("peer", later), None);
        penalties.record("peer", PenaltyKind::NoAuthentication, later);
        assert_eq!(penalties.refusal("peer", later), None);
    }

    #[test]
    fn luecken_task_source_map_is_bounded_and_prunes_expired_entries() {
        let penalties = AuthPenalties::default();
        let start = Instant::now();
        for index in 0..MAX_SOURCES {
            penalties.record(
                &format!("peer-{index}"),
                PenaltyKind::NoAuthentication,
                start,
            );
        }
        assert_eq!(penalties.tracked(), MAX_SOURCES);
        penalties.record("overflow", PenaltyKind::AuthFailure, start);
        assert_eq!(penalties.tracked(), MAX_SOURCES);
        let later = start + Duration::from_secs(2);
        penalties.record("overflow", PenaltyKind::AuthFailure, later);
        assert_eq!(penalties.tracked(), 1);
        penalties.record("", PenaltyKind::AuthFailure, later);
        assert_eq!(penalties.tracked(), 1);
    }
}
