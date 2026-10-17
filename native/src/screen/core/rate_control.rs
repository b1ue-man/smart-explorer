//! Target bit rate of a picture stream (decision C3 „Staukontrolle und
//! Bitrate“, after GCC, draft-ietf-rmcat-gcc-02). QUIC's Cubic stays
//! underneath. Two controllers, the target is the smaller of both:
//! - delay-based from `FRAME_ACK`: slope of the accumulated delay variation
//!   over the last 20 frames; overuse sets the target to 0.85 × receive rate,
//!   otherwise it grows by 1.08 per second;
//! - loss-based from the difference of two cumulative `Connection::stats()`
//!   readings a second apart: loss above 10 % multiplies by (1 − 0.5 p),
//!   2–10 % holds, below 2 % multiplies by 1.05.
//!
//! The quality ladder lowers the frame rate down to 10/s first, then the
//! resolution by 0.75 per step.
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
const DELAY_GROWTH_PER_SECOND: f64 = 1.08;
const LOSS_HIGH: f64 = 0.10;
const LOSS_LOW: f64 = 0.02;
const LOSS_GROWTH: f64 = 1.05;
/// Window of the receive and send rates.
const RATE_WINDOW_US: u64 = 1_000_000;
/// Shortest span for a receive rate.
const MIN_RATE_SPAN_US: u64 = 100_000;
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
        }
    }

    pub(crate) fn path(&self) -> PathKind {
        self.path
    }

    /// The target for the encoder, in bits per second.
    pub(crate) fn target_bps(&self) -> u64 {
        clamp_bps(self.delay_target.min(self.loss_target)).round() as u64
    }

    /// The path changed; moving onto the relay caps the target at the relay
    /// start value, because the relay drops above 1 MiB per destination.
    pub(crate) fn set_path(&mut self, path: PathKind) {
        if path == PathKind::Relay && self.path == PathKind::Direct {
            let cap = START_RELAY_BPS as f64;
            self.delay_target = self.delay_target.min(cap);
            self.loss_target = self.loss_target.min(cap);
        }
        self.path = path;
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
            self.last_update_us = now_us;
            return DelaySignal::Learning;
        }
        let Some(slope) = self.gradient.push(sample.send_us, sample.recv_us) else {
            self.last_update_us = now_us;
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
        let elapsed_us = now_us
            .saturating_sub(self.last_update_us)
            .min(RATE_WINDOW_US);
        self.last_update_us = now_us;
        self.sent.prune(now_us);
        let used_bps = (self.sent.bytes * 8) as f64;
        if used_bps >= APP_LIMITED_SHARE * self.delay_target {
            let growth = DELAY_GROWTH_PER_SECOND.powf(elapsed_us as f64 / 1_000_000.0);
            self.delay_target = clamp_bps(self.delay_target * growth);
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
            current * LOSS_GROWTH
        } else {
            current
        });
        Some(loss)
    }
}

/// Bits per pixel and frame H.264 needs for readable screen content (A1,
/// calibrated with `se screen bench`).
pub(crate) const BITS_PER_PIXEL: f64 = 0.05;
/// Lowest frame rate of the ladder before the resolution sinks (C3).
pub(crate) const MIN_LADDER_FPS: u32 = 10;
/// Smallest long edge of the ladder: the preview size of the group view (C6);
/// no step goes below it.
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

/// The best quality `target_bps` carries: the source size with the highest
/// frame rate down to 10/s, then 10/s with the size × 0.75 per step, at most
/// down to the last step whose long edge is still 320 pixels or more.
pub(crate) fn quality_for(target_bps: u64, source: FrameSize, max_fps: u32) -> Quality {
    let max_fps = max_fps.max(1);
    let floor_fps = MIN_LADDER_FPS.min(max_fps);
    let fps_for = |size: FrameSize| {
        (target_bps as f64 / (size.pixels().max(1) as f64 * BITS_PER_PIXEL)).floor() as u64
    };
    let fps = fps_for(source);
    if fps >= u64::from(floor_fps) {
        return Quality {
            fps: fps.min(u64::from(max_fps)) as u32,
            size: source,
        };
    }
    let mut size = source;
    loop {
        let next = size.scale(3, 4);
        if fps_for(size) >= u64::from(floor_fps)
            || next.width.max(next.height) < MIN_LONG_EDGE
            || next == size
        {
            return Quality {
                fps: floor_fps,
                size,
            };
        }
        size = next;
    }
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
