//! Pacing of one picture stream (decision C3, points 3 and 4 of
//! „Staukontrolle und Bitrate“): the frame clock that takes captured frames
//! at the wanted rate; the queue guard that skips a capture before encoding
//! while too many bytes are unacknowledged, on the relay path at most
//! 768 KiB because the relay drops above 1 MiB per destination (NR N19); the
//! reset of a picture stream whose oldest unacknowledged frame is older than
//! one second; and coalesced key-frame requests. Frames are only ever
//! dropped before encoding, so the decoder never misses a reference.
use std::collections::VecDeque;

use super::rate_control::PathKind;

/// Per-destination queue of the relay server; above it the relay drops.
pub(crate) const RELAY_QUEUE_BYTES: u64 = 1024 * 1024;
/// Guard on the relay path: 256 KiB below `RELAY_QUEUE_BYTES` for the control
/// stream and other connections to the same device.
pub(crate) const RELAY_GUARD_BYTES: u64 = 768 * 1024;
/// Queueing the guard allows beyond one round trip: 100 ms of the target.
pub(crate) const QUEUE_ALLOWANCE_US: u64 = 100_000;
/// Age of the oldest unacknowledged frame that resets the picture stream.
pub(crate) const STALE_STREAM_US: u64 = 1_000_000;
/// Frames in flight that reset the stream regardless of age: one second at
/// 240 frames/s, four times over.
pub(crate) const MAX_IN_FLIGHT_FRAMES: usize = 1024;
/// Highest frame rate of the frame clock: the highest common display
/// refresh rate; capture never delivers more.
pub(crate) const MAX_FPS: u32 = 240;

/// Unacknowledged bytes the guard allows: the target rate over one round
/// trip plus `QUEUE_ALLOWANCE_US`, on the relay path at most
/// `RELAY_GUARD_BYTES`. The round trip counts up to `STALE_STREAM_US`.
pub(crate) fn guard_bytes(target_bps: u64, path: PathKind, rtt_us: u64) -> u64 {
    let window_us = u128::from(rtt_us.min(STALE_STREAM_US)) + u128::from(QUEUE_ALLOWANCE_US);
    let bytes = (u128::from(target_bps) * window_us / 8 / 1_000_000) as u64;
    match path {
        PathKind::Direct => bytes,
        PathKind::Relay => bytes.min(RELAY_GUARD_BYTES),
    }
}

/// What to do with the frame just captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaceDecision {
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
pub(crate) struct SentFrame {
    pub gop_id: u64,
    pub frame_id: u64,
    /// Bytes of all records of the frame.
    pub bytes: u64,
    /// Host clock when the last record was handed to the stream.
    pub sent_us: u64,
    pub keyframe: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pacer {
    in_flight: VecDeque<SentFrame>,
    unacked_bytes: u64,
    delta_bytes: u64,
    key_bytes: u64,
    keyframe_due: bool,
    dropped: u64,
    resets: u64,
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
    pub(crate) fn decide(
        &mut self,
        now_us: u64,
        target_bps: u64,
        path: PathKind,
        rtt_us: u64,
    ) -> PaceDecision {
        if let Some(oldest) = self.in_flight.front() {
            if now_us.saturating_sub(oldest.sent_us) > STALE_STREAM_US
                || self.in_flight.len() >= MAX_IN_FLIGHT_FRAMES
            {
                return PaceDecision::ResetStream;
            }
            let expected = if self.keyframe_due {
                self.key_bytes
            } else {
                self.delta_bytes
            };
            if self.unacked_bytes + expected > guard_bytes(target_bps, path, rtt_us) {
                self.dropped += 1;
                return PaceDecision::DropBeforeEncode;
            }
        }
        PaceDecision::Encode {
            keyframe: self.keyframe_due,
        }
    }

    pub(crate) fn on_frame_sent(&mut self, frame: SentFrame) {
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
    pub(crate) fn on_ack(&mut self, gop_id: u64, frame_id: u64) -> Option<SentFrame> {
        let mut acked = None;
        let mut released = 0u64;
        self.in_flight.retain(|frame| {
            if frame.gop_id != gop_id || frame.frame_id > frame_id {
                return true;
            }
            released += frame.bytes;
            if frame.frame_id == frame_id {
                acked = Some(*frame);
            }
            false
        });
        self.unacked_bytes = self.unacked_bytes.saturating_sub(released);
        acked
    }

    /// The picture stream was reset: every frame in flight is forgotten (late
    /// acknowledgements of them are ignored) and the next frame is a key
    /// frame on a new stream.
    pub(crate) fn on_stream_reset(&mut self) {
        self.in_flight.clear();
        self.unacked_bytes = 0;
        self.keyframe_due = true;
        self.resets += 1;
    }

    /// `KEYFRAME_REQUEST`, a joining viewer or a size change; requests before
    /// the next encoded frame make one key frame.
    pub(crate) fn request_keyframe(&mut self) {
        self.keyframe_due = true;
    }

    pub(crate) fn keyframe_due(&self) -> bool {
        self.keyframe_due
    }

    pub(crate) fn unacked_bytes(&self) -> u64 {
        self.unacked_bytes
    }

    pub(crate) fn frames_in_flight(&self) -> usize {
        self.in_flight.len()
    }

    pub(crate) fn dropped_before_encode(&self) -> u64 {
        self.dropped
    }

    pub(crate) fn resets(&self) -> u64 {
        self.resets
    }
}

/// Takes captured frames at the wanted rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameClock {
    interval_us: u64,
    next_due_us: Option<u64>,
}

fn interval_us(fps: u32) -> u64 {
    1_000_000 / u64::from(fps.clamp(1, MAX_FPS))
}

impl FrameClock {
    pub(crate) fn new(fps: u32) -> Self {
        Self {
            interval_us: interval_us(fps),
            next_due_us: None,
        }
    }

    pub(crate) fn set_fps(&mut self, fps: u32) {
        self.interval_us = interval_us(fps);
    }

    pub(crate) fn interval_us(&self) -> u64 {
        self.interval_us
    }

    /// Whether the frame captured at `now_us` is taken. A frame up to a
    /// quarter interval early counts as due, so a 60 Hz source at 30
    /// frames/s yields every second frame despite jitter; after a stall of
    /// more than one interval the cadence starts over instead of bursting.
    pub(crate) fn take(&mut self, now_us: u64) -> bool {
        let interval = self.interval_us;
        let next = match self.next_due_us {
            Some(due) if now_us + interval / 4 < due => return false,
            Some(due) if due + 2 * interval >= now_us => due + interval,
            _ => now_us + interval,
        };
        self.next_due_us = Some(next);
        true
    }
}

#[cfg(test)]
#[path = "pacing_tests.rs"]
mod tests;
