//! Direct retries of a connection whose relay carries payload as the marked
//! fallback (user report 2026-11-25: „die verbindung geht andauernd über das
//! relay … sie muss wenn möglich direkt gehen“).
//!
//! The finite direct phase of a generation (`route_policy.rs`) ends with the
//! relay as the marked fallback. A direct path validated later is still
//! admitted in place by the unchanged selection (`route_start.rs`,
//! `StartPhase::choose`): same connection and streams, no restart. Before
//! this schedule nothing on the connection asked for such a path any more.
//! It decides when the route actor (`route_monitor_retry.rs`) starts a new
//! direct attempt with freshly collected candidates:
//!
//! - the first one 5 s after the relay became the fallback, then after 10,
//!   20 and 40 s and every 60 s while the connection lives;
//! - at once, at most one per second, when this device's direct addresses or
//!   the peer's published ones differ from those of the last attempt; such an
//!   attempt restarts the backoff at 5 s;
//! - none once a LAN or direct path is admitted. A fallback within 60 s of a
//!   direct admission resumes the previous backoff instead of restarting it,
//!   so a path that validates and fails again cannot cause attempts every 5 s.
//!
//! Pure: the clock is an input (milliseconds of the route actor).
use super::super::route_diagnostics::RetryCause;

/// Delay of the first attempt after the relay became the fallback.
pub(in crate::share) const FIRST_RETRY_MS: u64 = 5_000;
/// Longest delay between two attempts.
pub(in crate::share) const MAX_RETRY_MS: u64 = 60_000;
/// Address changes closer together than this start one attempt.
pub(in crate::share) const MIN_TRIGGER_GAP_MS: u64 = 1_000;

/// Fingerprints of the candidate sets of one attempt: this device's direct
/// addresses and the peer's published ones.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::share) struct Candidates {
    pub(in crate::share) local: u64,
    pub(in crate::share) peer: u64,
}

/// Retry schedule of one route generation.
#[derive(Debug, Default)]
pub(in crate::share) struct RetrySchedule {
    armed: bool,
    attempts: u32,
    /// Delay of the scheduled attempt after the previous one.
    delay_ms: u64,
    next_ms: u64,
    scheduled_ms: u64,
    last_ms: Option<u64>,
    candidates: Candidates,
    /// An address change brought the next attempt forward.
    pending: Option<RetryCause>,
    last_cause: Option<RetryCause>,
    /// Start of the last LAN/direct admission.
    direct_since: Option<u64>,
    /// The previous observation had a LAN/direct admission.
    was_direct: bool,
    /// Changes whenever the shown reason may change.
    revision: u64,
}

impl RetrySchedule {
    /// Follows the route at `now`. `fallback`: the relay carries payload as
    /// the marked fallback on a connection this side can retry; `direct`: a
    /// LAN or direct path is admitted. `candidates` are the current ones,
    /// read only when the schedule arms. Returns whether it changed.
    pub(in crate::share) fn follow(
        &mut self,
        now: u64,
        fallback: bool,
        direct: bool,
        candidates: Candidates,
    ) -> bool {
        if direct && !self.was_direct {
            self.direct_since = Some(now);
        }
        self.was_direct = direct;
        if fallback {
            if self.armed {
                return false;
            }
            let resume = self.attempts > 0
                && self
                    .direct_since
                    .is_some_and(|since| now.saturating_sub(since) < MAX_RETRY_MS);
            if !resume {
                self.attempts = 0;
                self.delay_ms = FIRST_RETRY_MS;
                self.last_cause = None;
            }
            self.armed = true;
            self.direct_since = None;
            self.candidates = candidates;
            self.pending = None;
            self.schedule(now, now.saturating_add(self.delay_ms));
            return true;
        }
        if !self.armed {
            return false;
        }
        self.armed = false;
        self.pending = None;
        self.revision = self.revision.saturating_add(1);
        true
    }

    /// A new network generation starts a new start phase: no retry state
    /// carries over.
    pub(in crate::share) fn reset(&mut self) {
        let revision = self.revision.saturating_add(1);
        *self = Self {
            revision,
            ..Self::default()
        };
    }

    pub(in crate::share) fn armed(&self) -> bool {
        self.armed
    }

    pub(in crate::share) fn due(&self, now: u64) -> bool {
        self.armed && now >= self.next_ms
    }

    /// Time until the next attempt while armed.
    pub(in crate::share) fn wait_ms(&self, now: u64) -> Option<u64> {
        self.armed.then(|| self.next_ms.saturating_sub(now))
    }

    /// Compares the current candidates with those of the last attempt; a
    /// difference brings the next attempt forward to now, or to one second
    /// after the last attempt. Returns whether the schedule changed.
    pub(in crate::share) fn observe_candidates(&mut self, now: u64, current: Candidates) -> bool {
        if !self.armed || self.pending.is_some() || current == self.candidates {
            return false;
        }
        self.pending = Some(if current.local != self.candidates.local {
            RetryCause::LocalAddresses
        } else {
            RetryCause::PeerAddresses
        });
        let earliest = self
            .last_ms
            .map_or(now, |last| last.saturating_add(MIN_TRIGGER_GAP_MS))
            .max(now);
        if earliest < self.next_ms {
            self.schedule(now, earliest);
        }
        true
    }

    /// An attempt starts at `now` with the `current` candidates; returns its
    /// number since the relay became the fallback.
    pub(in crate::share) fn started(&mut self, now: u64, current: Candidates) -> u32 {
        self.attempts = self.attempts.saturating_add(1);
        self.last_ms = Some(now);
        self.candidates = current;
        let cause = self.pending.take().unwrap_or(RetryCause::Backoff);
        self.last_cause = Some(cause);
        self.delay_ms = match cause {
            RetryCause::Backoff => self
                .delay_ms
                .saturating_mul(2)
                .clamp(FIRST_RETRY_MS, MAX_RETRY_MS),
            RetryCause::LocalAddresses | RetryCause::PeerAddresses => FIRST_RETRY_MS,
        };
        self.schedule(now, now.saturating_add(self.delay_ms));
        self.attempts
    }

    pub(in crate::share) fn attempts(&self) -> u32 {
        self.attempts
    }

    pub(in crate::share) fn last_cause(&self) -> Option<RetryCause> {
        self.last_cause
    }

    pub(in crate::share) fn revision(&self) -> u64 {
        self.revision
    }

    /// The route reason while retries are armed: the next attempt's delay as
    /// scheduled and its number, then the reason of the fallback (`base`).
    pub(in crate::share) fn reason(&self, base: &str) -> Option<String> {
        if !self.armed {
            return None;
        }
        let seconds = self
            .next_ms
            .saturating_sub(self.scheduled_ms)
            .div_ceil(1_000);
        Some(format!(
            "Relay als Rückfall; neuer Direktversuch in {seconds} s (Versuch {}); {base}",
            self.attempts.saturating_add(1)
        ))
    }

    fn schedule(&mut self, now: u64, next: u64) {
        self.scheduled_ms = now;
        self.next_ms = next;
        self.revision = self.revision.saturating_add(1);
    }
}

#[cfg(test)]
#[path = "route_retry_tests.rs"]
mod tests;
