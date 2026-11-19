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
    /// The control stream wrote `bytes` at `now_us` (`STATE`, `STATS`, the
    /// pointer, window lists; review 2026-11-19): they count against the
    /// user's ceiling, which the pictures get less of (`user_picture_bps`).
    pub fn on_control_bytes(&mut self, now_us: u64, bytes: u64) {
        self.control.push(now_us, bytes);
        self.update_control(now_us);
    }

    /// The control stream's rate of the last window at `now_us`.
    pub fn update_control(&mut self, now_us: u64) {
        self.control.prune(now_us);
        self.control_bps = self.control.window_bps();
    }

    pub fn control_bps(&self) -> u64 {
        self.control_bps
    }

    /// The user's ceiling less the control stream's rate: what the
    /// pictures may take; never below the floor of the target (or the
    /// ceiling itself below it). `None` without a user ceiling.
    pub fn user_picture_bps(&self) -> Option<u64> {
        self.user_ceiling_bps.map(|ceiling| {
            ceiling
                .saturating_sub(self.control_bps)
                .max(super::MIN_BITRATE_BPS.min(ceiling))
        })
    }

    /// `(control rate, ceiling)` once when the control stream alone came
    /// above the user's ceiling (a notice for the viewer), else `None`.
    pub fn control_over_ceiling(&mut self) -> Option<(u64, u64)> {
        let ceiling = self.user_ceiling_bps?;
        let over = self.control_bps > ceiling;
        let fresh = over && !self.control_over;
        self.control_over = over;
        fresh.then_some((self.control_bps, ceiling))
    }

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
        self.user_picture_bps().map_or(rate, |user| rate.min(user))
    }

    /// The rate the byte budget of a stream pays off at: the tiles'
    /// (`tiles_budget_bps`); for a video codec the user's ceiling exactly,
    /// `None` (no budget) without one.
    pub fn stream_budget_bps(&self, now_us: u64, video: bool) -> Option<u64> {
        if video {
            self.user_picture_bps()
        } else {
            Some(self.tiles_budget_bps(now_us))
        }
    }

    /// The pace of a picture's send at `now_us` (review 2026-11-19, Codex
    /// 4/5): the budget's rate and the bytes of its saved credit; `None`
    /// without a budget.
    pub fn stream_pace(&self, now_us: u64, video: bool) -> Option<(u64, u64)> {
        let rate = self.stream_budget_bps(now_us, video)?;
        Some((rate.max(1), self.tiles.credit_bits(now_us, rate) / 8))
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
