//! When the capture thread of a Windows source takes its next picture (plan
//! 2026-11-14 bildkette, B-10). The thread learns the frame interval from the
//! cadence of the pipeline's `Produce` requests and asks the compositor for a
//! picture shortly before the next request is due: Desktop Duplication waits
//! in `AcquireNextFrame` and Graphics Capture for `FrameArrived` at most until
//! that request. A request then gets the newest picture the compositor
//! delivered without starting a capture itself. Pure logic without a Windows
//! call (tests `bildkette_task_win_capture_*`).
use std::time::{Duration, Instant};

/// Request gaps remembered for the interval estimate [P].
const GAPS: usize = 8;
/// Capture costs remembered for the lead time [P].
const COSTS: usize = 8;
/// Shortest frame interval the cadence yields: 1 ms, far above every display
/// refresh rate (`core/pacing.rs` caps the pipeline at 240 pictures/s).
pub(in crate::screen::os) const MIN_INTERVAL: Duration = Duration::from_millis(1);
/// Longest frame interval: the first-picture wait of a source; a slower
/// cadence means the pipeline paused, not a frame rate.
pub(in crate::screen::os) const MAX_INTERVAL: Duration =
    crate::screen::os::capture_source::FIRST_FRAME_WAIT;
/// Longest single wait of the capture thread inside one capture call, so a
/// command of the request thread (pause, reopen) waits at most this long for
/// the native objects [P: a tenth of the first-picture wait].
pub(in crate::screen::os) const MAX_BLOCK: Duration = Duration::from_millis(100);

/// The cadence of the pipeline's requests.
#[derive(Clone, Debug, Default)]
pub(in crate::screen::os) struct Cadence {
    last: Option<Instant>,
    gaps: [Option<Duration>; GAPS],
    next: usize,
}

impl Cadence {
    /// A request arrived at `at`.
    pub(in crate::screen::os) fn record(&mut self, at: Instant) {
        if let Some(last) = self.last {
            let gap = at.saturating_duration_since(last);
            // Two requests at once (a retry) say nothing about the rate.
            if gap >= MIN_INTERVAL {
                self.gaps[self.next] = Some(gap.min(MAX_INTERVAL));
                self.next = (self.next + 1) % GAPS;
            }
        }
        self.last = Some(at);
    }

    pub(in crate::screen::os) fn last(&self) -> Option<Instant> {
        self.last
    }

    /// The frame interval the pipeline asks at: the lower median of the
    /// remembered gaps (one skipped tick or one burst does not move it).
    pub(in crate::screen::os) fn interval(&self) -> Option<Duration> {
        let mut gaps: Vec<Duration> = self.gaps.iter().flatten().copied().collect();
        if gaps.is_empty() {
            return None;
        }
        gaps.sort_unstable();
        Some(gaps[(gaps.len() - 1) / 2].clamp(MIN_INTERVAL, MAX_INTERVAL))
    }
}

/// What the capture thread does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum Step {
    /// Nothing is due yet; sleep this long (interrupted by commands).
    Sleep(Duration),
    /// Ask the compositor, waiting at most this long; `due` is the request
    /// the picture is for, when the cadence is known.
    Acquire {
        wait: Duration,
        due: Option<Instant>,
    },
}

/// The schedule of the capture thread.
#[derive(Clone, Debug, Default)]
pub(in crate::screen::os) struct Schedule {
    served: Option<Instant>,
    costs: [Duration; COSTS],
    next: usize,
}

impl Schedule {
    /// One capture from the start of its read until the picture was ready.
    pub(in crate::screen::os) fn record_cost(&mut self, cost: Duration) {
        self.costs[self.next] = cost.min(MAX_INTERVAL);
        self.next = (self.next + 1) % COSTS;
    }

    /// How long before a request the read starts: the longest recent read.
    pub(in crate::screen::os) fn lead(&self) -> Duration {
        self.costs.iter().copied().max().unwrap_or_default()
    }

    /// A picture for the request due at `due` is ready; the next read is
    /// for a later request.
    pub(in crate::screen::os) fn served(&mut self, due: Instant) {
        self.served = Some(self.served.map_or(due, |served| served.max(due)));
    }

    /// The step at `now`. Without a known cadence every compositor picture
    /// is taken (the first requests of a session).
    pub(in crate::screen::os) fn step(&self, now: Instant, cadence: &Cadence) -> Step {
        let unknown = Step::Acquire {
            wait: MAX_BLOCK,
            due: None,
        };
        let (Some(last), Some(interval)) = (cadence.last(), cadence.interval()) else {
            return unknown;
        };
        let Some(mut due) = last.checked_add(interval) else {
            return unknown;
        };
        // The first request after `now` and after the one already served.
        for floor in [Some(now), self.served].into_iter().flatten() {
            if due <= floor {
                let behind = floor.saturating_duration_since(due).as_nanos();
                let cycles = behind / interval.as_nanos().max(1) + 1;
                let advanced = u32::try_from(cycles)
                    .ok()
                    .and_then(|cycles| interval.checked_mul(cycles))
                    .and_then(|ahead| due.checked_add(ahead));
                let Some(advanced) = advanced else {
                    return unknown;
                };
                due = advanced;
            }
        }
        let start = due.checked_sub(self.lead()).unwrap_or(now);
        if now < start {
            return Step::Sleep((start - now).min(MAX_BLOCK));
        }
        Step::Acquire {
            wait: due.saturating_duration_since(now).min(MAX_BLOCK),
            due: Some(due),
        }
    }
}

/// The `AcquireNextFrame` timeout of a wait, in whole milliseconds.
pub(in crate::screen::os) fn timeout_ms(wait: Duration) -> u32 {
    u32::try_from(wait.as_millis()).unwrap_or(u32::MAX)
}
