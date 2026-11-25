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
//! Plan 2026-11-17 replaces the growth rule of B-05/5.3a (`rate_model.rs`,
//! user of 2026-11-17: „av1 ist zu viel. punkt.“, „h.264 capped bei
//! 800kbit/s … nicht ok“): the start is the rate table of the stream
//! (`set_stream_start`); a sender that fills the target grows to at most
//! 1.5 × what was delivered + 10 kbit/s, an application-limited one (it
//! offers less than half the target, or fewer than two frames a second) to
//! at most the larger of the table and that bound, and a persistent overuse
//! lowers an application-limited sender to 0.85 × its target, never to the
//! rate of its content. The growth no longer stops at what the encoding
//! „carries“ (bytes per encoding time scale with the target, so that bound
//! moved with it and blocked the growth). The link estimate L (delay and
//! loss) is kept apart from the target E = min(L, user ceiling, codec
//! ceiling); the encoder is given E times the correction of its honour
//! meter (`honour`, `encoder_bps`). The target only sets the quality of the
//! encoder, never the frame rate (`rate_cadence`, B-02). Its ceiling is the
//! smaller of what the codec pairing reports (`rate_cadence::
//! codec_ceiling_bps`, B-06) and the user's „Höchste Datenrate“
//! (`set_ceiling`, B-09).
use serde::{Deserialize, Serialize};

#[path = "rate_cadence.rs"]
mod cadence;
pub use cadence::{
    adjust_quality, avc_level_max_bitrate_bps, avc_max_bitrate_bps, cadence_fps, codec_ceiling_bps,
    quality_for, EncodeTiming, Quality, BITS_PER_PIXEL, ENCODE_WINDOW_US, MIN_LADDER_FPS,
    MIN_LONG_EDGE,
};
#[path = "rate_honour.rs"]
pub mod honour;
#[path = "rate_model.rs"]
pub mod model;
#[path = "rate_probe.rs"]
pub mod probe;
#[path = "rate_queue.rs"]
pub mod queue;
#[path = "rate_windows.rs"]
mod rate_windows;
#[path = "rate_tiles.rs"]
mod tiles_budget;
#[path = "rate_unthrottled.rs"]
mod unthrottled;
pub use model::RateLimit;
use model::{GrowthFacts, INTERVAL_GROWTH};
use rate_windows::{DelayGradient, RateWindow, RATE_WINDOW_US};
pub use tiles_budget::picture_floor_bps;

/// Smallest target (C2 „Grenzen“, A1).
pub const MIN_BITRATE_BPS: u64 = 150_000;
/// H.264 level 5.1 MaxBR of the Baseline and Main profiles (240 000 kbit/s,
/// ITU-T H.264 Table A-1): the clamp the H.264 encoders of this build apply
/// to the target they are given. It is not a ceiling of the rate control,
/// whose ceiling comes from the codec pairing (`codec_ceiling_bps`).
pub const MAX_BITRATE_BPS: u64 = 240_000_000;
/// Start values of a path before the pipeline names its stream
/// (`set_stream_start`); on the relay the start of a stream is at most
/// `START_RELAY_BPS` (A1, `model::session_start_bps`).
pub const START_DIRECT_BPS: u64 = 6_000_000;
pub const START_RELAY_BPS: u64 = 2_000_000;
/// Frames in the delay window (C3).
pub const DELAY_WINDOW: usize = 20;
/// Slope of the accumulated delay (ms per ms of arrival time) above which the
/// path counts as overused: the queue grows by 50 ms per second (A1).
pub const OVERUSE_SLOPE: f64 = 0.05;
/// Consecutive overuse readings before the target sinks; one jittery
/// reading does not count.
const OVERUSE_PERSISTENCE: u32 = 3;
const LOSS_HIGH: f64 = 0.10;
const LOSS_LOW: f64 = 0.02;
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
    /// Too few frames since the last reset; the target holds (may grow).
    Learning,
    /// No overuse; the target may grow.
    Increase,
    /// Overuse seen, not yet persistent; the target holds.
    Hold,
    /// Persistent overuse; the target sank.
    Decrease,
}

#[derive(Clone, Debug)]
pub struct RateController {
    path: PathKind,
    /// The link estimate L of the delay controller.
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
    encode_timing: EncodeTiming,
    probe: probe::Probe,
    /// What the codec pairing carries; `None` until the pipeline names it.
    codec_ceiling_bps: Option<u64>,
    /// „Höchste Datenrate“ of the viewer; `None` is „automatisch“.
    user_ceiling_bps: Option<u64>,
    /// Last frame dropped before encoding or congestion event of the path.
    pressure_us: Option<u64>,
    /// The last loss reading was at least `LOSS_LOW`.
    loss_pressure: bool,
    /// The target the path last carried (`rate_probe`: `sustained_bps`).
    sustained: Option<f64>,
    /// `model::start_table_bps` of the stream (`set_stream_start`).
    table_bps: Option<u64>,
    /// A frame was sent: a new stream start no longer moves the target.
    started: bool,
    /// The last halving by a stale stream and the stale time it had.
    stale_halved: Option<(u64, u64)>,
    /// The last growth or decrease found the sender application-limited;
    /// no estimate lowers the target (`rate_unthrottled.rs`).
    app_limited_now: bool,
    unthrottled: bool,
    measured_bps: Option<u64>,
    /// The honour of the session's encoder (`honour`, plan 2026-11-17).
    honour: honour::SessionHonour,
    /// Delivery rate, round-trip floor and refusals (`queue`, P9).
    queue: queue::QueueState,
    /// The byte budget of the tiles under the target (`tiles_rate`, 2026-11-18).
    tiles: crate::screen::core::tiles_rate::SharedBudget,
    /// The control stream's bytes (`rate_tiles.rs`, review 2026-11-19) and
    /// their rate of the last window; whether it was above the ceiling.
    control: RateWindow,
    control_bps: u64,
    control_over: bool,
    /// The sound bytes of the session counted so far, when, and the sound's
    /// rate between the last two readings (`on_audio_total`).
    audio_seen: u64,
    audio_at: Option<u64>,
    audio_bps: u64,
}

impl RateController {
    /// Starts at the start value of `path` and ramps up from there; the
    /// pipeline replaces it by the start of its stream (`set_stream_start`)
    /// before the first frame. The loss-based target only limits once loss
    /// counters arrive, so a host without them still ramps up.
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
            encode_timing: EncodeTiming::default(),
            probe: probe::Probe::new(),
            codec_ceiling_bps: None,
            user_ceiling_bps: None,
            pressure_us: None,
            loss_pressure: false,
            sustained: None,
            table_bps: None,
            started: false,
            stale_halved: None,
            app_limited_now: false,
            unthrottled: false,
            measured_bps: None,
            honour: honour::SessionHonour::default(),
            queue: queue::QueueState::default(),
            tiles: Default::default(),
            control: RateWindow::default(),
            control_bps: 0,
            control_over: false,
            audio_seen: 0,
            audio_at: None,
            audio_bps: 0,
        }
    }

    pub fn path(&self) -> PathKind {
        self.path
    }

    /// Within the floor and both ceilings: the target E.
    fn clamp(&self, value: f64) -> f64 {
        value.clamp(MIN_BITRATE_BPS as f64, self.ceiling_bps() as f64)
    }

    /// Within the floor and the codec pairing's ceiling: the link estimate
    /// L, which the viewer's ceiling does not cut (it may be raised again).
    fn link_clamp(&self, value: f64) -> f64 {
        let codec = self
            .codec_ceiling_bps
            .unwrap_or(u64::MAX)
            .max(MIN_BITRATE_BPS);
        value.clamp(MIN_BITRATE_BPS as f64, codec as f64)
    }

    /// The link estimate L: the smaller of the delay and the loss target.
    fn current(&self) -> f64 {
        if self.unthrottled {
            return self.unthrottled_bps();
        }
        self.delay_target.min(self.loss_target)
    }

    /// The target E in bits per second: L within both ceilings.
    pub fn target_bps(&self) -> u64 {
        self.clamp(self.current()).round() as u64
    }

    /// The link estimate L in bits per second.
    pub fn link_bps(&self) -> u64 {
        self.link_clamp(self.current()).round() as u64
    }

    /// Whether the start ramp still runs (B-05).
    pub fn ramping(&self) -> bool {
        self.probe.ramping()
    }

    pub fn set_timing(&mut self, rtt_us: u64, frame_interval_us: u64) {
        self.interval_us = rtt_us.max(frame_interval_us).max(1);
        self.queue.set_rtt(rtt_us);
    }

    /// The stream of the pipeline: `table_bps` is `model::start_table_bps`
    /// of its size, rate and codec. Before the first frame, and while no
    /// rate was measured or remembered, it is the start of the target; it
    /// always bounds the growth of an application-limited sender.
    pub fn set_stream_start(&mut self, table_bps: u64) {
        self.table_bps = Some(table_bps.max(MIN_BITRATE_BPS));
        if !self.started && self.sustained.is_none() {
            self.delay_target = self.link_clamp(self.start_value(None) as f64);
        }
    }

    /// The start of the current path (`model::session_start_bps`).
    fn start_value(&self, remembered: Option<u64>) -> u64 {
        model::session_start_bps(
            self.table_bps,
            remembered,
            self.path == PathKind::Relay,
            self.path.start_bps(),
            u64::MAX,
        )
    }

    /// A frame finished at `now_us` after `elapsed_us` from capture to
    /// encoded bytes: the encoding capacity of the frame rate (B-02). Its
    /// size no longer bounds the growth (plan 2026-11-17).
    pub fn on_encode_at(&mut self, _bytes: u64, elapsed_us: u64, now_us: u64) {
        if elapsed_us > 0 {
            self.encode_timing.push(now_us, elapsed_us);
        }
        self.honour.on_encoded_at(now_us);
    }

    /// Frames per second the encoding of this session carries (B-02).
    pub fn encode_capacity_fps(&self) -> Option<u32> {
        self.encode_timing.capacity_fps()
    }

    pub fn on_frame_sent(&mut self, bytes: u64, now_us: u64) {
        self.started = true;
        self.sent.push(now_us, bytes);
        self.queue_on_sent(bytes, now_us);
    }

    /// A frame was dropped before encoding (queue guard, sender credit): a
    /// target above what the line delivers sinks to it (`on_queue_full`).
    pub fn on_back_pressure(&mut self, now_us: u64) {
        self.pressure_us = Some(now_us);
        self.on_queue_full(now_us);
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
    /// target; a send window of fewer than two frames offers nothing.
    fn app_limited(&self) -> bool {
        let target = self.target_bps() as f64;
        self.sent
            .rate_bps()
            .is_none_or(|offered| (offered as f64) < APP_LIMITED_SHARE * target)
    }

    /// Receive rate of the last second, from the viewer's arrival times.
    pub fn receive_rate_bps(&self) -> Option<u64> {
        self.received.rate_bps()
    }

    /// One `FRAME_ACK`. Key frames only count for the receive rate: their
    /// size shows as delay, so the delay window starts over after one.
    pub fn on_frame_acked(&mut self, sample: AckSample, now_us: u64) -> DelaySignal {
        self.received.push(sample.recv_us, sample.bytes);
        self.queue.on_acked(sample.send_us, now_us);
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
        self.sent.prune(now_us);
        let app_limited = self.app_limited();
        self.app_limited_now = app_limited;
        // An application-limited sender did not fill the path: its receive
        // rate measures its content, not the path (plan 2026-11-17).
        let lowered = model::decreased(
            self.target_bps() as f64,
            app_limited,
            self.received.rate_bps(),
        );
        self.delay_target = self.link_clamp(lowered);
        self.sustained = Some(self.delay_target);
        // The overuse is back pressure for one rate window: an
        // application-limited sender does not grow straight back.
        self.pressure_us = Some(now_us);
        self.tiles.on_congestion(now_us);
        self.probe.on_limit();
        self.gradient.reset();
        self.overuse_streak = 0;
        DelaySignal::Decrease
    }

    /// Growth once per interval (`model::grown`). During the ramp only with
    /// delay evidence (`Increase`) and by `RAMP_FACTOR`, after it by
    /// `INTERVAL_GROWTH`.
    fn grow(&mut self, now_us: u64, signal: DelaySignal) {
        if now_us.saturating_sub(self.last_update_us) < self.interval_us {
            return;
        }
        // A frame the queue could not take (P9): the sender already has
        // more than the line carries.
        if self.growth_held(now_us) {
            return;
        }
        let ramping = self.probe.ramping();
        if ramping && signal != DelaySignal::Increase {
            return;
        }
        let factor = if ramping {
            probe::RAMP_FACTOR
        } else {
            INTERVAL_GROWTH
        };
        self.sent.prune(now_us);
        let app_limited = self.app_limited();
        self.app_limited_now = app_limited;
        let facts = GrowthFacts {
            current: self.current(),
            factor,
            ramping,
            app_limited,
            delivered: self.received.rate_bps(),
            table: self.table_bps,
            back_pressure: self.back_pressure(now_us),
        };
        let Some(next) = model::grown(facts) else {
            return;
        };
        self.delay_target = self.link_clamp(next);
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
        let current = self.target_bps() as f64;
        self.loss_pressure = loss >= LOSS_LOW;
        self.tiles.set_loss_limited(loss > LOSS_HIGH);
        self.loss_target = if loss > LOSS_HIGH {
            self.probe.on_limit();
            self.link_clamp(current * (1.0 - 0.5 * loss))
        } else if loss < LOSS_LOW {
            f64::INFINITY
        } else {
            self.link_clamp(current)
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

#[cfg(test)]
#[path = "bildkette_rate_model_task_tests.rs"]
mod bildkette_rate_model_task_tests;

#[cfg(test)]
#[path = "bildkette_rate_honour_task_tests.rs"]
mod bildkette_rate_honour_task_tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "batches_rate_reference_task_tests.rs"]
mod p27_acceptance;
