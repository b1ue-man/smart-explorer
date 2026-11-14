//! One finite, platform-independent LAN → direct → relay decision per generation.
//! Clock and transport facts are inputs; no address hint grants identity or rights.
//! A validated relay carries payload from the start until the first LAN/direct
//! admission (`route_start.rs`); the finite direct phase only decides when the
//! relay becomes the marked fallback.
use super::route_diagnostics::{milliseconds, RouteBudget};
use super::route_state::{RouteClass, RoutePhase, RouteStatus};
use std::collections::BTreeSet;
use std::time::Duration;
/// Cap on candidates admitted during the collection window. LAN paths are
/// exempt (see `observe`); refused candidates are counted in
/// `RouteBudget::dropped_candidates`.
const MAX_FROZEN_CANDIDATES: usize = 128;

#[cfg(test)]
#[path = "batches_route_task_tests.rs"]
pub mod batches_route_task_tests;

#[cfg(test)]
#[path = "unbelegt_route_cap_task_tests.rs"]
mod unbelegt_route_cap_task_tests;

#[path = "route_start.rs"]
pub(super) mod route_start;

#[derive(Clone, Debug)]
pub struct RouteCandidate {
    pub key: String,
    /// Stable remote candidate identity, shared by its hint and opened paths.
    pub snapshot_key: String,
    pub class: RouteClass,
    pub validated: bool,
    pub validation_pending: bool,
    pub rtt: Option<Duration>,
    /// The path the transport selected (local PATH_STATUS Available, A7):
    /// orders validated paths of the same class, so the app admits the path
    /// the transport keeps and the redundant ones can be released.
    pub preferred: bool,
}

#[derive(Clone, Debug)]
pub struct RoundFacts {
    pub round: String,
    pub candidates: Vec<String>,
    pub exhausted: bool,
    pub response_deadline_ms: Option<u64>,
    pub pending_path_opens: usize,
    pub retry_intervals: Vec<Duration>,
    pub max_probe_attempts: u8,
}

pub struct RoutePolicy {
    status: RouteStatus,
    budget: RouteBudget,
    collection_until: u64,
    direct_until: u64,
    frozen: bool,
    candidates: BTreeSet<String>,
    lan_candidates: BTreeSet<String>,
    initial_round: Option<String>,
    /// A refused round candidate keeps the initial round open (`round_completed`).
    round_refused: bool,
    chosen: Option<String>,
    had_route: bool,
    start: route_start::StartPhase,
}

impl RoutePolicy {
    pub fn new(generation: u64, initial_pto: Duration, best_pto: Duration) -> Self {
        let wait = milliseconds(initial_pto.max(best_pto).saturating_mul(3));
        Self {
            status: RouteStatus::initial(generation),
            budget: RouteBudget {
                initial_pto_ms: milliseconds(initial_pto),
                best_pto_ms: milliseconds(best_pto),
                collection_ms: wait,
                response_ms: wait,
                validation_ms: wait,
                ..RouteBudget::default()
            },
            collection_until: wait,
            direct_until: wait,
            frozen: false,
            candidates: BTreeSet::new(),
            lan_candidates: BTreeSet::new(),
            initial_round: None,
            round_refused: false,
            chosen: None,
            had_route: false,
            start: route_start::StartPhase::default(),
        }
    }

    pub fn status(&self) -> &RouteStatus {
        &self.status
    }
    pub fn budget(&self) -> &RouteBudget {
        &self.budget
    }

    pub fn resuming(&mut self) {
        self.had_route = true;
    }

    /// Whether the time-bounded part of this generation is over: candidates are
    /// frozen and the absolute direct deadline has passed. From then on the
    /// decision changes only with the observed transport facts, never with time.
    pub fn decided(&self, now_ms: u64) -> bool {
        self.frozen && now_ms >= self.direct_until
    }

    pub fn include_validation_pto(&mut self, pto: Duration) {
        if !self.frozen {
            let wait = milliseconds(pto.saturating_mul(3));
            self.budget.validation_ms = self.budget.validation_ms.max(wait);
            self.budget.response_ms = self.budget.response_ms.max(wait);
        }
    }

    pub fn observe(
        &mut self,
        now_ms: u64,
        paths: &[RouteCandidate],
        round: Option<&RoundFacts>,
    ) -> Option<String> {
        if !self.frozen {
            // LAN paths come first and are never refused: the LAN phase needs all of them.
            for candidate in paths.iter().filter(|p| p.class == RouteClass::Lan) {
                self.candidates.insert(candidate.snapshot_key.clone());
                self.lan_candidates.insert(candidate.snapshot_key.clone());
            }
            // Refused remote candidates of this observation, each once: open paths and
            // hints of one remote share its snapshot key, and a hinted round candidate
            // arrives both as path and as round entry (`route_adapter.rs`).
            let mut refused: BTreeSet<&str> = BTreeSet::new();
            for candidate in paths
                .iter()
                .filter(|p| p.class != RouteClass::Relay && p.class != RouteClass::Lan)
            {
                if self.candidates.contains(&candidate.snapshot_key)
                    || self.candidates.len() < MAX_FROZEN_CANDIDATES
                {
                    self.candidates.insert(candidate.snapshot_key.clone());
                } else {
                    refused.insert(candidate.snapshot_key.as_str());
                }
            }
            if let Some(round) = round {
                if self.initial_round.as_deref() != Some(round.round.as_str()) {
                    self.round_refused = false;
                }
                for key in &round.candidates {
                    if self.candidates.contains(key) {
                        continue;
                    }
                    if self.candidates.len() < MAX_FROZEN_CANDIDATES {
                        self.candidates.insert(key.clone());
                    } else {
                        refused.insert(key.as_str());
                        self.round_refused = true;
                    }
                }
                self.initial_round = Some(round.round.clone());
                self.budget.retry_intervals_ms = round
                    .retry_intervals
                    .iter()
                    .copied()
                    .map(milliseconds)
                    .collect();
                self.budget.max_probe_attempts = round.max_probe_attempts;
            }
            self.budget.dropped_candidates = refused.len();
            if now_ms >= self.collection_until {
                self.frozen = true;
                self.budget.frozen_candidates = self.candidates.len();
                let retries: u64 = self
                    .budget
                    .retry_intervals_ms
                    .iter()
                    .copied()
                    .fold(0, u64::saturating_add);
                self.direct_until = self
                    .collection_until
                    .saturating_add(retries)
                    .saturating_add(self.budget.response_ms)
                    .saturating_add(self.budget.validation_ms);
                // No NAT capability still retains ordinary direct path validation.
                self.budget.direct_deadline_ms = self.direct_until;
            }
        }
        let expired = self.frozen && now_ms >= self.direct_until;
        let direct_done = self.frozen
            && (expired
                || (!paths.iter().any(|path| path.validation_pending)
                    && self.round_completed(now_ms, round)));
        let lan_done = self.frozen && (self.lan_candidates.is_empty() || direct_done);
        let progress = route_start::Progress {
            lan_done,
            direct_done,
            expired,
        };
        if let Some(choice) = self.start.choose(paths, progress) {
            let key = choice.path.key.clone();
            self.set_state(choice.phase, Some(choice.path.class), true, choice.reason);
            self.chosen = Some(key);
            self.had_route = true;
        } else {
            let phase = if direct_done {
                RoutePhase::Offline
            } else if self.had_route {
                RoutePhase::Paused
            } else if !self.frozen {
                RoutePhase::Collecting
            } else if !lan_done {
                RoutePhase::CheckingLan
            } else {
                RoutePhase::CheckingDirect
            };
            self.set_state(phase, None, false, match phase {
                RoutePhase::Paused => "Nutzdaten pausieren bis zur erneuten Pfadzulassung; kein Operationsreplay",
                RoutePhase::Offline => "Kein validierter Pfad im aktuellen Versuch; spätere Pfade werden weiter geprüft",
                RoutePhase::Collecting => "Begrenztes Sammelfenster für Routingkandidaten",
                RoutePhase::CheckingLan => "Bekannte LANkandidaten werden geprüft",
                _ => "Endliche Direktphase läuft", });
            self.chosen = None;
        }
        self.chosen.clone()
    }

    fn round_completed(&self, now: u64, round: Option<&RoundFacts>) -> bool {
        let Some(round) = round else {
            return false;
        };
        // Later upgrade rounds neither end nor extend the frozen initial round.
        // A round that refused a candidate of the cap never probed that candidate.
        self.initial_round.as_deref() == Some(round.round.as_str())
            && !self.round_refused
            && round.exhausted
            && round.pending_path_opens == 0
            && round
                .response_deadline_ms
                .is_some_and(|deadline| now >= deadline.saturating_add(self.budget.validation_ms))
            && self
                .candidates
                .iter()
                .all(|candidate| round.candidates.contains(candidate))
    }

    pub fn gate_rejected(&mut self) {
        self.start.rejected();
        self.chosen = None;
        self.set_state(
            RoutePhase::Paused,
            None,
            false,
            "Pfadbeobachtung wurde verworfen; erneute Prüfung läuft",
        );
    }

    fn set_state(
        &mut self,
        phase: RoutePhase,
        class: Option<RouteClass>,
        allowed: bool,
        reason: &str,
    ) {
        if self.status.application_allowed && !allowed {
            self.status.pause_epoch = self.status.pause_epoch.saturating_add(1);
        }
        if (
            self.status.phase,
            self.status.class,
            self.status.application_allowed,
            self.status.reason.as_str(),
        ) != (phase, class, allowed, reason)
        {
            self.status.revision = self.status.revision.saturating_add(1);
            self.status.phase = phase;
            self.status.class = class;
            self.status.application_allowed = allowed;
            self.status.reason = reason.into();
        }
    }
}
