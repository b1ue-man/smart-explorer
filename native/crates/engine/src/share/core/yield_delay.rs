//! Delay samples of the yielding controller (`yield_controller.rs`, E2):
//! base and current delay after RFC 6817 §2.4.2 and §2.5. Moved out of
//! `yield_controller.rs` unchanged (2026-11-14, line limit).
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const BASE_HISTORY: usize = 10;
const BASE_INTERVAL: Duration = Duration::from_secs(60);
/// Samples of the current-delay filter (tests read it).
pub(crate) const CURRENT_FILTER: usize = 4;

/// The delay sample of one ACK frame: its round trip `latest` less the
/// reported `ack_delay`, unless that falls below `min_rtt` (RFC 9002 §5.3).
pub(crate) fn network_delay(latest: Duration, ack_delay: Duration, min_rtt: Duration) -> Duration {
    if latest >= min_rtt.saturating_add(ack_delay) {
        latest - ack_delay
    } else {
        latest
    }
}

/// Base and current delay of one path (RFC 6817 §2.4.2).
#[derive(Clone, Debug, Default)]
pub(crate) struct DelayTracker {
    /// One minimum per minute: (start of the minute, minimum).
    base: VecDeque<(Instant, Duration)>,
    /// Recent samples: (arrival, delay).
    current: VecDeque<(Instant, Duration)>,
}

impl DelayTracker {
    /// Records the delay `sample` seen at `now`; `round_trip` bounds the age
    /// of the current samples.
    pub(crate) fn record(&mut self, now: Instant, sample: Duration, round_trip: Duration) {
        match self.base.back_mut() {
            Some((start, minimum)) if now.saturating_duration_since(*start) < BASE_INTERVAL => {
                *minimum = (*minimum).min(sample);
            }
            _ => {
                self.base.push_back((now, sample));
                while self.base.len() > BASE_HISTORY {
                    self.base.pop_front();
                }
            }
        }
        self.current.push_back((now, sample));
        while self.current.len() > CURRENT_FILTER {
            self.current.pop_front();
        }
        while self.current.len() > 1
            && self
                .current
                .front()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at) > round_trip)
        {
            self.current.pop_front();
        }
    }

    /// The smallest delay of the history.
    pub(crate) fn base_delay(&self) -> Option<Duration> {
        self.base.iter().map(|(_, minimum)| *minimum).min()
    }

    /// Filtered current delay minus base delay.
    pub(crate) fn queuing_delay(&self) -> Option<Duration> {
        let current = self.current.iter().map(|(_, delay)| *delay).min()?;
        Some(current.saturating_sub(self.base_delay()?))
    }
}
