//! The byte budget of the lossless tiles in the rate control
//! (`tiles_rate.rs`; plan 2026-11-18 „Netzrate“ M1, fix „Diashow“).
use super::RateController;

impl RateController {
    /// The rate the tiles' budget pays off at, at `now_us`: the ceiling
    /// (user's „Höchste Datenrate“, codec pairing) and, while the path
    /// signalled congestion (`TileBudget::network_limited`), the target E.
    /// Otherwise the queue guard holds the tiles to what the line delivers.
    pub fn tiles_budget_bps(&self, now_us: u64) -> u64 {
        if self.tiles.network_limited(now_us) {
            self.target_bps()
        } else {
            self.ceiling_bps()
        }
    }

    /// Whether a picture of the tiles may be taken at `now_us`.
    pub fn tiles_admit(&self, now_us: u64) -> bool {
        self.tiles.admits(now_us, self.tiles_budget_bps(now_us))
    }

    /// A picture of the tiles of `bytes` started its send at `at_us`.
    pub fn on_tiles_sent(&mut self, at_us: u64, bytes: u64) {
        let rate = self.tiles_budget_bps(at_us);
        self.tiles.on_sent(at_us, bytes, rate);
    }
}
