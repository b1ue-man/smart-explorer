//! Adaptive concurrency for one connection (or local volume): goodput hill
//! climbing with multiplicative decrease on overload.
//!
//! The right number of concurrent operations depends on bandwidth × latency,
//! file sizes, server capacity and the storage medium, none of which is known
//! in advance. The controller therefore measures goodput per window and keeps
//! only changes that pay off: it doubles while doubling gains ≥ 10 % (slow
//! start), then probes a step of an eighth of the limit up or down
//! periodically. A probe spans two windows and is judged against the smoothed
//! rate of the current limit: a step up stays only with ≥ 5 % more goodput, a
//! step down only when it costs < 3 %, and the next probe after a step down
//! goes up. Symmetric steps keep noise from walking the limit away from the
//! optimum in either direction (simulated with ±10 % noise: ≥ 93 % of the
//! optimal goodput at ≤ 1.11× the needed concurrency). Overload signals halve
//! the limit at once. Pure logic; the caller supplies the clock.

/// Work credited per finished operation so pure small-file load is measurable.
pub(crate) const OP_WEIGHT_BYTES: u64 = 64 * 1024;
/// Noise guard: all streams of one connection share one congestion window, so
/// beyond this many concurrent operations a connection cannot gain; each one
/// holds a worker thread and up to about a mebibyte of buffers.
pub(crate) const RESOURCE_CEILING: usize = 256;
const START_LIMIT: usize = 2;
const MIN_WINDOW_MS: u64 = 1_000;
const MAX_WINDOW_MS: u64 = 5_000;
const SLOW_START_GAIN: f64 = 1.10;
const PROBE_GAIN: f64 = 1.05;
const PROBE_KEEP_LOWER: f64 = 0.97;
/// Probe step as a fraction of the limit (at least one).
const PROBE_STEP_DIVISOR: usize = 8;
const PROBE_WINDOWS: u32 = 2;
const PROBE_INTERVAL_WINDOWS: u32 = 4;
const OVERLOAD_HOLD_WINDOWS: u32 = 3;
const SATURATED_FRACTION: f64 = 0.5;

/// How an operation ended, as far as concurrency control is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpOutcome {
    /// Completed; its work counts toward goodput.
    Done,
    /// The peer signalled overload (rate limit, too many requests, timeout).
    Overload,
    /// Failed or canceled for another reason; no work is credited.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    SlowStart,
    Steady,
}

/// A running probe: the limit it left, the rate it must beat and the
/// windows measured so far.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Probe {
    upward: bool,
    from: usize,
    baseline: f64,
    windows: u32,
    total: f64,
}

#[derive(Debug)]
pub(crate) struct FlowControl {
    limit: usize,
    ceiling: usize,
    phase: Phase,
    probe: Option<Probe>,
    /// Smoothed goodput at the current limit (steady phase).
    baseline: Option<f64>,
    window_start_ms: u64,
    last_tick_ms: u64,
    work: u64,
    saturated_ms: u64,
    latency_ewma_ms: f64,
    previous: Option<(usize, f64)>,
    hold: u32,
    idle_windows: u32,
    probe_up_next: bool,
    last_decrease_window: Option<u64>,
}

impl FlowControl {
    pub(crate) fn new(ceiling: usize, now_ms: u64) -> Self {
        let ceiling = ceiling.clamp(1, RESOURCE_CEILING);
        Self {
            limit: START_LIMIT.min(ceiling),
            ceiling,
            phase: Phase::SlowStart,
            probe: None,
            baseline: None,
            window_start_ms: now_ms,
            last_tick_ms: now_ms,
            work: 0,
            saturated_ms: 0,
            latency_ewma_ms: 0.0,
            previous: None,
            hold: 0,
            idle_windows: 0,
            probe_up_next: true,
            last_decrease_window: None,
        }
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn ceiling(&self) -> usize {
        self.ceiling
    }

    /// A protocol bound learned at run time (for example refused connections)
    /// or reported by the backend.
    pub(crate) fn set_ceiling(&mut self, ceiling: usize) {
        self.ceiling = ceiling.clamp(1, RESOURCE_CEILING);
        self.limit = self.limit.min(self.ceiling);
    }

    /// Accounts the time since the previous tick; `saturated` says whether
    /// demand filled the limit during that time.
    pub(crate) fn tick(&mut self, now_ms: u64, saturated: bool) {
        let elapsed = now_ms.saturating_sub(self.last_tick_ms);
        if saturated {
            self.saturated_ms = self.saturated_ms.saturating_add(elapsed);
        }
        self.last_tick_ms = self.last_tick_ms.max(now_ms);
        if now_ms.saturating_sub(self.window_start_ms) >= self.window_len() {
            self.evaluate(now_ms);
        }
    }

    pub(crate) fn progress(&mut self, bytes: u64) {
        self.work = self.work.saturating_add(bytes);
    }

    pub(crate) fn finish(&mut self, outcome: OpOutcome, latency_ms: u64, now_ms: u64) {
        let latency = latency_ms as f64;
        self.latency_ewma_ms = if self.latency_ewma_ms == 0.0 {
            latency
        } else {
            self.latency_ewma_ms * 0.8 + latency * 0.2
        };
        match outcome {
            OpOutcome::Done => self.work = self.work.saturating_add(OP_WEIGHT_BYTES),
            OpOutcome::Failed => {}
            OpOutcome::Overload => self.decrease(now_ms),
        }
    }

    fn window_len(&self) -> u64 {
        ((self.latency_ewma_ms * 2.0) as u64).clamp(MIN_WINDOW_MS, MAX_WINDOW_MS)
    }

    /// One decrease per window: a burst of simultaneous overload replies is
    /// one congestion event, not many.
    fn decrease(&mut self, now_ms: u64) {
        if self.last_decrease_window == Some(self.window_start_ms) {
            return;
        }
        self.limit = (self.limit / 2).max(1);
        self.phase = Phase::Steady;
        self.probe = None;
        self.baseline = None;
        self.previous = None;
        self.hold = OVERLOAD_HOLD_WINDOWS;
        self.idle_windows = 0;
        self.probe_up_next = true;
        self.reset_window(now_ms);
        self.last_decrease_window = Some(self.window_start_ms);
    }

    fn reset_window(&mut self, now_ms: u64) {
        self.window_start_ms = now_ms;
        self.last_tick_ms = now_ms;
        self.work = 0;
        self.saturated_ms = 0;
    }

    fn evaluate(&mut self, now_ms: u64) {
        let span = now_ms.saturating_sub(self.window_start_ms).max(1);
        let rate = self.work as f64 * 1000.0 / span as f64;
        let saturated = self.saturated_ms as f64 >= span as f64 * SATURATED_FRACTION;
        self.reset_window(now_ms);
        if self.hold > 0 {
            self.hold -= 1;
            return;
        }
        if !saturated {
            // Demand did not fill the limit: the window says nothing about
            // more parallelism. An open upward probe is withdrawn.
            if let Some(probe) = self.probe.take() {
                if probe.upward {
                    self.limit = probe.from;
                }
            }
            return;
        }
        match self.phase {
            Phase::SlowStart => self.slow_start(rate),
            Phase::Steady => self.steady(rate),
        }
    }

    fn slow_start(&mut self, rate: f64) {
        match self.previous {
            Some((previous_limit, previous_rate)) if previous_limit < self.limit => {
                if rate >= previous_rate * SLOW_START_GAIN {
                    self.previous = Some((self.limit, rate));
                    self.limit = (self.limit * 2).min(self.ceiling);
                } else {
                    self.limit = previous_limit;
                    self.enter_steady();
                    return;
                }
            }
            _ => {
                self.previous = Some((self.limit, rate));
                self.limit = (self.limit * 2).min(self.ceiling);
            }
        }
        if self
            .previous
            .is_some_and(|(limit, _)| limit >= self.ceiling)
        {
            self.limit = self.ceiling;
            self.enter_steady();
        }
    }

    fn enter_steady(&mut self) {
        self.phase = Phase::Steady;
        self.previous = None;
        self.probe = None;
        self.baseline = None;
        self.idle_windows = 0;
        self.hold = 1;
    }

    fn steady(&mut self, rate: f64) {
        if let Some(mut probe) = self.probe.take() {
            probe.windows += 1;
            probe.total += rate;
            if probe.windows < PROBE_WINDOWS {
                self.probe = Some(probe);
                return;
            }
            let mean = probe.total / f64::from(probe.windows);
            self.idle_windows = 0;
            if probe.upward {
                if mean >= probe.baseline * PROBE_GAIN {
                    // Paid off: keep it and probe upward again right away.
                    self.baseline = Some(mean);
                    self.idle_windows = PROBE_INTERVAL_WINDOWS;
                    self.probe_up_next = true;
                } else {
                    self.limit = probe.from;
                    self.probe_up_next = false;
                }
            } else {
                if mean >= probe.baseline * PROBE_KEEP_LOWER {
                    // Nearly the same goodput with less concurrency: keep it.
                    self.baseline = Some(mean);
                } else {
                    self.limit = probe.from;
                }
                // Up next either way, so noise cannot ratchet the limit down.
                self.probe_up_next = true;
            }
            return;
        }
        self.baseline = Some(match self.baseline {
            Some(baseline) => baseline * 0.5 + rate * 0.5,
            None => rate,
        });
        self.idle_windows += 1;
        if self.idle_windows < PROBE_INTERVAL_WINDOWS {
            return;
        }
        self.idle_windows = 0;
        let baseline = self.baseline.unwrap_or(rate);
        let step = (self.limit / PROBE_STEP_DIVISOR).max(1);
        if self.probe_up_next && self.limit < self.ceiling {
            self.probe = Some(Probe {
                upward: true,
                from: self.limit,
                baseline,
                windows: 0,
                total: 0.0,
            });
            self.limit = (self.limit + step).min(self.ceiling);
        } else if self.limit > 1 {
            self.probe = Some(Probe {
                upward: false,
                from: self.limit,
                baseline,
                windows: 0,
                total: 0.0,
            });
            self.limit = self.limit.saturating_sub(step).max(1);
        }
        self.probe_up_next = !self.probe_up_next;
    }
}

#[cfg(test)]
#[path = "flow_control_tests.rs"]
mod tests;
