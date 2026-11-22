//! The capture tick of one host session (plan 2026-11-14 „Bildkette“ B-08):
//! one tick per frame interval of the viewer's highest rate; the pipeline's
//! frame clock decides which ticks take a picture. Where the platform has a
//! precise session timer (Windows: a high-resolution waitable timer with the
//! system timer resolution raised for the session,
//! `os::timer_resolution::SessionTimer`) it drives the tick; elsewhere the
//! runtime's interval does. Both skip ticks missed while a picture operation
//! ran instead of bursting.
use std::time::Duration;

use crate::screen::os::timer_resolution::SessionTimer;

enum Ticks {
    Runtime(tokio::time::Interval),
    Precise(SessionTimer),
}

pub(super) struct SessionClock {
    ticks: Ticks,
}

/// One frame interval at `max_fps` (at least one frame per second).
pub(super) fn tick_interval(max_fps: u32) -> Duration {
    Duration::from_micros(1_000_000 / u64::from(max_fps.max(1)))
}

impl SessionClock {
    /// Starts the tick of a session whose viewer wants at most `max_fps`.
    pub(super) fn start(max_fps: u32) -> Self {
        let tick = tick_interval(max_fps);
        let ticks = match SessionTimer::start(tick) {
            Some(timer) => Ticks::Precise(timer),
            None => {
                let mut interval = tokio::time::interval(tick);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                Ticks::Runtime(interval)
            }
        };
        Self { ticks }
    }

    /// Whether the platform's precise timer drives the tick.
    #[cfg(test)]
    pub(super) fn precise(&self) -> bool {
        matches!(self.ticks, Ticks::Precise(_))
    }

    /// The next tick; cancel-safe, so it can wait inside `select!`.
    pub(super) async fn tick(&mut self) {
        match &mut self.ticks {
            Ticks::Runtime(interval) => {
                interval.tick().await;
            }
            Ticks::Precise(timer) => timer.tick().await,
        }
    }
}

/// Whether a capture tick was missed: it started no picture operation
/// because the previous one still ran.
pub(super) fn tick_missed(started_before: u64, started_after: u64, working: bool) -> bool {
    started_before == started_after && working
}

#[cfg(test)]
mod pacing_tests {
    use super::tick_missed;

    /// A tick that found the previous operation running is made up as soon
    /// as it ends; a tick that started one, or found nothing to do (paused,
    /// hidden, waiting for a visibility acknowledgement), is not.
    #[test]
    fn screen_metadata_runtime_task_missed_capture_tick_is_made_up_after_the_operation() {
        assert!(tick_missed(4, 4, true), "still running: wait for its end");
        assert!(
            !tick_missed(4, 5, true),
            "the tick started the next operation"
        );
        assert!(!tick_missed(4, 4, false), "nothing ran and nothing started");
    }
}
