//! What the viewer shows paces the pictures (review 2026-11-19, Gemini
//! 3/3 and round 5): its `FRAME_ACK` leaves when a picture is read off the
//! network, its `PRESENTED` when the picture is on screen.
use super::Pacer;

/// Review 2026-11-19 (Gemini 3/3, round 5 G5): how long a picture the
/// viewer received (its `FRAME_ACK`, sent when the picture is read off the
/// network) may wait for its `PRESENTED` before the next picture waits for
/// the viewer: a host sent 30–120 pictures a second of which the viewer
/// showed 1–10. A time, not a count: at 240 pictures a second a few
/// pictures are 20 ms of relay, decoding and vsync. [Assumption: a viewer
/// that keeps up shows a picture within 250 ms of its arrival.]
pub const VIEWER_LAG_US: u64 = 250_000;
/// How long the last `PRESENTED` counts at least: a viewer that shows
/// nothing (a gate after a visibility change, a display path without
/// `PRESENTED`) does not hold the pictures back after it.
pub const PRESENTED_FRESH_US: u64 = 1_000_000;
/// A viewer that shows slowly (a software decoder at 4K) keeps counting for
/// `PRESENTED_GAPS` of its own time between two `PRESENTED`, at most
/// `PRESENTED_HOLD_MAX_US` (round 8 Gemini 8/2: after one second without a
/// `PRESENTED` such a viewer was no longer held, the pictures piled up in
/// the relay and its reader, and the stream broke off as stale).
pub const PRESENTED_GAPS: u64 = 3;
pub const PRESENTED_HOLD_MAX_US: u64 = 5_000_000;
/// Received pictures kept for `viewer_behind` at most (a memory bound).
pub(super) const MAX_RECEIVED: usize = 256;

impl Pacer {
    /// The viewer acknowledged `frame_id` at `at_us`: it has the picture.
    pub fn on_received(&mut self, frame_id: u64, at_us: u64) {
        // Round 7 P9: an acknowledgement below the visibility floor that
        // comes after it is not waited for either.
        if frame_id >= self.received_floor
            && self.presented.is_none_or(|(shown, _)| frame_id > shown)
        {
            if self.received.len() == MAX_RECEIVED {
                self.received.pop_front();
            }
            self.received.push_back((frame_id, at_us));
        }
    }

    /// `PRESENTED` of `frame_id` at `now_us`: the viewer showed it, and every
    /// earlier picture is done with (shown or skipped). One picture may
    /// follow at once (`viewer_behind`).
    pub fn on_presented(&mut self, frame_id: u64, now_us: u64) {
        // Round 9 F6: a gap counts only when received pictures still waited
        // at the `PRESENTED` before (a still screen between two pictures is
        // no slow viewer).
        self.presented_gap_us = match self.presented {
            Some((_, before)) if self.waiting_at_presented => now_us.saturating_sub(before),
            _ => 0,
        };
        let newest = self
            .presented
            .map_or(frame_id, |(frame, _)| frame.max(frame_id));
        self.presented = Some((newest, now_us));
        while self
            .received
            .front()
            .is_some_and(|(frame, _)| *frame <= newest)
        {
            self.received.pop_front();
        }
        self.waiting_at_presented = !self.received.is_empty();
        self.present_credit = true;
    }

    /// A visibility acknowledgement named `floor`: the viewer shows no
    /// picture below it (round 6 L4), so those are not waited for.
    pub fn forget_received_below(&mut self, floor: u64) {
        self.received_floor = self.received_floor.max(floor);
        while self
            .received
            .front()
            .is_some_and(|(frame, _)| *frame < floor)
        {
            self.received.pop_front();
        }
    }

    /// Whether the next picture waits for the viewer: it shows pictures
    /// (its last `PRESENTED` came within `PRESENTED_FRESH_US`, or within
    /// `PRESENTED_GAPS` of its own time between two of them, at most
    /// `PRESENTED_HOLD_MAX_US`), the oldest picture it received and did not
    /// show came more than `VIEWER_LAG_US` ago, and no picture went out
    /// since its last `PRESENTED`. A viewer that keeps up is never held; one
    /// that falls behind gets one picture per picture it shows, and never
    /// waits past its next `PRESENTED`. A viewer that stops sending
    /// `PRESENTED` is no longer held after that time (a display path without
    /// it never becomes one picture a second). No stream is reset for it and
    /// the line's rate stays; a viewer without `PRESENTED` (older ones,
    /// `se screen bench`) is never held.
    pub fn viewer_behind(&self, now_us: u64) -> bool {
        let fresh = PRESENTED_FRESH_US.max(
            self.presented_gap_us
                .saturating_mul(PRESENTED_GAPS)
                .min(PRESENTED_HOLD_MAX_US),
        );
        let showing = self
            .presented
            .is_some_and(|(_, at)| now_us.saturating_sub(at) <= fresh);
        let lagging = self
            .received
            .front()
            .is_some_and(|(_, at)| now_us.saturating_sub(*at) > VIEWER_LAG_US);
        showing && lagging && !self.present_credit
    }
}
