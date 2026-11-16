//! Target bit rate of a picture stream (decision C3 „Staukontrolle und
//! Bitrate“, after GCC, draft-ietf-rmcat-gcc-02; plan 2026-11-14
//! „Bildkette“ B-02, B-05, B-06, B-09). QUIC's Cubic stays underneath. Two
//! controllers, the target is the smaller of both:
//! - delay-based from `FRAME_ACK`: slope of the accumulated delay variation
//!   over the last 20 frames; persistent overuse sets the target to 0.85 ×
//!   receive rate, otherwise it probes by 1.25 per max(RTT, frame interval),
//!   at the start by 2 until the first overuse (`rate_probe`, B-05);
//! - loss-based from the difference of two cumulative `Connection::stats()`
//!   readings a second apart: loss above 10 % multiplies by (1 − 0.5 p),
//!   2–10 % holds; below 2 % releases the loss ceiling.
//!
//! While the sender is application-limited (it offers less than half the
//! target) and the transport shows no back pressure, the receive rate is a
//! measure of the content, not of the path: it neither caps the growth nor
//! sets the target after an overuse (plan 5.3a). The target only sets the
//! quality of the encoder, never the frame rate (`rate_cadence`, B-02). Its
//! ceiling is the smaller of what the codec pairing reports
//! (`rate_cadence::codec_ceiling_bps`, B-06) and the user's „Höchste
//! Datenrate“ (`set_ceiling`, B-09).
use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

#[path = "rate_cadence.rs"]
mod cadence;
pub use cadence::{
    adjust_quality, avc_level_max_bitrate_bps, avc_max_bitrate_bps, cadence_fps, codec_ceiling_bps,
    quality_for, EncodeTiming, Quality, BITS_PER_PIXEL, ENCODE_WINDOW_US, MIN_LADDER_FPS,
    MIN_LONG_EDGE,
};
#[path = "rate_probe.rs"]
pub mod probe;

/// Smallest target (C2 „Grenzen“, A1).
pub const MIN_BITRATE_BPS: u64 = 150_000;
/// H.264 level 5.1 MaxBR of the Baseline and Main profiles (240 000 kbit/s,
/// ITU-T H.264 Table A-1): the clamp the H.264 encoders of this build apply
/// to the target they are given. It is not a ceiling of the rate control,
/// whose ceiling comes from the codec pairing (`codec_ceiling_bps`).
pub const MAX_BITRATE_BPS: u64 = 240_000_000;
/// Start values of the ramp without a remembered rate (A1).
pub const START_DIRECT_BPS: u64 = 6_000_000;
pub const START_RELAY_BPS: u64 = 2_000_000;
/// Frames in the delay window (C3).
pub const DELAY_WINDOW: usize = 20;
/// Fewest frames for a delay decision: half the window.
const MIN_DELAY_POINTS: usize = DELAY_WINDOW / 2;
/// Slope of the accumulated delay (ms per ms of arrival time) above which the
/// path counts as overused: the queue grows by 50 ms per second (A1).
pub const OVERUSE_SLOPE: f64 = 0.05;
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
/// A sender that offers less than this share of the target is
/// application-limited: its receive rate does not measure the path.
const APP_LIMITED_SHARE: f64 = 0.5;

/// Path of the connection, from the transport label of the Iroh connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Direct,
    Relay,
}

impl PathKind {
    pub fn start_bps(self) -> u64 {
        match self {
            Self::Direct => START_DIRECT_BPS,
            Self::Relay => START_RELAY_BPS,
        }
    }

    pub fn key(self) -> &'static str {
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
pub struct AckSample {
    pub send_us: u64,
    pub recv_us: u64,
    pub bytes: u64,
    pub keyframe: bool,
}

/// What the delay-based controller did with one acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelaySignal {
    /// Too few frames since the last reset; the target holds (it may grow
    /// after the ramp).
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
        let bits = u128::from(self.bytes - first_bytes) * 8 * 1_000_000;
        Some((bits / u128::from(span_us)).min(u128::from(u64::MAX)) as u64)
    }
}

#[derive(Clone, Debug)]
pub struct RateController {
    path: PathKind,
    delay_target: f64,
    /// `INFINITY` while the loss controller does not limit.
    loss_target: f64,
    gradient: DelayGradient,
    received: RateWindow,
    sent: RateWindow,
    last_update_us: u64,
    overuse_streak: u32,
    loss_counters: Option<(u64, u64)>,
    congestion_events: Option<u64>,
    interval_us: u64,
    encode_capacity_bps: Option<u64>,
    encode_timing: EncodeTiming,
    probe: probe::Probe,
    /// What the codec pairing carries (`codec_ceiling_bps`); `None` until the
    /// pipeline names its codec.
    codec_ceiling_bps: Option<u64>,
    /// „Höchste Datenrate“ of the viewer; `None` is „automatisch“.
    user_ceiling_bps: Option<u64>,
    /// Last frame dropped before encoding or congestion event of the path.
    pressure_us: Option<u64>,
    /// The last loss reading was at least `LOSS_LOW`.
    loss_pressure: bool,
    /// The target the path last carried (`rate_probe`: `sustained_bps`).
    sustained: Option<f64>,
}

impl RateController {
    /// Starts at the start value of `path` and ramps up from there. The
    /// loss-based target only limits once loss counters arrive, so a host
    /// without them still ramps up.
    pub fn new(path: PathKind, now_us: u64) -> Self {
        Self {
            path,
            delay_target: path.start_bps() as f64,
            loss_target: f64::INFINITY,
            gradient: DelayGradient::default(),
            received: RateWindow::default(),
            sent: RateWindow::default(),
            last_update_us: now_us,
            overuse_streak: 0,
            loss_counters: None,
            congestion_events: None,
            interval_us: 33_333,
            encode_capacity_bps: None,
            encode_timing: EncodeTiming::default(),
            probe: probe::Probe::new(),
            codec_ceiling_bps: None,
            user_ceiling_bps: None,
            pressure_us: None,
            loss_pressure: false,
            sustained: None,
        }
    }

    pub fn path(&self) -> PathKind {
        self.path
    }

    fn clamp(&self, value: f64) -> f64 {
        value.clamp(MIN_BITRATE_BPS as f64, self.ceiling_bps() as f64)
    }

    fn current(&self) -> f64 {
        self.delay_target.min(self.loss_target)
    }

    /// The target for the encoder, in bits per second.
    pub fn target_bps(&self) -> u64 {
        self.clamp(self.current()).round() as u64
    }

    /// Whether the start ramp still runs (B-05).
    pub fn ramping(&self) -> bool {
        self.probe.ramping()
    }

    pub fn set_timing(&mut self, rtt_us: u64, frame_interval_us: u64) {
        self.interval_us = rtt_us.max(frame_interval_us).max(1);
    }

    /// One encoded frame: `bytes` after `elapsed_us` from capture to encoded
    /// bytes; the bit rate the encoding carries back to back.
    pub fn on_encode(&mut self, bytes: u64, elapsed_us: u64) {
        if bytes == 0 || elapsed_us == 0 {
            return;
        }
        let sample =
            (u128::from(bytes) * 8 * 1_000_000 / u128::from(elapsed_us)).min(u128::from(u64::MAX));
        let smoothed = self
            .encode_capacity_bps
            .map_or(sample, |previous| (u128::from(previous) * 7 + sample) / 8);
        self.encode_capacity_bps = Some(smoothed.min(u128::from(u64::MAX)) as u64);
    }

    /// `on_encode` of a frame that finished at `now_us`; it also counts for
    /// the encoding capacity of the frame rate (B-02).
    pub fn on_encode_at(&mut self, bytes: u64, elapsed_us: u64, now_us: u64) {
        if elapsed_us > 0 {
            self.encode_timing.push(now_us, elapsed_us);
        }
        self.on_encode(bytes, elapsed_us);
    }

    /// Frames per second the encoding of this session carries (B-02).
    pub fn encode_capacity_fps(&self) -> Option<u32> {
        self.encode_timing.capacity_fps()
    }

    pub fn on_frame_sent(&mut self, bytes: u64, now_us: u64) {
        self.sent.push(now_us, bytes);
    }

    /// A frame was dropped before encoding (queue guard, sender credit).
    pub fn on_back_pressure(&mut self, now_us: u64) {
        self.pressure_us = Some(now_us);
    }

    /// The cumulative congestion events of the path, read once a second; a
    /// rise is back pressure. A first reading only sets the baseline.
    pub fn on_congestion_events(&mut self, events: u64, now_us: u64) {
        let before = self.congestion_events.replace(events);
        if before.is_some_and(|before| events > before) {
            self.pressure_us = Some(now_us);
        }
    }

    /// Back pressure within the last rate window, or loss of at least
    /// `LOSS_LOW` in the last loss reading.
    pub fn back_pressure(&self, now_us: u64) -> bool {
        self.loss_pressure
            || self
                .pressure_us
                .is_some_and(|at| now_us.saturating_sub(at) <= RATE_WINDOW_US)
    }

    /// Whether the sender offers less than `APP_LIMITED_SHARE` of the
    /// target; `None` while the send window holds fewer than two frames.
    fn app_limited(&self) -> Option<bool> {
        let offered = self.sent.rate_bps()?;
        Some((offered as f64) < APP_LIMITED_SHARE * self.current())
    }

    /// Receive rate of the last second, from the viewer's arrival times.
    pub fn receive_rate_bps(&self) -> Option<u64> {
        self.received.rate_bps()
    }

    /// One `FRAME_ACK`. Key frames only count for the receive rate: their
    /// size shows as delay, so the delay window starts over after one.
    pub fn on_frame_acked(&mut self, sample: AckSample, now_us: u64) -> DelaySignal {
        self.received.push(sample.recv_us, sample.bytes);
        if sample.keyframe {
            self.gradient.reset();
            self.overuse_streak = 0;
            self.grow(now_us, DelaySignal::Learning);
            return DelaySignal::Learning;
        }
        let Some(slope) = self.gradient.push(sample.send_us, sample.recv_us) else {
            self.grow(now_us, DelaySignal::Learning);
            return DelaySignal::Learning;
        };
        if slope <= OVERUSE_SLOPE {
            self.overuse_streak = 0;
            self.grow(now_us, DelaySignal::Increase);
            return DelaySignal::Increase;
        }
        self.overuse_streak += 1;
        self.last_update_us = now_us;
        if self.overuse_streak < OVERUSE_PERSISTENCE {
            return DelaySignal::Hold;
        }
        let current = self.current();
        self.sent.prune(now_us);
        // An application-limited sender without back pressure did not fill
        // the path: its receive rate says nothing about the path (5.3a).
        let measured = if self.app_limited() == Some(true) && !self.back_pressure(now_us) {
            None
        } else {
            self.received.rate_bps()
        };
        let lowered = DECREASE_FACTOR * measured.map_or(current, |rate| rate as f64);
        self.delay_target = self.clamp(lowered.min(current));
        self.sustained = Some(self.delay_target);
        self.probe.on_limit();
        self.gradient.reset();
        self.overuse_streak = 0;
        DelaySignal::Decrease
    }

    /// Growth once per interval. During the ramp only with delay evidence
    /// (`Increase`) and by `RAMP_FACTOR`, after it by `INTERVAL_GROWTH`. A
    /// sender that fills the target grows to at most that factor times what
    /// was delivered and to what the encoding carries; an
    /// application-limited sender grows by the factor while the transport
    /// shows no back pressure and holds while it does.
    fn grow(&mut self, now_us: u64, signal: DelaySignal) {
        if now_us.saturating_sub(self.last_update_us) < self.interval_us {
            return;
        }
        let factor = if self.probe.ramping() {
            if signal != DelaySignal::Increase {
                return;
            }
            probe::RAMP_FACTOR
        } else {
            INTERVAL_GROWTH
        };
        self.sent.prune(now_us);
        let Some(app_limited) = self.app_limited() else {
            return;
        };
        let current = self.current();
        let mut next = current * factor;
        if !app_limited {
            let Some(delivery) = self.received.rate_bps() else {
                return;
            };
            next = next.min(delivery as f64 * factor);
            if let Some(capacity) = self.encode_capacity_bps {
                next = next.min(capacity as f64);
            }
        } else if self.back_pressure(now_us) {
            return;
        }
        self.delay_target = self.clamp(next.max(current));
        if !app_limited {
            // What delivery backs is a measurement of the path.
            self.note_delivered(next);
        }
        self.last_update_us = now_us;
    }

    /// Cumulative packet counters of the connection, read once a second.
    /// Returns the loss share of the interval; `None` for the first reading,
    /// an interval without packets, or counters that went backwards. Loss of
    /// at least `LOSS_LOW` counts as back pressure until the next reading;
    /// loss above `LOSS_HIGH` also ends the ramp.
    pub fn on_loss_counters(&mut self, sent_packets: u64, lost_packets: u64) -> Option<f64> {
        let (sent_before, lost_before) =
            self.loss_counters.replace((sent_packets, lost_packets))?;
        if sent_packets <= sent_before || lost_packets < lost_before {
            return None;
        }
        let sent = sent_packets - sent_before;
        let lost = (lost_packets - lost_before).min(sent);
        let loss = lost as f64 / sent as f64;
        let current = self.current();
        self.loss_pressure = loss >= LOSS_LOW;
        self.loss_target = if loss > LOSS_HIGH {
            self.probe.on_limit();
            self.clamp(current * (1.0 - 0.5 * loss))
        } else if loss < LOSS_LOW {
            f64::INFINITY
        } else {
            self.clamp(current)
        };
        Some(loss)
    }
}

#[cfg(test)]
#[path = "rate_control_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bildkette_rate_task_tests.rs"]
mod bildkette_rate_task_tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "batches_rate_reference_task_tests.rs"]
mod p27_acceptance;
