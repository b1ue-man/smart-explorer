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

/// The least of a user ceiling `ceiling` the pictures need (round 6 L2):
/// below it no picture is made (`stream_admit`), so a picture is never
/// paced at a few bits a second and its job never hangs for hours. Half the
/// ceiling at most, so a low ceiling with steady status traffic still shows
/// pictures; never above the floor of the target.
pub fn picture_floor_bps(ceiling: u64) -> u64 {
    super::MIN_BITRATE_BPS.min(ceiling / 2).max(1)
}

impl RateController {
    /// The control stream wrote `bytes` at `now_us` (`STATE`, `STATS`, the
    /// pointer, window lists; review 2026-11-19): they count against the
    /// user's ceiling, which the pictures get less of (`user_picture_bps`).
    pub fn on_control_bytes(&mut self, now_us: u64, bytes: u64) {
        self.control.push(now_us, bytes);
        self.update_control(now_us);
    }

    /// The sound sent on the session's connection so far, `total` bytes at
    /// `now_us` (Codex 5/4): what came since the last reading counts like
    /// the control stream, so the pictures get the ceiling less control and
    /// sound. Returns the new bytes (for the statistics).
    pub fn on_audio_total(&mut self, now_us: u64, total: u64) -> u64 {
        let fresh = total.saturating_sub(self.audio_seen);
        self.audio_seen = total;
        // Round 6 L3: a rate over the time between two readings, not a block
        // of a second in the control window, which held two blocks at its edge.
        if let Some(at) = self.audio_at {
            let elapsed = now_us.saturating_sub(at).max(1);
            let bits = u128::from(fresh) * 8 * 1_000_000 / u128::from(elapsed);
            self.audio_bps = u64::try_from(bits).unwrap_or(u64::MAX);
        }
        self.audio_at = Some(now_us);
        self.update_control(now_us);
        fresh
    }

    /// The control stream's rate of the last window at `now_us`.
    pub fn update_control(&mut self, now_us: u64) {
        self.control.prune(now_us);
        self.control_bps = self.control.window_bps().saturating_add(self.audio_bps);
        // The share the pictures leave the session's small control frames
        // (round 11, Codex 11/7: what goes beyond it takes the budget).
        self.tiles.set_reserved(self.control.window_bps());
    }

    pub fn control_bps(&self) -> u64 {
        self.control_bps
    }

    /// The user's ceiling less the control stream's rate: what the
    /// pictures may put on the line, down to nothing (Codex 5/4: raised to
    /// the floor of the target, pictures and control together exceeded the
    /// ceiling). The target E keeps its floor (`ceiling_bps`); the byte
    /// budget holds the rest. `None` without a user ceiling.
    pub fn user_picture_bps(&self) -> Option<u64> {
        self.user_ceiling_bps.map(|ceiling| {
            ceiling
                .saturating_sub(self.control_bps)
                .max(picture_floor_bps(ceiling))
        })
    }

    /// Whether control and sound leave the pictures less than their floor
    /// of the user's ceiling (`picture_floor_bps`): no picture is made.
    pub fn picture_share_short(&self) -> bool {
        self.user_ceiling_bps.is_some_and(|ceiling| {
            ceiling.saturating_sub(self.control_bps) < picture_floor_bps(ceiling)
        })
    }

    /// `(control rate, ceiling)` once when the control stream alone came
    /// above the user's ceiling (a notice for the viewer), else `None`.
    pub fn control_over_ceiling(&mut self) -> Option<(u64, u64)> {
        let ceiling = self.user_ceiling_bps?;
        let over = self.picture_share_short();
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
        // The session's held-back control frames wait for the same budget
        // at the same rate (Codex 6/8); without a user ceiling for none.
        // Round 7 P2: with too little left for pictures, the held frames get
        // only that rest, not the pictures' floor (the sum keeps the ceiling).
        let rate = if self.picture_share_short() {
            self.user_ceiling_bps
                .map(|ceiling| ceiling.saturating_sub(self.control_bps).max(1))
        } else {
            self.user_ceiling_bps
                .and_then(|_| self.stream_budget_bps(now_us, video))
        };
        self.tiles.set_rate(rate);
        if self.picture_share_short() {
            return false;
        }
        self.stream_budget_bps(now_us, video)
            .is_none_or(|rate| self.tiles.admits(now_us, rate))
    }

    /// The byte budget the session's held-back control frames share with
    /// the pictures (`tiles_rate::SharedBudget`, Codex 6/8).
    pub fn shared_budget(&self) -> crate::screen::core::tiles_rate::SharedBudget {
        self.tiles.clone()
    }

    /// A picture's write runs until the returned guard drops (round 9 F4):
    /// the session's held control frames wait for it (`Outbox::ready`), also
    /// when the write is dropped half way.
    pub fn picture_write(&self) -> crate::screen::core::tiles_rate::PictureWrite {
        crate::screen::core::tiles_rate::PictureWrite::start(self.tiles.clone())
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
