//! Small values the flow controller reports and learns from: the facts the
//! verdict of a transfer reads, the operation times of one window and the
//! start limit a learned limit gives.
use super::flow_control::START_LIMIT;

/// What the verdict of a transfer needs to know about a flow (U44-6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FlowFacts {
    /// Demand filled the limit during at least half the observed time.
    pub saturated: bool,
    /// The concurrency has stopped growing: the last upward probe brought
    /// less than 5 %, slow start ended, or the ceiling is reached. Valid
    /// until the limit grows again.
    pub plateau: bool,
}

/// The start limit a learned limit gives: half of it (the first measurement
/// replaces the hint), never below the general start.
pub(crate) fn start_from_learned(learned: usize) -> usize {
    (learned / 2).max(START_LIMIT)
}

/// Operation times of one window.
#[derive(Debug, Default)]
pub(super) struct LatencyWindow {
    sum_ms: u64,
    pub(super) count: u64,
    pub(super) min_ms: Option<u64>,
}

impl LatencyWindow {
    pub(super) fn record(&mut self, latency_ms: u64) {
        self.sum_ms = self.sum_ms.saturating_add(latency_ms);
        self.count += 1;
        self.min_ms = Some(self.min_ms.map_or(latency_ms, |min| min.min(latency_ms)));
    }

    pub(super) fn mean(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum_ms as f64 / self.count as f64)
    }
}
