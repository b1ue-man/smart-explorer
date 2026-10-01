//! When the connected signal worker has to act, as a pure function of both
//! clocks (tests run it with a stopped monotonic clock).
//!
//! Normal operation keeps the established cadence: heartbeat every 20 s,
//! pong within 40 s, presence every 60 s, tracked outbox every 2 s. In idle
//! mode (server confirmed `idle_keepalive_v1`) the server drives the
//! connection: no heartbeats, presence renewed at server keepalives and
//! probes, and the connection counts as dead once the server was silent for
//! its keepalive interval plus the reply window. In low power without idle
//! mode (old server) heartbeats continue; staleness and presence age then
//! also read the wall clock, because monotonic timers stood still while the
//! device slept.

use std::time::{Duration, Instant};

use crate::share::keepalive::{
    SignalMaintenanceDue, SIGNAL_HEARTBEAT_INTERVAL, SIGNAL_MAINTENANCE_POLICY,
    SIGNAL_PONG_TIMEOUT, SIGNAL_PRESENCE_REFRESH_INTERVAL, SIGNAL_TRACKED_OUTBOX_INTERVAL,
};
use crate::share::power::clock::Now;

/// Server keepalive interval when its `idle_ack` names none (V1 default).
pub(crate) const IDLE_DEFAULT_KEEPALIVE_SECS: u32 = 180;
/// Shortest keepalive interval a client proposes (K5).
pub(crate) const IDLE_MIN_KEEPALIVE_SECS: u32 = 30;
const IDLE_MAX_KEEPALIVE_SECS: u32 = 1_800;
/// The server closes an idle connection this long after an unanswered
/// keepalive (`IDLE_REPLY_WINDOW`).
const IDLE_REPLY_WINDOW: Duration = Duration::from_secs(60);
/// Transit and scheduling slack on top of the server's own deadlines.
const STALE_SLACK: Duration = Duration::from_secs(30);
/// Without idle mode the server closes after 60 s without a client line;
/// heartbeats every 20 s make a silent server dead after this much.
const LEGACY_STALE_AFTER: Duration = Duration::from_secs(90);
/// Lifetime of a signed presence (`signal_presence`).
const PRESENCE_LIFETIME: Duration = Duration::from_secs(300);
/// In idle mode a presence older than this is renewed at the next wake (K2).
const IDLE_PRESENCE_MAX_AGE: Duration = Duration::from_secs(120);
/// A probe's heartbeat must be answered within this.
pub(crate) const PROBE_REPLY_DEADLINE: Duration = Duration::from_secs(10);

pub(crate) fn clamp_keepalive_secs(secs: Option<u32>) -> u32 {
    secs.unwrap_or(IDLE_DEFAULT_KEEPALIVE_SECS)
        .clamp(IDLE_MIN_KEEPALIVE_SECS, IDLE_MAX_KEEPALIVE_SECS)
}

/// What the worker's mode means for its timers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SignalMode {
    pub(crate) low_power: bool,
    /// Keepalive interval of a server-confirmed idle mode.
    pub(crate) idle_secs: Option<u32>,
    /// Whether the tracked-direct outbox is polled.
    pub(crate) tracked: bool,
}

pub(crate) struct SignalSchedule {
    last_heartbeat: Instant,
    heartbeat_now: bool,
    pong_since: Option<Instant>,
    last_publish: Now,
    last_tracked: Instant,
    last_inbound: Now,
    probe_until: Option<Instant>,
}

impl SignalSchedule {
    pub(crate) fn new(now: Now) -> Self {
        Self {
            last_heartbeat: now.mono,
            heartbeat_now: false,
            pong_since: None,
            last_publish: now,
            last_tracked: now.mono,
            last_inbound: now,
            probe_until: None,
        }
    }

    pub(crate) fn heartbeat_due(&self, now: Now, mode: SignalMode) -> bool {
        mode.idle_secs.is_none() && (self.heartbeat_now || self.due(now, mode).heartbeat)
    }

    /// Sends the next heartbeat at once (leaving idle mode).
    pub(crate) fn heartbeat_soon(&mut self) {
        self.heartbeat_now = true;
    }

    pub(crate) fn heartbeat_sent(&mut self, now: Now) {
        self.last_heartbeat = now.mono;
        self.heartbeat_now = false;
        self.pong_since.get_or_insert(now.mono);
    }

    /// A pong arrived; returns whether it answered a running probe.
    pub(crate) fn pong_received(&mut self) -> bool {
        self.pong_since = None;
        self.probe_until.take().is_some()
    }

    pub(crate) fn pong_expired(&self, now: Now) -> bool {
        SIGNAL_MAINTENANCE_POLICY.pong_expired(
            self.pong_since
                .map(|since| now.mono.saturating_duration_since(since)),
        )
    }

    /// Presence renewal by timer; idle mode renews at wakes instead.
    pub(crate) fn presence_due(&self, now: Now, mode: SignalMode) -> bool {
        if mode.idle_secs.is_some() {
            return false;
        }
        self.due(now, mode).presence_refresh
            || (mode.low_power
                && now.wall_since(&self.last_publish) >= SIGNAL_PRESENCE_REFRESH_INTERVAL)
    }

    /// Presence renewal at a server keepalive or probe in idle mode: when
    /// older than two minutes, or when it would expire before the next
    /// keepalive could renew it.
    pub(crate) fn idle_presence_due(&self, now: Now, keepalive_secs: u32) -> bool {
        let age = now.wall_since(&self.last_publish);
        let next_keepalive = Duration::from_secs(u64::from(keepalive_secs))
            .saturating_sub(now.wall_since(&self.last_inbound));
        age >= IDLE_PRESENCE_MAX_AGE
            || PRESENCE_LIFETIME.saturating_sub(age)
                < next_keepalive + IDLE_REPLY_WINDOW + STALE_SLACK
    }

    pub(crate) fn published(&mut self, now: Now) {
        self.last_publish = now;
    }

    pub(crate) fn tracked_due(&self, now: Now, mode: SignalMode) -> bool {
        self.due(now, mode).tracked_outbox
    }

    pub(crate) fn tracked_sent(&mut self, now: Now) {
        self.last_tracked = now.mono;
    }

    pub(crate) fn inbound(&mut self, now: Now) {
        self.last_inbound = now;
    }

    pub(crate) fn last_inbound(&self) -> Now {
        self.last_inbound
    }

    /// The server has been silent longer than it may be on a live
    /// connection. Only in low power: a desktop keeps its pong timeout.
    pub(crate) fn stale(&self, now: Now, mode: SignalMode) -> bool {
        mode.low_power && now.wall_since(&self.last_inbound) > stale_after(mode)
    }

    pub(crate) fn probe_started(&mut self, now: Now) {
        self.probe_until = Some(now.mono + PROBE_REPLY_DEADLINE);
    }

    pub(crate) fn probing(&self) -> bool {
        self.probe_until.is_some()
    }

    pub(crate) fn probe_expired(&self, now: Now) -> bool {
        self.probe_until.is_some_and(|until| now.mono >= until)
    }

    /// Time until the next timer of this mode, if any.
    pub(crate) fn next_wake(&self, now: Now, mode: SignalMode) -> Option<Duration> {
        let mut deadlines = Vec::with_capacity(6);
        if mode.idle_secs.is_none() {
            deadlines.push(self.last_heartbeat + SIGNAL_HEARTBEAT_INTERVAL);
            deadlines.push(self.last_publish.mono + SIGNAL_PRESENCE_REFRESH_INTERVAL);
        }
        if let Some(since) = self.pong_since {
            deadlines.push(since + SIGNAL_PONG_TIMEOUT);
        }
        if mode.tracked {
            deadlines.push(self.last_tracked + SIGNAL_TRACKED_OUTBOX_INTERVAL);
        }
        if mode.low_power {
            deadlines.push(self.last_inbound.mono + stale_after(mode) + Duration::from_secs(1));
        }
        if let Some(until) = self.probe_until {
            deadlines.push(until);
        }
        let earliest = deadlines.into_iter().min()?;
        if self.heartbeat_now && mode.idle_secs.is_none() {
            return Some(Duration::ZERO);
        }
        Some(earliest.saturating_duration_since(now.mono))
    }

    fn due(&self, now: Now, mode: SignalMode) -> SignalMaintenanceDue {
        SIGNAL_MAINTENANCE_POLICY.due(
            now.mono.saturating_duration_since(self.last_heartbeat),
            now.mono.saturating_duration_since(self.last_publish.mono),
            now.mono.saturating_duration_since(self.last_tracked),
            mode.tracked,
        )
    }
}

fn stale_after(mode: SignalMode) -> Duration {
    match mode.idle_secs {
        Some(secs) => Duration::from_secs(u64::from(secs)) + IDLE_REPLY_WINDOW + STALE_SLACK,
        None => LEGACY_STALE_AFTER,
    }
}

#[cfg(test)]
#[path = "signal_schedule_tests.rs"]
mod android_background_task_schedule_tests;
