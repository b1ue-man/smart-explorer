//! Target bit rate of a picture stream (decision C3 „Staukontrolle und
//! Bitrate“, after GCC, draft-ietf-rmcat-gcc-02). QUIC's Cubic stays
//! underneath. Two controllers, the target is the smaller of both:
//! - delay-based from `FRAME_ACK`: slope of the accumulated delay variation
//!   over the last 20 frames; overuse sets the target to 0.85 × receive rate,
//!   otherwise it probes by 1.25 per max(RTT, frame interval);
//! - loss-based from the difference of two cumulative `Connection::stats()`
//!   readings a second apart: loss above 10 % multiplies by (1 − 0.5 p),
//!   2–10 % holds; below 2 % releases the loss ceiling.
//!
//! Low rates lower cadence. Source pixels stay native for the lossless repair path.
use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::pixel_convert::FrameSize;

/// Smallest target (C2 „Grenzen“, A1).
pub(crate) const MIN_BITRATE_BPS: u64 = 150_000;
/// H.264 level 5.1 MaxBR of the Baseline and Main profiles (240 000 kbit/s).
pub(crate) const MAX_BITRATE_BPS: u64 = 240_000_000;
/// Start values (A1).
pub(crate) const START_DIRECT_BPS: u64 = 6_000_000;
pub(crate) const START_RELAY_BPS: u64 = 2_000_000;
/// Frames in the delay window (C3).
pub(crate) const DELAY_WINDOW: usize = 20;
/// Fewest frames for a delay decision: half the window.
const MIN_DELAY_POINTS: usize = DELAY_WINDOW / 2;
/// Slope of the accumulated delay (ms per ms of arrival time) above which the
/// path counts as overused: the queue grows by 50 ms per second (A1).
pub(crate) const OVERUSE_SLOPE: f64 = 0.05;
/// Consecutive overuse readings before the target sinks; one jittery
/// reading does not count.
const OVERUSE_PERSISTENCE: u32 = 3;
const DECREASE_FACTOR: f64 = 0.85;
const INTERVAL_GROWTH: f64 = 1.25;
const LOSS_HIGH: f64 = 0.10;
const LOSS_LOW: f64 = 0.02;
/// Window of the receive and send rates.
const RATE_WINDOW_US: u64 = 1_000_000;
/// Shortest span for a receive rate.
const MIN_RATE_SPAN_US: u64 = 1;
/// Entries of one rate window: one second at 240 frames/s, four times over.
const MAX_RATE_ENTRIES: usize = 1024;
/// The delay-based target grows only while the encoder uses at least this
/// share of it, so a still screen does not drive it up unchecked.
const APP_LIMITED_SHARE: f64 = 0.5;

/// Path of the connection, from the transport label of the Iroh connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PathKind {
    Direct,
    Relay,
}

impl PathKind {
    pub(crate) fn start_bps(self) -> u64 {
        match self {
            Self::Direct => START_DIRECT_BPS,
            Self::Relay => START_RELAY_BPS,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Relay => "relay",
        }
    }
}

/// One acknowledged frame: host clock when its last record was handed to
/// the stream, viewer clock when its last record arrived, its size, and
/// whether it was a key frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AckSample {
    pub send_us: u64,
    pub recv_us: u64,
    pub bytes: u64,
    pub keyframe: bool,
}

/// What the delay-based controller did with one acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DelaySignal {
    /// Too few frames since the last reset; the target holds.
    Learning,
    /// No overuse; the target may grow.
    Increase,
    /// Overuse seen, not yet persistent; the target holds.
    Hold,
    /// Persistent overuse; the target sank.
    Decrease,
}

#[derive(Clone, Debug, Default)]
struct DelayGradient {
    points: VecDeque<(f64, f64)>,
    last: Option<(u64, u64)>,
    origin_us: u64,
    accumulated_ms: f64,
}

impl DelayGradient {
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Adds one frame; the slope once enough frames are in the window. Only
    /// differences of each clock are used, so the clock offset cancels.
    fn push(&mut self, send_us: u64, recv_us: u64) -> Option<f64> {
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
struct RateWindow {
    entries: VecDeque<(u64, u64)>,
    bytes: u64,
}

impl RateWindow {
    fn push(&mut self, at_us: u64, bytes: u64) {
        self.entries.push_back((at_us, bytes));
        self.bytes += bytes;
        self.prune(at_us);
    }

    fn prune(&mut self, now_us: u64) {
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
    fn rate_bps(&self) -> Option<u64> {
        let (first_us, first_bytes) = *self.entries.front()?;
        let (last_us, _) = *self.entries.back()?;
        let span_us = last_us.saturating_sub(first_us);
        if span_us < MIN_RATE_SPAN_US {
            return None;
        }
        Some((self.bytes - first_bytes) * 8 * 1_000_000 / span_us)
    }
}

fn clamp_bps(value: f64) -> f64 {
    value.clamp(MIN_BITRATE_BPS as f64, MAX_BITRATE_BPS as f64)
}

#[derive(Clone, Debug)]
pub(crate) struct RateController {
    path: PathKind,
    delay_target: f64,
    loss_target: f64,
    gradient: DelayGradient,
    received: RateWindow,
    sent: RateWindow,
    last_update_us: u64,
    overuse_streak: u32,
    loss_counters: Option<(u64, u64)>,
    interval_us: u64,
    encode_capacity_bps: Option<u64>,
}

impl RateController {
    /// Starts at the start value of `path`. The loss-based target only limits
    /// once loss counters arrive, so a host without them still ramps up.
    pub(crate) fn new(path: PathKind, now_us: u64) -> Self {
        Self {
            path,
            delay_target: path.start_bps() as f64,
            loss_target: MAX_BITRATE_BPS as f64,
            gradient: DelayGradient::default(),
            received: RateWindow::default(),
            sent: RateWindow::default(),
            last_update_us: now_us,
            overuse_streak: 0,
            loss_counters: None,
            interval_us: 33_333,
            encode_capacity_bps: None,
        }
    }

    pub(crate) fn path(&self) -> PathKind {
        self.path
    }

    /// The target for the encoder, in bits per second.
    pub(crate) fn target_bps(&self) -> u64 {
        clamp_bps(self.delay_target.min(self.loss_target)).round() as u64
    }

    /// A new path starts conservatively and discards the previous path's loss ceiling.
    pub(crate) fn set_path(&mut self, path: PathKind) {
        if path != self.path {
            self.delay_target = path.start_bps() as f64;
            self.loss_target = MAX_BITRATE_BPS as f64;
            self.gradient.reset(); self.received = RateWindow::default(); self.sent = RateWindow::default();
            self.loss_counters = None; self.overuse_streak = 0;
        }
        self.path = path;
    }

    pub(crate) fn set_timing(&mut self, rtt_us: u64, frame_interval_us: u64) {
        self.interval_us = rtt_us.max(frame_interval_us).max(1);
    }

    pub(crate) fn on_encode(&mut self, bytes: u64, elapsed_us: u64) {
        if bytes == 0 || elapsed_us == 0 { return; }
        let sample = (u128::from(bytes) * 8 * 1_000_000 / u128::from(elapsed_us)).min(u128::from(MAX_BITRATE_BPS)) as u64;
        self.encode_capacity_bps = Some(self.encode_capacity_bps.map_or(sample, |previous| (previous * 7 + sample) / 8));
    }

    pub(crate) fn on_frame_sent(&mut self, bytes: u64, now_us: u64) {
        self.sent.push(now_us, bytes);
    }

    /// Receive rate of the last second, from the viewer's arrival times.
    pub(crate) fn receive_rate_bps(&self) -> Option<u64> {
        self.received.rate_bps()
    }

    /// One `FRAME_ACK`. Key frames only count for the receive rate: their
    /// size shows as delay, so the delay window starts over after one.
    pub(crate) fn on_frame_acked(&mut self, sample: AckSample, now_us: u64) -> DelaySignal {
        self.received.push(sample.recv_us, sample.bytes);
        if sample.keyframe {
            self.gradient.reset();
            self.overuse_streak = 0;
            self.grow(now_us);
            return DelaySignal::Learning;
        }
        let Some(slope) = self.gradient.push(sample.send_us, sample.recv_us) else {
            self.grow(now_us);
            return DelaySignal::Learning;
        };
        if slope <= OVERUSE_SLOPE {
            self.overuse_streak = 0;
            self.grow(now_us);
            return DelaySignal::Increase;
        }
        self.overuse_streak += 1;
        self.last_update_us = now_us;
        if self.overuse_streak < OVERUSE_PERSISTENCE {
            return DelaySignal::Hold;
        }
        let current = self.delay_target.min(self.loss_target);
        let lowered = match self.received.rate_bps() {
            Some(rate) => DECREASE_FACTOR * rate as f64,
            None => DECREASE_FACTOR * current,
        };
        self.delay_target = clamp_bps(lowered.min(current));
        self.gradient.reset();
        self.overuse_streak = 0;
        DelaySignal::Decrease
    }

    fn grow(&mut self, now_us: u64) {
        if now_us.saturating_sub(self.last_update_us) < self.interval_us { return; }
        self.sent.prune(now_us);
        let Some(delivery) = self.received.rate_bps() else { return; };
        let Some(offered) = self.sent.rate_bps() else { return; };
        let current = self.delay_target.min(self.loss_target);
        if offered as f64 >= APP_LIMITED_SHARE * current {
            let capacity = self.encode_capacity_bps.unwrap_or(MAX_BITRATE_BPS) as f64;
            let probe = delivery as f64 * INTERVAL_GROWTH;
            self.delay_target = clamp_bps((current * INTERVAL_GROWTH).min(probe).min(capacity).max(current));
            self.last_update_us = now_us;
        }
    }

    /// Cumulative packet counters of the connection, read once a second.
    /// Returns the loss share of the interval; `None` for the first reading,
    /// an interval without packets, or counters that went backwards.
    pub(crate) fn on_loss_counters(&mut self, sent_packets: u64, lost_packets: u64) -> Option<f64> {
        let (sent_before, lost_before) =
            self.loss_counters.replace((sent_packets, lost_packets))?;
        if sent_packets <= sent_before || lost_packets < lost_before {
            return None;
        }
        let sent = sent_packets - sent_before;
        let lost = (lost_packets - lost_before).min(sent);
        let loss = lost as f64 / sent as f64;
        let current = self.delay_target.min(self.loss_target);
        self.loss_target = clamp_bps(if loss > LOSS_HIGH {
            current * (1.0 - 0.5 * loss)
        } else if loss < LOSS_LOW {
            MAX_BITRATE_BPS as f64
        } else {
            current
        });
        Some(loss)
    }
}

/// Bits per pixel and frame H.264 needs for readable screen content (A1,
/// calibrated with `se screen bench`).
pub(crate) const BITS_PER_PIXEL: f64 = 0.05;
/// V1 configuration constant retained for consumers; native stills may use lower cadence.
pub(crate) const MIN_LADDER_FPS: u32 = 10;
/// The legacy preview edge; source pixels are no longer reduced by the rate controller.
pub(crate) const MIN_LONG_EDGE: u32 = 320;
/// A better step is taken only when the target carries it 1.25 times, so
/// the ladder does not flap around one target.
const STEP_UP_HEADROOM: f64 = 1.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Quality {
    pub fps: u32,
    pub size: FrameSize,
}

impl Quality {
    fn load(self) -> u64 {
        self.size.pixels() * u64::from(self.fps)
    }
}

/// Keep source pixels and choose a cadence the current target can carry.
/// Change-driven lossless repair does not need a ten-frame-per-second pixel budget.
pub(crate) fn quality_for(target_bps: u64, source: FrameSize, max_fps: u32) -> Quality {
    let max_fps = max_fps.max(1);
    let fps = (target_bps as f64 / (source.pixels().max(1) as f64 * BITS_PER_PIXEL)).floor() as u32;
    Quality { fps: fps.clamp(1,max_fps), size: source }

}

/// The next quality from `current`: down at once when the target no longer
/// carries it, up only with 25 % headroom.
pub(crate) fn adjust_quality(
    current: Quality,
    target_bps: u64,
    source: FrameSize,
    max_fps: u32,
) -> Quality {
    let fitting = quality_for(target_bps, source, max_fps);
    if current.size.width > source.width
        || current.size.height > source.height
        || fitting.load() < current.load()
    {
        return fitting;
    }
    let headroom = (target_bps as f64 / STEP_UP_HEADROOM) as u64;
    let better = quality_for(headroom, source, max_fps);
    if better.load() > current.load() {
        better
    } else {
        current
    }
}

#[cfg(test)]
#[path = "rate_control_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "batches_rate_reference_task_tests.rs"]
mod p27_acceptance;
