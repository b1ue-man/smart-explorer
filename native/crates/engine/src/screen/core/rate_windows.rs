//! The two measuring windows of the rate control (decision C3, moved out of
//! `rate_control.rs` unchanged on 2026-11-17 to keep that file below its
//! size limit): the slope of the accumulated delay variation over the last
//! frames (`DelayGradient`, GCC draft-ietf-rmcat-gcc-02 trendline) and the
//! bytes of a sliding window of one second (`RateWindow`, receive and send
//! rates).
use std::collections::VecDeque;

use super::DELAY_WINDOW;

/// Fewest frames for a delay decision: half the window.
pub(super) const MIN_DELAY_POINTS: usize = DELAY_WINDOW / 2;
/// Window of the receive and send rates.
pub(super) const RATE_WINDOW_US: u64 = 1_000_000;
/// Shortest span for a receive rate.
const MIN_RATE_SPAN_US: u64 = 1;
/// Entries of one rate window: one second at 240 frames/s, four times over.
const MAX_RATE_ENTRIES: usize = 1024;

#[derive(Clone, Debug, Default)]
pub(super) struct DelayGradient {
    points: VecDeque<(f64, f64)>,
    last: Option<(u64, u64)>,
    origin_us: u64,
    accumulated_ms: f64,
}

impl DelayGradient {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Adds one frame; the slope once enough frames are in the window. Only
    /// differences of each clock are used, so the clock offset cancels.
    pub(super) fn push(&mut self, send_us: u64, recv_us: u64) -> Option<f64> {
        match self.last {
            Some((last_send, last_recv)) => {
                let delta_us = (i128::from(recv_us) - i128::from(last_recv))
                    - (i128::from(send_us) - i128::from(last_send));
                self.accumulated_ms += delta_us as f64 / 1000.0;
            }
            None => self.origin_us = recv_us,
        }
        self.last = Some((send_us, recv_us));
        let arrival_ms = recv_us.saturating_sub(self.origin_us) as f64 / 1000.0;
        self.points.push_back((arrival_ms, self.accumulated_ms));
        if self.points.len() > DELAY_WINDOW {
            self.points.pop_front();
        }
        if self.points.len() < MIN_DELAY_POINTS {
            return None;
        }
        least_squares_slope(&self.points)
    }
}

fn least_squares_slope(points: &VecDeque<(f64, f64)>) -> Option<f64> {
    let count = points.len() as f64;
    let (sum_x, sum_y) = points
        .iter()
        .fold((0.0, 0.0), |(sum_x, sum_y), (x, y)| (sum_x + x, sum_y + y));
    let (mean_x, mean_y) = (sum_x / count, sum_y / count);
    let (mut covariance, mut variance) = (0.0, 0.0);
    for (x, y) in points {
        covariance += (x - mean_x) * (y - mean_y);
        variance += (x - mean_x) * (x - mean_x);
    }
    if variance > f64::EPSILON {
        Some(covariance / variance)
    } else {
        None
    }
}

/// Bytes over a sliding window of one second.
#[derive(Clone, Debug, Default)]
pub(super) struct RateWindow {
    entries: VecDeque<(u64, u64)>,
    bytes: u64,
}

impl RateWindow {
    pub(super) fn push(&mut self, at_us: u64, bytes: u64) {
        self.entries.push_back((at_us, bytes));
        self.bytes += bytes;
        self.prune(at_us);
    }

    pub(super) fn prune(&mut self, now_us: u64) {
        while let Some(&(at_us, bytes)) = self.entries.front() {
            if now_us.saturating_sub(at_us) <= RATE_WINDOW_US
                && self.entries.len() <= MAX_RATE_ENTRIES
            {
                break;
            }
            self.entries.pop_front();
            self.bytes -= bytes;
        }
    }

    /// Bits per second between the first and the last entry; the bytes of
    /// the first entry arrived before the span and do not count.
    pub(super) fn rate_bps(&self) -> Option<u64> {
        let (first_us, first_bytes) = *self.entries.front()?;
        let (last_us, _) = *self.entries.back()?;
        let span_us = last_us.saturating_sub(first_us);
        if span_us < MIN_RATE_SPAN_US {
            return None;
        }
        let bits = u128::from(self.bytes - first_bytes) * 8 * 1_000_000;
        Some((bits / u128::from(span_us)).min(u128::from(u64::MAX)) as u64)
    }
}
