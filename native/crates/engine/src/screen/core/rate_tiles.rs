//! The byte budget of a picture stream in the rate control
//! (`tiles_rate.rs`; plan 2026-11-18 „Netzrate“ M1, fix „Diashow“).
//!
//! Review 2026-11-19: the user's „Höchste Datenrate“ is a hard ceiling for
//! every codec. A video encoder only aims at its target: key frames, the
//! tolerance of its honour meter and the constant quality of „Scharf (GPU)“
//! put more on the line than the ceiling. The same token bucket as the
//! tiles' therefore books every sent picture of a video stream as well and
//! holds the next one back (before capture and encoding) while it is in
//! debt; without a user ceiling a video stream is not held.
use super::RateController;

impl RateController {
    /// The rate the tiles' budget pays off at, at `now_us`: the ceiling
    /// (user's „Höchste Datenrate“, codec pairing) and, while the path
    /// signalled congestion (`TileBudget::network_limited`), the target E.
    /// Otherwise the queue guard holds the tiles to what the line delivers.
    /// Never above the user's ceiling, also below the floor of the target.
    pub fn tiles_budget_bps(&self, now_us: u64) -> u64 {
        let rate = if self.tiles.network_limited(now_us) {
            self.target_bps()
        } else {
            self.ceiling_bps()
        };
        self.user_ceiling_bps.map_or(rate, |user| rate.min(user))
    }

    /// The rate the byte budget of a stream pays off at: the tiles'
    /// (`tiles_budget_bps`); for a video codec the user's ceiling exactly,
    /// `None` (no budget) without one.
    pub fn stream_budget_bps(&self, now_us: u64, video: bool) -> Option<u64> {
        if video {
            self.user_ceiling_bps
        } else {
            Some(self.tiles_budget_bps(now_us))
        }
    }

    /// Whether a picture of the stream may be captured and encoded at
    /// `now_us`.
    pub fn stream_admit(&self, now_us: u64, video: bool) -> bool {
        self.stream_budget_bps(now_us, video)
            .is_none_or(|rate| self.tiles.admits(now_us, rate))
    }

    /// A picture of the stream of `bytes` started its send at `at_us`.
    pub fn on_stream_sent(&mut self, at_us: u64, bytes: u64, video: bool) {
        if let Some(rate) = self.stream_budget_bps(at_us, video) {
            self.tiles.on_sent(at_us, bytes, rate);
        }
    }

    /// Whether a picture of the tiles may be taken at `now_us`.
    pub fn tiles_admit(&self, now_us: u64) -> bool {
        self.stream_admit(now_us, false)
    }

    /// A picture of the tiles of `bytes` started its send at `at_us`.
    pub fn on_tiles_sent(&mut self, at_us: u64, bytes: u64) {
        self.on_stream_sent(at_us, bytes, false);
    }
}
