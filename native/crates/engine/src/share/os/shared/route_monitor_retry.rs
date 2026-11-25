//! Direct retries inside the route actor (`route_monitor_actor.rs`); when an
//! attempt starts decides `route_retry.rs`.
//!
//! Only a client connection retries: only the client opens QUIC paths and
//! starts NAT traversal rounds; the actor of a server connection keeps the
//! plain fallback reason (its peer retries from the other side). An attempt
//! is one message to the transport (`DirectRetryPort::attempt`, bounded by
//! [`ATTEMPT_TIMEOUT`]) and admits nothing: a path it gets validated is
//! admitted in place by the unchanged route policy. The actor wakes for the
//! next attempt and for address changes only while retries are armed.
use super::super::route_diagnostics::{milliseconds, DirectRetryFacts, RetryCause};
use super::super::route_policy::route_retry::{Candidates, RetrySchedule};
use super::super::route_retry_port::{DirectRetryPort, RetryCandidates};
use super::super::route_state::{RouteClass, RoutePhase, RouteStatus};
use iroh::endpoint::Connection;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

/// Bound of one attempt: a message to the transport's actor of the remote.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);

/// The retry part of a route entry: the port and the facts diagnostics read.
#[derive(Default)]
pub(super) struct EntryRetry {
    port: Option<Arc<dyn DirectRetryPort>>,
    mirror: Mutex<Mirror>,
}

#[derive(Clone, Default)]
struct Mirror {
    armed: bool,
    attempts: u32,
    next_at: Option<Instant>,
    last_cause: Option<RetryCause>,
    last_nat_round: Option<bool>,
}

impl EntryRetry {
    pub(super) fn new(port: Option<Arc<dyn DirectRetryPort>>) -> Self {
        Self {
            port,
            mirror: Mutex::default(),
        }
    }

    /// The retry facts taken now; `None` while no retry is armed.
    pub(super) fn facts(&self) -> Option<DirectRetryFacts> {
        let mirror = self.mirror.lock().ok()?.clone();
        if !mirror.armed {
            return None;
        }
        let wait = mirror
            .next_at
            .map(|at| at.saturating_duration_since(Instant::now()));
        let unix_now = SystemTime::now().duration_since(UNIX_EPOCH).ok();
        Some(DirectRetryFacts {
            attempts: mirror.attempts,
            next_attempt_in_ms: wait.map(milliseconds),
            next_attempt_unix_ms: wait
                .zip(unix_now)
                .map(|(wait, now)| milliseconds(now.saturating_add(wait))),
            last_cause: mirror.last_cause,
            last_nat_round: mirror.last_nat_round,
        })
    }
}

/// Retry state of one route actor.
pub(super) struct Runner {
    port: Option<Arc<dyn DirectRetryPort>>,
    triggers: Option<watch::Receiver<u64>>,
    /// `next_event` consumed a change of `triggers`.
    triggered: bool,
    schedule: RetrySchedule,
    last_nat_round: Option<bool>,
}

impl Runner {
    pub(super) fn new(entry: &EntryRetry) -> Self {
        let port = entry.port.clone();
        let triggers = port.as_ref().map(|port| port.subscribe());
        Self {
            port,
            triggers,
            triggered: false,
            schedule: RetrySchedule::default(),
            last_nat_round: None,
        }
    }

    /// A new network generation: no retry state carries over.
    pub(super) fn reset(&mut self) {
        self.schedule.reset();
        self.last_nat_round = None;
    }

    /// One actor observation at `now` (route clock): follows `status`, starts
    /// a due attempt and shows the schedule in the reason of `status`.
    pub(super) async fn step(
        &mut self,
        entry: &EntryRetry,
        connection: &Connection,
        remote: &str,
        hints: &[SocketAddr],
        now: u64,
        status: &mut RouteStatus,
    ) {
        let Some(port) = self.port.clone() else {
            return;
        };
        let triggered = self.take_trigger();
        let fallback = connection.side().is_client()
            && status.application_allowed
            && status.phase == RoutePhase::RelayFallback
            && status.class == Some(RouteClass::Relay);
        let direct = status.application_allowed
            && matches!(status.class, Some(RouteClass::Lan | RouteClass::Direct));
        let wanted = fallback && (!self.schedule.armed() || triggered || self.schedule.due(now));
        let fresh = wanted.then(|| port.candidates(remote));
        let prints = fresh.as_ref().map(fingerprints).unwrap_or_default();
        self.schedule.follow(now, fallback, direct, prints);
        if fresh.is_some() && triggered {
            self.schedule.observe_candidates(now, prints);
        }
        if let Some(fresh) = fresh.filter(|_| self.schedule.due(now)) {
            // The addresses known at setup when the peer publishes none now.
            let peer = if fresh.peer.is_empty() {
                hints.to_vec()
            } else {
                fresh.peer
            };
            let attempt = port.attempt(connection, peer);
            let outcome = tokio::time::timeout(ATTEMPT_TIMEOUT, attempt)
                .await
                .ok()
                .flatten();
            self.last_nat_round = outcome.map(|outcome| outcome.nat_round);
            self.schedule.started(now, prints);
        }
        if let Some(reason) = self.schedule.reason(&status.reason) {
            status.reason = reason;
        }
        status.revision = status.revision.saturating_add(self.schedule.revision());
        self.mirror(entry, now);
    }

    /// Resolves when the next attempt is due or the addresses may have
    /// changed; never while no retry is armed.
    pub(super) async fn next_event(&mut self, now: u64) {
        let Some(wait) = self.schedule.wait_ms(now) else {
            return std::future::pending::<()>().await;
        };
        let wait = Duration::from_millis(wait);
        if let Some(triggers) = self.triggers.as_mut() {
            let changed = tokio::select! {
                () = tokio::time::sleep(wait) => return,
                changed = triggers.changed() => changed.is_ok(),
            };
            if changed {
                self.triggered = true;
                return;
            }
            // The trigger source ended: the backoff still runs.
            self.triggers = None;
        }
        tokio::time::sleep(wait).await;
    }

    fn take_trigger(&mut self) -> bool {
        let polled = self
            .triggers
            .as_mut()
            .is_some_and(|triggers| match triggers.has_changed() {
                Ok(true) => {
                    drop(triggers.borrow_and_update());
                    true
                }
                _ => false,
            });
        std::mem::take(&mut self.triggered) || polled
    }

    fn mirror(&self, entry: &EntryRetry, now: u64) {
        let next_at = self
            .schedule
            .wait_ms(now)
            .and_then(|wait| Instant::now().checked_add(Duration::from_millis(wait)));
        if let Ok(mut mirror) = entry.mirror.lock() {
            *mirror = Mirror {
                armed: self.schedule.armed(),
                attempts: self.schedule.attempts(),
                next_at,
                last_cause: self.schedule.last_cause(),
                last_nat_round: self.last_nat_round,
            };
        }
    }
}

fn fingerprints(candidates: &RetryCandidates) -> Candidates {
    Candidates {
        local: fingerprint(&candidates.local),
        peer: fingerprint(&candidates.peer),
    }
}

fn fingerprint(addrs: &[SocketAddr]) -> u64 {
    let mut hasher = DefaultHasher::new();
    addrs.hash(&mut hasher);
    hasher.finish()
}
