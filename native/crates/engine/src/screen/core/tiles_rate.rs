//! The lossless tiles within the target E (plan 2026-11-18 „Netzrate“, M1).
//!
//! The tile codec has no rate the encoder could aim at: a picture costs what
//! its changed tiles cost. Before, only the queue guard of the pacer held it
//! back, and on a line that drains fast it sent 34–57 Mbit/s at 1080p even
//! under a „Höchste Datenrate“ of 650 kbit/s. The user's rule: the highest
//! data rate is a hard ceiling for every codec, mode and fallback, and „so
//! schnell wie möglich“ means as fast as the real connection carries.
//!
//! `TileBudget` is a byte budget derived from E = min(L, user ceiling, codec
//! ceiling) (`RateController::target_bps`): every picture that was sent
//! occupies the line for `bytes × 8 / E` seconds; the next picture is taken
//! only once that time has passed. A picture that overdraws the budget (a
//! large key frame) is sent whole and moves the next one back. An idle line
//! saves at most `TILE_BURST_US` of credit, so over any interval the bytes
//! sent stay within E × (interval + burst) plus one picture. Nothing is
//! lost while a picture waits: the next capture is compared with what the
//! encoder sent last, and the dirty tiles of pictures the host process
//! dropped mark every tile (`tiles_dirty.rs`).

/// The credit an idle line may save: 200 ms at the target.
pub const TILE_BURST_US: u64 = 200_000;

/// When the next picture of the tile codec may be taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileBudget {
    /// The time the line is free again; `None` before the first picture.
    free_us: Option<u64>,
}

impl TileBudget {
    /// Whether a picture may be captured and encoded at `now_us`.
    pub fn admits(&self, now_us: u64) -> bool {
        self.free_us.is_none_or(|free| now_us >= free)
    }

    /// Microseconds until the next picture may be taken (0: now).
    pub fn wait_us(&self, now_us: u64) -> u64 {
        self.free_us.map_or(0, |free| free.saturating_sub(now_us))
    }

    /// A picture of `bytes` was sent at `now_us` under the target
    /// `target_bps`: the line is busy with it for its serialization time.
    pub fn on_sent(&mut self, now_us: u64, bytes: u64, target_bps: u64) {
        let cost_us = (u128::from(bytes) * 8 * 1_000_000 / u128::from(target_bps.max(1)))
            .min(u128::from(u64::MAX)) as u64;
        let start = self.free_us.map_or(now_us, |free| {
            free.max(now_us.saturating_sub(TILE_BURST_US))
        });
        self.free_us = Some(start.saturating_add(cost_us));
    }
}
