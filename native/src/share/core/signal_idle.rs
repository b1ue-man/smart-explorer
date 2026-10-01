//! Client side of the server's idle mode (`idle_keepalive_v1`, V1): which
//! `set_idle` the connection needs, what the server confirmed, and the
//! keepalive interval the client proposes when a middlebox cuts quiet
//! connections early (K5).

use super::schedule::{clamp_keepalive_secs, IDLE_MIN_KEEPALIVE_SECS};
use crate::share::wire::ClientMsg;

/// After this long without an early end (wall clock) the client stops
/// proposing a shorter keepalive.
const STABLE_PERIOD_MS: i64 = 60 * 60 * 1000;
/// Consecutive early ends of idle connections before halving.
const EARLY_ENDS_BEFORE_HALVING: u8 = 2;

/// Idle state of one signal connection.
pub(crate) struct IdleLink {
    supported: bool,
    /// Last `set_idle` sent: (idle, proposed keepalive).
    requested: Option<(bool, Option<u32>)>,
    /// Keepalive interval of a server-confirmed idle mode.
    active_secs: Option<u32>,
}

impl IdleLink {
    pub(crate) fn new(supported: bool) -> Self {
        Self {
            supported,
            requested: None,
            active_secs: None,
        }
    }

    pub(crate) fn supported(&self) -> bool {
        self.supported
    }

    pub(crate) fn active_secs(&self) -> Option<u32> {
        self.active_secs
    }

    /// The `set_idle` that makes the server follow `low_power`, if any is
    /// missing. A fresh connection starts in normal mode on the server.
    pub(crate) fn wanted(&self, low_power: bool, proposal: Option<u32>) -> Option<ClientMsg> {
        if !self.supported {
            return None;
        }
        let want = (low_power, proposal.filter(|_| low_power));
        if self.requested == Some(want) || (self.requested.is_none() && !low_power) {
            return None;
        }
        Some(ClientMsg::SetIdle {
            idle: want.0,
            keepalive_secs: want.1,
        })
    }

    /// `message` (from `wanted`) was sent. Leaving idle mode takes effect
    /// at once: heartbeats are always accepted.
    pub(crate) fn sent(&mut self, message: &ClientMsg) {
        if let ClientMsg::SetIdle {
            idle,
            keepalive_secs,
        } = message
        {
            self.requested = Some((*idle, *keepalive_secs));
            if !*idle {
                self.active_secs = None;
            }
        }
    }

    /// The server's `idle_ack`; returns whether the idle state changed. An
    /// acknowledgement of an outdated request leaves the state unchanged.
    pub(crate) fn acknowledged(&mut self, idle: bool, keepalive_secs: Option<u32>) -> bool {
        if !self.supported {
            return false;
        }
        let requested_idle = self.requested.is_some_and(|(requested, _)| requested);
        let active = (idle && requested_idle).then(|| clamp_keepalive_secs(keepalive_secs));
        std::mem::replace(&mut self.active_secs, active) != active
    }
}

/// Keepalive proposal across the connections of one worker (K5).
#[derive(Default)]
pub(crate) struct KeepaliveTuning {
    proposal: Option<u32>,
    early_ends: u8,
    stable_since_ms: Option<i64>,
}

impl KeepaliveTuning {
    pub(crate) fn proposal(&self) -> Option<u32> {
        self.proposal
    }

    /// The transport of an idle connection ended (end of stream, reset,
    /// I/O error), `silent_ms` after the last line from the server.
    pub(crate) fn transport_ended(&mut self, active_secs: Option<u32>, silent_ms: i64) {
        let Some(secs) = active_secs else {
            return;
        };
        if silent_ms >= i64::from(secs) * 1000 {
            // The server's keepalive was due: not a middlebox timeout.
            self.early_ends = 0;
            return;
        }
        self.stable_since_ms = None;
        self.early_ends = self.early_ends.saturating_add(1);
        if self.early_ends >= EARLY_ENDS_BEFORE_HALVING {
            self.early_ends = 0;
            self.proposal = Some((secs / 2).max(IDLE_MIN_KEEPALIVE_SECS));
        }
    }

    /// An idle connection is alive at `now_ms`; returns whether the
    /// proposal fell back to the server default.
    pub(crate) fn alive(&mut self, now_ms: i64) -> bool {
        if self.proposal.is_none() {
            return false;
        }
        let since = *self.stable_since_ms.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) < STABLE_PERIOD_MS {
            return false;
        }
        self.proposal = None;
        self.stable_since_ms = None;
        self.early_ends = 0;
        true
    }
}

#[cfg(test)]
mod android_background_task_idle_tests {
    use super::{IdleLink, KeepaliveTuning, STABLE_PERIOD_MS};
    use crate::share::wire::ClientMsg;

    fn set_idle(message: Option<ClientMsg>) -> Option<(bool, Option<u32>)> {
        match message? {
            ClientMsg::SetIdle {
                idle,
                keepalive_secs,
            } => Some((idle, keepalive_secs)),
            _ => None,
        }
    }

    #[test]
    fn android_background_task_set_idle_follows_power_once() {
        let old_server = IdleLink::new(false);
        assert!(old_server.wanted(true, None).is_none());

        let mut link = IdleLink::new(true);
        assert!(link.wanted(false, None).is_none(), "fresh connection");
        let enter = link.wanted(true, Some(90)).unwrap();
        assert_eq!(set_idle(Some(enter.clone())), Some((true, Some(90))));
        link.sent(&enter);
        assert!(link.wanted(true, Some(90)).is_none(), "already requested");
        assert_eq!(link.active_secs(), None, "until the server confirms");
        assert!(link.acknowledged(true, Some(90)));
        assert_eq!(link.active_secs(), Some(90));
        // The proposal falls back to the default: ask again without it.
        assert_eq!(set_idle(link.wanted(true, None)), Some((true, None)));

        let leave = link.wanted(false, Some(90)).unwrap();
        assert_eq!(set_idle(Some(leave.clone())), Some((false, None)));
        link.sent(&leave);
        assert_eq!(link.active_secs(), None, "leaving idle is immediate");
        // A late acknowledgement of the old request changes nothing.
        assert!(!link.acknowledged(true, Some(90)));
        assert_eq!(link.active_secs(), None);
    }

    #[test]
    fn android_background_task_server_keepalive_is_clamped() {
        let mut link = IdleLink::new(true);
        let enter = link.wanted(true, None).unwrap();
        link.sent(&enter);
        link.acknowledged(true, None);
        assert_eq!(link.active_secs(), Some(180), "V1 default");
        link.acknowledged(true, Some(5));
        assert_eq!(link.active_secs(), Some(30));
    }

    #[test]
    fn android_background_task_early_ends_halve_the_keepalive() {
        let mut tuning = KeepaliveTuning::default();
        // Not idle, or ended after the server's keepalive was due: ignored.
        tuning.transport_ended(None, 10_000);
        tuning.transport_ended(Some(180), 200_000);
        tuning.transport_ended(Some(180), 60_000);
        assert_eq!(tuning.proposal(), None, "one early end is not enough");
        tuning.transport_ended(Some(180), 200_000);
        tuning.transport_ended(Some(180), 60_000);
        assert_eq!(tuning.proposal(), None, "early ends must be consecutive");
        tuning.transport_ended(Some(180), 61_000);
        assert_eq!(tuning.proposal(), Some(90));
        tuning.transport_ended(Some(90), 40_000);
        tuning.transport_ended(Some(90), 40_000);
        assert_eq!(tuning.proposal(), Some(45));
        tuning.transport_ended(Some(45), 20_000);
        tuning.transport_ended(Some(45), 20_000);
        assert_eq!(tuning.proposal(), Some(30), "never below 30 s");

        assert!(!tuning.alive(1_000_000));
        assert!(!tuning.alive(1_000_000 + STABLE_PERIOD_MS - 1));
        assert!(tuning.alive(1_000_000 + STABLE_PERIOD_MS));
        assert_eq!(tuning.proposal(), None, "back to the server default");
    }
}
