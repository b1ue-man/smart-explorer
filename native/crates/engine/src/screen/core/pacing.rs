//! Pacing of one picture stream (decision C3, points 3 and 4 of
//! „Staukontrolle und Bitrate“): the frame clock that takes captured frames
//! at the wanted rate; the queue guard that skips a capture before encoding
//! while too many bytes are unacknowledged, on the relay path at most
//! 768 KiB because the relay drops above 1 MiB per destination (NR N19); the
//! reset of a picture stream whose oldest unacknowledged frame is older than
//! the stale time (`stale_after_us`, plan 2026-11-14 „Bildkette“ B-35); and
//! coalesced key-frame requests. Frames are only ever dropped before
//! encoding, so the decoder never misses a reference.
use std::collections::VecDeque;

use super::rate_control::PathKind;

/// Per-destination queue of the relay server; above it the relay drops.
pub const RELAY_QUEUE_BYTES: u64 = 1024 * 1024;
/// Guard on the relay path: 256 KiB below `RELAY_QUEUE_BYTES` for the control
/// stream and other connections to the same device.
pub const RELAY_GUARD_BYTES: u64 = 768 * 1024;
/// Queueing the guard allows beyond one round trip: 100 ms of the target.
pub const QUEUE_ALLOWANCE_US: u64 = 100_000;
/// Age of the oldest unacknowledged frame that resets the picture stream
/// while the pacer knows no round trip (`Pacer::set_timing`), and the
/// longest a single write may stall (decision C3, 2026-10-17).
pub const STALE_STREAM_US: u64 = 1_000_000;
/// Upper bound of the frames in flight, a memory bound: 1024 `SentFrame`s
/// of 40 bytes are 40 KiB per session. Below it the bound follows the stale
/// time (`in_flight_limit`); without timing it is the old rule (one second
/// at 240 frames/s, four times over).
pub const MAX_IN_FLIGHT_FRAMES: usize = 1024;
/// B-35: round trips the oldest frame may wait for its acknowledgement. As
/// the probe timeout of RFC 9002 (6.2.1: `smoothed_rtt + max(4 × rttvar,
/// granularity) + max_ack_delay`) the bound is a multiple of the smoothed
/// round trip, so it scales from the LAN to the WAN instead of the fixed
/// second; 4 covers the PTO with a variance up to ¾ of the round trip. The
/// viewer acknowledges without delay (`gop_stream.rs`).
pub const STALE_RTT_FACTOR: u64 = 4;
/// B-35: frame intervals the oldest frame may wait at least, so a LAN with
/// a round trip below one frame interval does not reset on the jitter of
/// one late frame (the frame clock already takes frames ¼ interval early).
pub const STALE_FRAME_INTERVALS: u64 = 3;
/// Highest frame rate of the frame clock: the highest common display
/// refresh rate; capture never delivers more.
pub const MAX_FPS: u32 = 240;
#[path = "pacing_viewer.rs"]
mod viewer;
pub use viewer::{PRESENTED_FRESH_US, PRESENTED_HOLD_MAX_US, VIEWER_LAG_US};

/// Unacknowledged bytes the guard allows: the target rate over one round
/// trip plus `QUEUE_ALLOWANCE_US`, on the relay path at most
/// `RELAY_GUARD_BYTES`. The round trip counts up to `STALE_STREAM_US`.
pub fn guard_bytes(target_bps: u64, path: PathKind, rtt_us: u64) -> u64 {
    let window_us = u128::from(rtt_us.min(STALE_STREAM_US)) + u128::from(QUEUE_ALLOWANCE_US);
    let bytes = (u128::from(target_bps) * window_us / 8 / 1_000_000) as u64;
    match path {
        PathKind::Direct => bytes,
        PathKind::Relay => bytes.min(RELAY_GUARD_BYTES),
    }
}

/// The stale time of a picture stream (B-35): max(4 × smoothed round trip,
/// 3 frame intervals), plus the time the line needs at `target_bps` to
/// carry the bytes still unacknowledged. A frame's age counts from the
/// moment its last record was handed to the stream (`SentFrame::sent_us`),
/// not from the moment it left, so a key frame larger than four round trips
/// of the line (300 KB at 6 Mbit/s need 400 ms) would otherwise reset its
/// own stream before its acknowledgement could arrive, every time.
pub fn stale_after_us(srtt_us: u64, interval_us: u64, unacked_bytes: u64, target_bps: u64) -> u64 {
    let base = srtt_us
        .saturating_mul(STALE_RTT_FACTOR)
        .max(interval_us.saturating_mul(STALE_FRAME_INTERVALS));
    let drain = u128::from(unacked_bytes) * 8 * 1_000_000 / u128::from(target_bps.max(1));
    base.saturating_add(u64::try_from(drain).unwrap_or(u64::MAX))
}

/// Frames in flight that reset the stream regardless of their age: twice
/// the frames the cadence sends within the stale time. Frames come at most
/// every ¾ interval (`FrameClock::take`) plus one encoded ahead (B-27), so
/// a stream whose oldest frame is still in time never reaches it; the count
/// only catches a broken cadence. At most `MAX_IN_FLIGHT_FRAMES`.
pub fn in_flight_limit(stale_us: u64, interval_us: u64) -> usize {
    let frames = stale_us / interval_us.max(1) + 1;
    usize::try_from(frames.saturating_mul(2))
        .unwrap_or(usize::MAX)
        .min(MAX_IN_FLIGHT_FRAMES)
}

/// What to do with the frame just captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaceDecision {
    /// Encode it; `keyframe` asks the encoder for an IDR or full tile frame.
    Encode { keyframe: bool },
    /// Skip it before encoding; the queue is too full.
    DropBeforeEncode,
    /// Reset the picture stream, call `on_stream_reset`, decide again: the
    /// next frame is a key frame on a new stream.
    ResetStream,
}

/// One frame handed to a picture stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SentFrame {
    pub gop_id: u64,
    pub frame_id: u64,
    /// Bytes of all records of the frame.
    pub bytes: u64,
    /// Host clock when the last record was handed to the stream.
    pub sent_us: u64,
    pub keyframe: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pacer {
    in_flight: VecDeque<SentFrame>,
    unacked_bytes: u64,
    delta_bytes: u64,
    key_bytes: u64,
    keyframe_due: bool,
    dropped: u64,
    resets: u64,
    /// Smoothed round trip and frame interval (µs) for B-35; without them
    /// the fixed second and `MAX_IN_FLIGHT_FRAMES` apply.
    timing: Option<(u64, u64)>,
    /// The rate the line was measured to carry (`set_drain_rate`); the
    /// unacknowledged bytes drain at most this fast.
    drain_bps: Option<u64>,
    /// P9: the delivered rate and the round-trip floor the queue guard
    /// follows (`set_queue`); without them the guard of `guard_bytes`.
    queue: Option<(u64, u64)>,
    /// Smoothed time from handing a frame to the stream until its
    /// acknowledgement arrived (µs, 0: none yet). Unlike the path's round
    /// trip it holds the viewer's decoding before its `FRAME_ACK`.
    ack_age_us: u64,
    /// The pictures the viewer received and did not show yet, with the time
    /// their acknowledgement came; the newest it showed with the time of its
    /// `PRESENTED`; whether one picture may follow that `PRESENTED`
    /// (`viewer_behind`).
    received: VecDeque<(u64, u64)>,
    /// The last visibility acknowledgement's floor: pictures below it are
    /// never waited for (`forget_received_below`).
    received_floor: u64,
    presented: Option<(u64, u64)>,
    present_credit: bool,
    /// The time between the viewer's last two `PRESENTED` while pictures
    /// waited for it (`viewer_behind`), and whether some still wait.
    presented_gap_us: u64,
    waiting_at_presented: bool,
    /// No queue guard (user 2026-11-25, „ich WILL NICHT DASS DU DROSSELST“);
    /// `delivery` alone decides about a picture the line delivers late.
    unthrottled: bool,
    delivery: super::delivery::Delivery,
    /// The pictures go as datagrams (`video_transport.rs`): sent and
    /// forgotten, never a stale reset (`pacing_datagram.rs`).
    datagrams: bool,
}

impl Default for Pacer {
    /// The first frame of a session is a key frame.
    fn default() -> Self {
        Self {
            in_flight: VecDeque::new(),
            unacked_bytes: 0,
            delta_bytes: 0,
            key_bytes: 0,
            keyframe_due: true,
            dropped: 0,
            resets: 0,
            timing: None,
            drain_bps: None,
            queue: None,
            ack_age_us: 0,
            received: VecDeque::new(),
            received_floor: 0,
            presented: None,
            present_credit: false,
            presented_gap_us: 0,
            waiting_at_presented: false,
            unthrottled: false,
            delivery: super::delivery::Delivery::Realtime,
            datagrams: false,
        }
    }
}

/// Running average of frame sizes with weight 1/8 for the new size.
fn average(current: u64, sample: u64) -> u64 {
    if current == 0 {
        sample
    } else {
        (current * 7 + sample) / 8
    }
}

impl Pacer {
    /// Decides for the frame captured at `now_us`. With nothing in flight a
    /// frame is always encoded, so even a key frame larger than the guard
    /// gets through; otherwise the expected size of the next frame (average
    /// of the same kind) must fit under the guard.
    pub fn decide(
        &mut self,
        now_us: u64,
        target_bps: u64,
        path: PathKind,
        rtt_us: u64,
    ) -> PaceDecision {
        // „Zuverlässig“ (user 2026-11-25): no picture is given up, the
        // stream catches up on a slower line.
        let reliable = self.unthrottled && self.delivery == super::delivery::Delivery::Reliable;
        if self.datagrams {
            self.forget_stale_datagrams(now_us, target_bps);
        }
        let resets = !reliable && !self.datagrams;
        if let Some(oldest) = self.in_flight.front().filter(|_| resets) {
            let (stale_us, limit) = self.stale_limits(target_bps);
            if now_us.saturating_sub(oldest.sent_us) > stale_us || self.in_flight.len() >= limit {
                return PaceDecision::ResetStream;
            }
        }
        // Unthrottled, only the relay's own queue bounds the bytes in flight
        // (it drops above it per destination, NR N19).
        let guard = match (self.unthrottled, path) {
            (false, _) => Some(self.guard(target_bps, path, rtt_us)),
            (true, PathKind::Relay) => Some(RELAY_GUARD_BYTES),
            (true, PathKind::Direct) => None,
        };
        if let Some(guard) = guard.filter(|_| !self.in_flight.is_empty()) {
            let expected = match self.keyframe_due {
                true => self.key_bytes,
                false => self.delta_bytes,
            };
            if self.unacked_bytes + expected > guard {
                self.dropped += 1;
                return PaceDecision::DropBeforeEncode;
            }
        }
        PaceDecision::Encode {
            keyframe: self.keyframe_due,
        }
    }

    pub fn set_unthrottled(&mut self, unthrottled: bool) {
        self.unthrottled = unthrottled;
    }

    pub fn set_delivery(&mut self, delivery: super::delivery::Delivery) {
        self.delivery = delivery;
    }

    pub fn delivery(&self) -> super::delivery::Delivery {
        self.delivery
    }

    /// B-35: the smoothed round trip of the path and the frame interval of
    /// the cadence; the stale time follows them from now on.
    pub fn set_timing(&mut self, srtt_us: u64, interval_us: u64) {
        self.timing = Some((srtt_us, interval_us));
    }

    /// Plan 2026-11-17: the rate the line was measured to carry
    /// (`RateController::sustained_bps`). The stale time lets the
    /// unacknowledged bytes drain at the smaller of it and the target, so a
    /// large lossless key frame on a line slower than the target is not
    /// declared stale before it can arrive (the reset loop of lossless
    /// sessions).
    pub fn set_drain_rate(&mut self, bps: u64) {
        self.drain_bps = Some(bps.max(1));
    }

    /// P9: the rate the line delivered (`RateController::queue_rate_bps`)
    /// and the smallest round trip of the path; the queue guard follows
    /// them from now on (`guard`).
    pub fn set_queue(&mut self, rate_bps: u64, base_rtt_us: u64) {
        self.queue = Some((rate_bps, base_rtt_us));
    }

    /// Unacknowledged bytes the next frame may join. After `set_queue` the
    /// delivered rate over the round-trip floor plus two frames
    /// (`rate_control::queue::queue_cap_bytes`): at most about one frame
    /// waits beyond what the line drains, and a frame larger than the
    /// bandwidth-delay product is sent while the next one is encoded.
    /// Before it the target over the round trip plus `QUEUE_ALLOWANCE_US`.
    pub fn guard(&self, target_bps: u64, path: PathKind, rtt_us: u64) -> u64 {
        match self.queue {
            Some((rate_bps, base_rtt_us)) => {
                // Before the first delta frame its size is unknown: the key
                // frame's stands for it (a lossless stream's frames are alike).
                let frame = if self.keyframe_due || self.delta_bytes == 0 {
                    self.key_bytes.max(self.delta_bytes)
                } else {
                    self.delta_bytes
                };
                super::rate_control::queue::queue_cap_bytes(
                    rate_bps.min(target_bps),
                    base_rtt_us,
                    frame,
                    path,
                )
            }
            None => guard_bytes(target_bps, path, rtt_us),
        }
    }

    /// The stale time and the frame limit for the next decision.
    pub fn stale_limits(&self, target_bps: u64) -> (u64, usize) {
        // The bytes drain at most at the measured delivered rate (fix
        // 2026-11-19): the sustained estimate runs up to 1.5 times of it.
        let delivered = self.queue.map_or(u64::MAX, |(rate_bps, _)| rate_bps.max(1));
        let drain_bps = self
            .drain_bps
            .map_or(target_bps, |drain| drain.min(target_bps))
            .min(delivered);
        match self.timing {
            Some((srtt_us, interval_us)) => {
                // Fix 2026-11-19: the round trip of a frame is the age of its
                // acknowledgement, the viewer's decoding included; with only
                // the path's round trip (a LAN: 3 frame intervals, 12.5 ms at
                // 240 frames/s) a viewer that decodes longer reset the stream
                // with every frame, each reset a key frame and half the rate.
                let ack_age = self.ack_age_us.min(STALE_STREAM_US / STALE_RTT_FACTOR);
                let round_trip = srtt_us.max(ack_age);
                let stale = stale_after_us(round_trip, interval_us, self.unacked_bytes, drain_bps);
                (stale, in_flight_limit(stale, interval_us))
            }
            None => (STALE_STREAM_US, MAX_IN_FLIGHT_FRAMES),
        }
    }

    pub fn on_budget_drop(&mut self) {
        self.dropped = self.dropped.saturating_add(1);
    }

    pub fn on_frame_sent(&mut self, frame: SentFrame) {
        self.present_credit = false;
        if frame.keyframe {
            self.keyframe_due = false;
            self.key_bytes = average(self.key_bytes, frame.bytes);
        } else {
            self.delta_bytes = average(self.delta_bytes, frame.bytes);
        }
        self.unacked_bytes += frame.bytes;
        self.in_flight.push_back(frame);
    }

    /// `FRAME_ACK` for `frame_id` of `gop_id`: a picture stream is ordered,
    /// so every earlier frame of that stream arrived as well. Returns the
    /// acknowledged frame for the rate control; frames forgotten by a reset
    /// return `None`.
    pub fn on_ack(&mut self, gop_id: u64, frame_id: u64) -> Option<SentFrame> {
        let mut acked = None;
        let mut released = 0u64;
        // Datagram pictures reach the decoder in order or are given up:
        // an acknowledgement releases every older picture of any group.
        let any_group = self.datagrams;
        self.in_flight.retain(|frame| {
            if (frame.gop_id != gop_id && !any_group) || frame.frame_id > frame_id {
                return true;
            }
            released += frame.bytes;
            if frame.frame_id == frame_id && frame.gop_id == gop_id {
                acked = Some(*frame);
            }
            false
        });
        self.unacked_bytes = self.unacked_bytes.saturating_sub(released);
        acked
    }

    /// The age of an acknowledged frame (arrival minus `SentFrame::sent_us`,
    /// both clocks of this host), averaged like the frame sizes.
    pub fn note_ack_age(&mut self, age_us: u64) {
        self.ack_age_us = average(self.ack_age_us, age_us.max(1));
    }

    /// The picture stream was reset: every frame in flight is forgotten (late
    /// acknowledgements of them are ignored) and the next frame is a key
    /// frame on a new stream.
    pub fn on_stream_reset(&mut self) {
        // Pictures of the old stream the viewer did not show are not
        // waited for (Codex 5/3): the next picture starts the new one.
        self.received.clear();
        self.in_flight.clear();
        self.unacked_bytes = 0;
        self.keyframe_due = true;
        self.resets += 1;
    }

    /// The picture stream was reset and the next one continues the encoder's
    /// chain (B-36): the frames in flight are forgotten as by
    /// `on_stream_reset`, but no key frame is due for it; one already due
    /// stays due.
    pub fn on_stream_reset_continued(&mut self) {
        self.received.clear();
        self.in_flight.clear();
        self.unacked_bytes = 0;
        self.resets += 1;
    }

    /// `KEYFRAME_REQUEST`, a joining viewer or a size change; requests before
    /// the next encoded frame make one key frame.
    pub fn request_keyframe(&mut self) {
        self.keyframe_due = true;
    }

    pub fn keyframe_due(&self) -> bool {
        self.keyframe_due
    }

    pub fn unacked_bytes(&self) -> u64 {
        self.unacked_bytes
    }

    /// Age at `now_us` of the oldest unacknowledged frame, if any.
    pub fn oldest_age_us(&self, now_us: u64) -> Option<u64> {
        self.in_flight
            .front()
            .map(|frame| now_us.saturating_sub(frame.sent_us))
    }

    pub fn frames_in_flight(&self) -> usize {
        self.in_flight.len()
    }

    pub fn dropped_before_encode(&self) -> u64 {
        self.dropped
    }

    pub fn resets(&self) -> u64 {
        self.resets
    }
}

#[path = "pacing_clock.rs"]
mod clock;
pub use clock::{codec_cap, FrameClock};
#[path = "pacing_datagram.rs"]
mod datagram;

#[cfg(test)]
#[path = "pacing_tests.rs"]
mod tests;
