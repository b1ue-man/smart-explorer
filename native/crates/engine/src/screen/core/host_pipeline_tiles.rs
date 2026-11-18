//! A stale picture stream of the lossless tiles (fix „Diashow“ 2026-11-18,
//! `tiles_rate::tile_stale`): it waits for its reliable stream instead of
//! a reset with a full picture; only a stream that does not move at all is
//! given up.
use super::*;
use crate::screen::core::tiles_rate::{tile_stale, TileStale};

impl Pipeline {
    /// `Pacer::decide` found the tile stream stale at `now_us` under
    /// `target_bps`. `Some`: the tick ends with it (the picture is dropped
    /// before encoding, as back pressure); `None`: the stream is reset
    /// as for every codec (`reset_stale_stream`).
    pub(super) fn tiles_stale(&mut self, now_us: u64, target_bps: u64) -> Option<Produced> {
        let (stale_us, _) = self.pacer.stale_limits(target_bps);
        match tile_stale(self.pacer.oldest_age_us(now_us), stale_us) {
            TileStale::Stuck => None,
            TileStale::Late => {
                // The path did not carry the target (halves it at most once
                // per stale time, `RateController::on_stale_stream`).
                self.rate.on_stale_stream(now_us, stale_us);
                self.pacer.on_budget_drop();
                Some(Produced::Dropped)
            }
            TileStale::Crowded => {
                self.rate.on_back_pressure(now_us);
                self.pacer.on_budget_drop();
                Some(Produced::Dropped)
            }
        }
    }
}
