//! When the host measures the line (`bandwidth_probe.rs`): right after the
//! session opened, after a route change of the connection (a new
//! generation, a pause or another class, `share::route_monitor`, as the
//! pipeline's `route_epoch`), on the viewer's „Neu messen“ (`REMEASURE`),
//! every `PERIODIC_US` while the session streams, and when the line no longer
//! carries the measured rate: under `Realtime` `STALE_RESETS` stale stream
//! resets within `RESET_WINDOW_US`, under `Reliable` an oldest unacknowledged
//! picture older than `BACKLOG_US` that was sent after the last probe
//! started. Never two probes at once; a probe without an answer within
//! `RESULT_TIMEOUT_US` is dropped and a late answer ignored. The resets the
//! probe itself may cause (it fills the line's queue) count for nothing: the
//! count starts again when a probe starts and when it ends.
//!
//! The session asks once a second (`STATS`), and at once on `REMEASURE`; the
//! facts are those of that moment, so the reset times are the times of the
//! check that saw them (at most a second late).
use std::collections::VecDeque;

use super::super::delivery::Delivery;
use super::RESULT_TIMEOUT_US;

/// A route as the pipeline watches it: generation, pause epoch, class.
pub type RouteKey = (u64, u64, Option<crate::share::route_state::RouteClass>);

/// A new probe this long after the last one started: five minutes. A probe
/// fills the line for up to 1.5 s and delays the pictures meanwhile
/// (measurement 2026-11-25 ASUS→Shadow over WLAN: latency p95 above 600 ms
/// in the probe's second); a changed line shows earlier by its resets or
/// backlog, a route change or „Neu messen“.
pub const PERIODIC_US: u64 = 300_000_000;
/// Stale resets within `RESET_WINDOW_US` that ask for a new probe.
pub const STALE_RESETS: usize = 3;
pub const RESET_WINDOW_US: u64 = 10_000_000;
/// Age of the oldest unacknowledged picture that asks for a new probe under
/// `Reliable`.
pub const BACKLOG_US: u64 = 2_000_000;

/// What the pipeline knows of the line (`Pipeline::line_facts`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineFacts {
    /// Picture streams given up because their oldest picture went stale
    /// (B-35), since the session started.
    pub stale_resets: u64,
    /// When the oldest unacknowledged picture was handed to its stream (µs
    /// of the wall clock); `None` with nothing in flight.
    pub oldest_unacked_sent_us: Option<u64>,
}

/// The facts of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeFacts {
    pub now_us: u64,
    /// `None` without a route monitor (a connection without the policy).
    pub route: Option<RouteKey>,
    /// The route lets the application send and the session streams the
    /// full view.
    pub sendable: bool,
    pub delivery: Delivery,
    pub line: LineFacts,
}

/// Why a probe starts (`key` in the host log).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeReason {
    Opening,
    RouteChange,
    Remeasure,
    Periodic,
    StaleResets,
    Backlog,
}

impl ProbeReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::Opening => "opening",
            Self::RouteChange => "route_change",
            Self::Remeasure => "remeasure",
            Self::Periodic => "periodic",
            Self::StaleResets => "stale_resets",
            Self::Backlog => "backlog",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Running {
    id: u64,
    started_us: u64,
}

/// The probes of one session.
#[derive(Clone, Debug, Default)]
pub struct ProbeSchedule {
    next_id: u64,
    running: Option<Running>,
    last_started_us: Option<u64>,
    route: Option<RouteKey>,
    route_changed: bool,
    remeasure: bool,
    seen_resets: Option<u64>,
    /// The check times of the newest stale resets since the last probe
    /// started or ended, oldest first; at most `STALE_RESETS`.
    resets: VecDeque<u64>,
}

impl ProbeSchedule {
    /// „Neu messen“: the next check starts a probe. A running probe answers
    /// it already.
    pub fn request(&mut self) {
        self.remeasure = self.running.is_none();
    }

    /// The id of the running probe.
    pub fn running(&self) -> Option<u64> {
        self.running.map(|running| running.id)
    }

    /// Drops a probe without an answer after `RESULT_TIMEOUT_US`; its id.
    pub fn expire(&mut self, now_us: u64) -> Option<u64> {
        let running = self.running?;
        if now_us.saturating_sub(running.started_us) <= RESULT_TIMEOUT_US {
            return None;
        }
        self.running = None;
        self.resets.clear();
        Some(running.id)
    }

    /// The answer to probe `probe_id`: `true` when it is the running one,
    /// which ends with it.
    pub fn on_result(&mut self, probe_id: u64) -> bool {
        if self.running() != Some(probe_id) {
            return false;
        }
        self.running = None;
        self.resets.clear();
        true
    }

    /// Takes the facts in; the id and reason of a probe to start now, which
    /// counts as running from here on.
    pub fn due(&mut self, facts: &ProbeFacts) -> Option<(u64, ProbeReason)> {
        self.note_route(facts.route);
        self.note_resets(facts.now_us, facts.line.stale_resets);
        if self.running.is_some() || !facts.sendable {
            return None;
        }
        let reason = self.reason(facts)?;
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        self.running = Some(Running {
            id,
            started_us: facts.now_us,
        });
        self.last_started_us = Some(facts.now_us);
        self.route_changed = false;
        self.remeasure = false;
        self.resets.clear();
        Some((id, reason))
    }

    fn reason(&self, facts: &ProbeFacts) -> Option<ProbeReason> {
        let now = facts.now_us;
        let Some(last) = self.last_started_us else {
            return Some(if self.remeasure {
                ProbeReason::Remeasure
            } else {
                ProbeReason::Opening
            });
        };
        let resets = facts.delivery == Delivery::Realtime
            && self.resets.len() >= STALE_RESETS
            && self
                .resets
                .front()
                .is_some_and(|&first| now.saturating_sub(first) <= RESET_WINDOW_US);
        let backlog = facts.delivery == Delivery::Reliable
            && facts
                .line
                .oldest_unacked_sent_us
                .is_some_and(|sent| sent >= last && now.saturating_sub(sent) > BACKLOG_US);
        if self.remeasure {
            Some(ProbeReason::Remeasure)
        } else if self.route_changed {
            Some(ProbeReason::RouteChange)
        } else if resets {
            Some(ProbeReason::StaleResets)
        } else if backlog {
            Some(ProbeReason::Backlog)
        } else if now.saturating_sub(last) >= PERIODIC_US {
            Some(ProbeReason::Periodic)
        } else {
            None
        }
    }

    /// The first route seen is the session's own, no change.
    fn note_route(&mut self, route: Option<RouteKey>) {
        let Some(route) = route else {
            return;
        };
        if self.route.is_some_and(|known| known != route) {
            self.route_changed = true;
        }
        self.route = Some(route);
    }

    fn note_resets(&mut self, now_us: u64, total: u64) {
        let seen = self.seen_resets.replace(total).unwrap_or(total);
        let fresh = usize::try_from(total.saturating_sub(seen)).unwrap_or(usize::MAX);
        for _ in 0..fresh.min(STALE_RESETS) {
            if self.resets.len() == STALE_RESETS {
                self.resets.pop_front();
            }
            self.resets.push_back(now_us);
        }
    }
}

#[cfg(test)]
#[path = "bandwidth_probe_schedule_tests.rs"]
mod tests;
