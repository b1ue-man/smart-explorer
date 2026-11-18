//! What the line delivers and how much of a picture stream may wait for it
//! (plan 2026-11-17 P9, user of 2026-11-17: a window moved in circles with
//! AV1 made „einen massiv steigenden Datenstrom, der sich aufzustauen
//! scheint, bis die Bildübertragung hängt“, and the lossless tiles stayed
//! at about 5 Mbit/s where the line carried 84). Only numbers and
//! decisions on the host's clock: no connection, no encoder.
//!
//! - Delivery rate (`DeliveryRate`): the rate sample of BBR
//!   (draft-cheng-iccrg-delivery-rate-estimation-02, section 3: the bytes
//!   delivered between the send of a frame and its acknowledgement over the
//!   larger of the send and the acknowledgement interval), one sample per
//!   acknowledged frame, all on the host's clock (`SentFrame::sent_us`, the
//!   arrival of `FRAME_ACK`). A sample is never larger than what the line
//!   carried, so the largest sample of `DELIVERY_WINDOW_US` estimates the
//!   line. A frame sent while an earlier one was still unacknowledged
//!   (`busy`) measures the line itself, not the content: only those samples
//!   may lower the target (`link_bps`).
//! - Queue guard (`queue_cap_bytes`): the frames in flight may hold the
//!   line's rate over the smallest round trip of `RTT_FLOOR_WINDOW_US`
//!   plus `QUEUE_FRAMES` frames: the one on the line and the next one. A
//!   sender therefore queues at most about one frame beyond what the line
//!   drains, and two frames always fit, so a frame larger than the
//!   bandwidth-delay product (a lossless picture) is sent while the next one
//!   is encoded instead of one frame per round trip (stop and wait).
//!   Before, the guard was the target over the round trip plus 100 ms: a
//!   target above the line queued 100 ms and more of it, and a lossless
//!   frame larger than that guard waited for the acknowledgement of the
//!   previous one, every frame.
use std::collections::VecDeque;

use super::PathKind;
use crate::screen::core::pacing::{RELAY_GUARD_BYTES, STALE_STREAM_US};

/// Window of the largest delivery sample.
pub const DELIVERY_WINDOW_US: u64 = 2_000_000;
/// Window of the smallest round trip (BBR keeps its minimum for 10 s).
pub const RTT_FLOOR_WINDOW_US: u64 = 10_000_000;
/// Shortest interval a delivery sample may span: below it the clocks'
/// resolution decides the rate.
pub const MIN_SAMPLE_US: u64 = 1_000;
/// Frames the guard allows beyond the bandwidth-delay product.
pub const QUEUE_FRAMES: u64 = 2;
/// Frames one window keeps at most: one second at 240 frames/s, four times
/// over, as `RateWindow`.
const MAX_ENTRIES: usize = 1024;

/// `QUEUE_FRAMES` frames of `frame_bytes`; on the relay at most
/// `RELAY_GUARD_BYTES` (the relay drops above its queue per destination).
pub fn frames_allowance(frame_bytes: u64, path: PathKind) -> u64 {
    let frames = frame_bytes.saturating_mul(QUEUE_FRAMES);
    match path {
        PathKind::Direct => frames,
        PathKind::Relay => frames.min(RELAY_GUARD_BYTES),
    }
}

/// Unacknowledged bytes the queue guard allows: `rate_bps` over the round
/// trip `base_rtt_us` (at most `STALE_STREAM_US`) plus `QUEUE_FRAMES`
/// frames of `frame_bytes`; on the relay at most `RELAY_GUARD_BYTES`.
pub fn queue_cap_bytes(rate_bps: u64, base_rtt_us: u64, frame_bytes: u64, path: PathKind) -> u64 {
    let window_us = u128::from(base_rtt_us.min(STALE_STREAM_US));
    let bdp = (u128::from(rate_bps) * window_us / 8 / 1_000_000).min(u128::from(u64::MAX)) as u64;
    let cap = bdp.saturating_add(frames_allowance(frame_bytes, path));
    match path {
        PathKind::Direct => cap,
        PathKind::Relay => cap.min(RELAY_GUARD_BYTES),
    }
}

/// The largest (`Extremum::max`) or smallest (`Extremum::min`) value of a
/// sliding time window, as a monotone queue: every value is pushed once
/// and leaves once.
#[derive(Clone, Debug)]
pub struct Extremum {
    entries: VecDeque<(u64, u64)>,
    window_us: u64,
    largest: bool,
}

impl Extremum {
    pub fn max(window_us: u64) -> Self {
        Self {
            entries: VecDeque::new(),
            window_us,
            largest: true,
        }
    }

    pub fn min(window_us: u64) -> Self {
        Self {
            entries: VecDeque::new(),
            window_us,
            largest: false,
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// `value` measured at `at_us`.
    pub fn push(&mut self, at_us: u64, value: u64) {
        while let Some(&(_, last)) = self.entries.back() {
            let dominated = if self.largest {
                last <= value
            } else {
                last >= value
            };
            if !dominated {
                break;
            }
            self.entries.pop_back();
        }
        self.entries.push_back((at_us, value));
        while self.entries.len() > MAX_ENTRIES
            || self
                .entries
                .front()
                .is_some_and(|&(at, _)| at_us.saturating_sub(at) > self.window_us)
        {
            self.entries.pop_front();
        }
    }

    /// The extremum of the values measured within the window before `now_us`.
    pub fn value(&self, now_us: u64) -> Option<u64> {
        self.entries
            .iter()
            .find(|&&(at, _)| now_us.saturating_sub(at) <= self.window_us)
            .map(|&(_, value)| value)
    }
}

/// One frame handed to the picture stream, with the delivery state at its
/// send (BBR: `P.delivered`, `P.delivered_time`, `P.first_sent_time`).
#[derive(Clone, Copy, Debug)]
struct Stamp {
    sent_us: u64,
    bytes: u64,
    delivered: u64,
    delivered_us: u64,
    first_sent_us: u64,
    /// An earlier frame was still unacknowledged when this one was sent.
    busy: bool,
}

/// The delivery rate of one picture stream's path.
#[derive(Clone, Debug)]
pub struct DeliveryRate {
    delivered: u64,
    delivered_us: u64,
    first_sent_us: u64,
    stamps: VecDeque<Stamp>,
    all: Extremum,
    busy: Extremum,
}

impl Default for DeliveryRate {
    fn default() -> Self {
        Self {
            delivered: 0,
            delivered_us: 0,
            first_sent_us: 0,
            stamps: VecDeque::new(),
            all: Extremum::max(DELIVERY_WINDOW_US),
            busy: Extremum::max(DELIVERY_WINDOW_US),
        }
    }
}

impl DeliveryRate {
    /// A frame of `bytes` was handed to the stream at `sent_us`.
    pub fn on_sent(&mut self, bytes: u64, sent_us: u64) {
        let busy = !self.stamps.is_empty();
        if !busy {
            // Nothing in flight: the delivery starts with this frame.
            self.first_sent_us = sent_us;
            self.delivered_us = sent_us;
        }
        self.stamps.push_back(Stamp {
            sent_us,
            bytes,
            delivered: self.delivered,
            delivered_us: self.delivered_us,
            first_sent_us: self.first_sent_us,
            busy,
        });
        if self.stamps.len() > MAX_ENTRIES {
            self.stamps.pop_front();
        }
    }

    /// The frame sent at `sent_us` was acknowledged at `now_us`: its rate
    /// sample in bits per second, `None` for a frame this meter does not
    /// know or an interval below `MIN_SAMPLE_US`. Earlier frames that were
    /// never acknowledged (a reset stream) leave without counting.
    pub fn on_acked(&mut self, sent_us: u64, now_us: u64) -> Option<u64> {
        let index = self
            .stamps
            .iter()
            .position(|stamp| stamp.sent_us == sent_us)?;
        for _ in 0..index {
            self.stamps.pop_front();
        }
        let stamp = self.stamps.pop_front()?;
        self.delivered = self.delivered.saturating_add(stamp.bytes);
        let delivered = self.delivered.saturating_sub(stamp.delivered);
        let send_elapsed = stamp.sent_us.saturating_sub(stamp.first_sent_us);
        let ack_elapsed = now_us.saturating_sub(stamp.delivered_us);
        self.delivered_us = now_us;
        self.first_sent_us = stamp.sent_us;
        let interval = send_elapsed.max(ack_elapsed);
        if interval < MIN_SAMPLE_US {
            return None;
        }
        let rate = u128::from(delivered) * 8 * 1_000_000 / u128::from(interval);
        let rate = rate.min(u128::from(u64::MAX)) as u64;
        self.all.push(now_us, rate);
        if stamp.busy {
            self.busy.push(now_us, rate);
        }
        Some(rate)
    }

    /// The largest delivery sample of the window: what the line carried at
    /// least.
    pub fn rate_bps(&self, now_us: u64) -> Option<u64> {
        self.all.value(now_us)
    }

    /// The largest sample of frames that queued behind an earlier one: the
    /// line's rate, not the content's.
    pub fn link_bps(&self, now_us: u64) -> Option<u64> {
        self.busy.value(now_us)
    }

    /// A new path: nothing measured on the old one holds.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// What the rate control keeps of the queue (`RateController::queue`).
#[derive(Clone, Debug)]
pub struct QueueState {
    delivery: DeliveryRate,
    rtt_floor: Extremum,
    /// The smoothed round trip of the path at the last tick (0: none).
    last_rtt_us: u64,
    /// The last frame the queue guard or the sender credit refused.
    full_us: Option<u64>,
    /// The last decrease to the line's delivery rate.
    lowered_us: Option<u64>,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            delivery: DeliveryRate::default(),
            rtt_floor: Extremum::min(RTT_FLOOR_WINDOW_US),
            last_rtt_us: 0,
            full_us: None,
            lowered_us: None,
        }
    }
}

impl QueueState {
    pub fn set_rtt(&mut self, rtt_us: u64) {
        self.last_rtt_us = rtt_us;
    }

    pub fn on_sent(&mut self, bytes: u64, sent_us: u64) {
        self.delivery.on_sent(bytes, sent_us);
    }

    /// One acknowledged frame; the round trip of the last tick joins the
    /// floor's window at the same time.
    pub fn on_acked(&mut self, sent_us: u64, now_us: u64) -> Option<u64> {
        if self.last_rtt_us != 0 {
            self.rtt_floor.push(now_us, self.last_rtt_us);
        }
        self.delivery.on_acked(sent_us, now_us)
    }

    /// A refused frame within the last `interval_us`.
    pub fn recently_full(&self, now_us: u64, interval_us: u64) -> bool {
        self.full_us
            .is_some_and(|at| now_us.saturating_sub(at) <= interval_us)
    }
}

#[cfg(test)]
#[path = "p9_rate_queue_task_tests.rs"]
mod p9_rate_queue_task_tests;

/// The queue of the picture stream (plan 2026-11-17 P9).
impl super::RateController {
    /// The rate the queue guard lets wait: the target, at most what the line
    /// was measured to deliver (`DeliveryRate::rate_bps`).
    pub fn queue_rate_bps(&self, now_us: u64) -> u64 {
        let target = self.target_bps();
        self.queue
            .delivery
            .rate_bps(now_us)
            .map_or(target, |delivered| delivered.min(target))
    }

    /// The smallest smoothed round trip of the last `RTT_FLOOR_WINDOW_US`;
    /// the queue the stream itself builds does not raise it.
    pub fn base_rtt_us(&self, now_us: u64) -> Option<u64> {
        self.queue.rtt_floor.value(now_us)
    }

    /// The line's rate from frames that queued behind earlier ones.
    pub fn link_rate_bps(&self, now_us: u64) -> Option<u64> {
        self.queue.delivery.link_bps(now_us)
    }

    /// Growth waits: a frame was refused within the last interval while the
    /// target is not below the line's measured rate (or none is known). A
    /// refusal below the line (two lossless frames on a faster line) holds
    /// nothing.
    pub(super) fn growth_held(&self, now_us: u64) -> bool {
        self.queue.recently_full(now_us, self.interval_us)
            && self
                .link_rate_bps(now_us)
                .is_none_or(|link| self.target_bps() >= link)
    }

    /// The queue guard or the sender credit refused a frame at `now_us`:
    /// the sender has more than the line takes. When the target is above
    /// the rate the line delivered with frames queued behind each other,
    /// the encoder gets less: the target sinks to `model::decreased` of that
    /// rate (0.85 ×, as after a persistent overuse), at most once per
    /// interval (max(round trip, frame interval)). A frame refused while the
    /// line carries the target (a key frame in flight) lowers nothing, and a
    /// sender whose frames never queued knows no line rate and keeps its
    /// target. Growth waits one interval after a refusal (`grow`).
    pub(super) fn on_queue_full(&mut self, now_us: u64) {
        self.queue.full_us = Some(now_us);
        let Some(link) = self.link_rate_bps(now_us) else {
            return;
        };
        let current = self.target_bps() as f64;
        if current <= link as f64 {
            return;
        }
        let recent = self
            .queue
            .lowered_us
            .is_some_and(|at| now_us.saturating_sub(at) < self.interval_us);
        if recent {
            return;
        }
        self.queue.lowered_us = Some(now_us);
        let lowered = super::model::decreased(current, false, Some(link));
        self.delay_target = self.link_clamp(lowered);
        self.sustained = Some(self.delay_target);
        self.probe.on_limit();
        self.last_update_us = now_us;
    }
}
