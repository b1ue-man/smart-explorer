//! The lossless tiles within the ceilings and the measured network limit
//! (plan 2026-11-18 „Netzrate“, M1; fix „Diashow“ of 2026-11-18).
//!
//! The tile codec has no rate the encoder could aim at: a picture costs what
//! its changed tiles cost. Before M1, only the queue guard of the pacer held
//! it back, and on a line that drains fast it sent 34–57 Mbit/s at 1080p
//! even under a „Höchste Datenrate“ of 650 kbit/s. The user's rule: the
//! highest data rate is a hard ceiling for every codec, mode and fallback,
//! and „so schnell wie möglich“ means as fast as the real connection carries.
//!
//! `TileBudget` is a token bucket of bits (RFC 2697/2698 style): credit
//! grows at the budget rate up to `TILE_BURST_US` of it, every sent picture
//! takes its bits, and a picture larger than the credit is sent whole and
//! leaves a debt that the following time pays off at the rate (spread over
//! time, never booked twice): the next picture waits only until the debt is
//! paid. A picture is booked at the start of its send, so the time its send
//! took is not charged again after it.
//!
//! The budget rate (`RateController::tiles_budget_bps`) is the ceiling C =
//! min(user ceiling, codec ceiling) and, while the path signalled
//! congestion within `CONGESTION_HOLD_US` (persistent delay overuse, a stale
//! stream, loss above 10 %), the target E. M1 used E always; for the tiles
//! E is the start table of the stream and cannot grow while their content
//! offers less than half of it (`rate_model::grown`), so on a good line
//! every full picture froze the stream for size/E (measured 2026-11-18:
//! 3 pictures/s at „Ziel 20414“ where 0.5.199 sent 137/s). Without
//! congestion the line is held by the queue guard as in 0.5.199
//! (`Pacer::guard`, delivered rate over the round-trip floor). Nothing is
//! lost while a picture waits: the next capture is compared with what the
//! encoder sent last, and the dirty tiles of pictures the host process
//! dropped mark every tile (`tiles_dirty.rs`).

/// The credit an idle line may save: 200 ms at the budget rate.
pub const TILE_BURST_US: u64 = 200_000;
/// How long a congestion signal of the path holds the tiles at E.
pub const CONGESTION_HOLD_US: u64 = 1_000_000;
/// Age of the oldest unacknowledged tile picture after which its stream is
/// given up after all (`tile_stale`): the stream is reliable, so only a
/// stream that does not move at all within this time is reset.
pub const TILE_STUCK_US: u64 = 30_000_000;

/// When the next picture of the tile codec may be taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileBudget {
    /// The last booking: its time and the balance of bits then (credit
    /// positive, debt negative); `None` before the first picture.
    booked: Option<(u64, i128)>,
    /// The last congestion signal of the path.
    congested_us: Option<u64>,
    /// The last loss reading was above 10 % (until the next reading).
    loss_limited: bool,
}

/// Bits `rate_bps` carries in `micros`.
fn bits(rate_bps: u64, micros: u64) -> i128 {
    i128::from(rate_bps).saturating_mul(i128::from(micros)) / 1_000_000
}

impl TileBudget {
    /// The balance at `now_us` under `rate_bps`: the credit is full before
    /// the first picture and never more than `TILE_BURST_US` of the rate.
    fn balance(&self, now_us: u64, rate_bps: u64) -> i128 {
        let cap = bits(rate_bps, TILE_BURST_US);
        match self.booked {
            None => cap,
            Some((at, balance)) => balance
                .saturating_add(bits(rate_bps, now_us.saturating_sub(at)))
                .min(cap),
        }
    }

    /// The saved credit at `now_us` under `rate_bps` in bits (0 in debt):
    /// what a picture may put on the line at once before its send is paced
    /// (`GopWriter::pace_next_frame`).
    pub fn credit_bits(&self, now_us: u64, rate_bps: u64) -> u64 {
        u64::try_from(self.balance(now_us, rate_bps).max(0)).unwrap_or(u64::MAX)
    }

    /// Whether a picture may be captured and encoded at `now_us`.
    pub fn admits(&self, now_us: u64, rate_bps: u64) -> bool {
        self.balance(now_us, rate_bps) >= 0
    }

    /// Microseconds until the next picture may be taken (0: now).
    pub fn wait_us(&self, now_us: u64, rate_bps: u64) -> u64 {
        let balance = self.balance(now_us, rate_bps);
        if balance >= 0 {
            return 0;
        }
        let debt = balance.unsigned_abs().saturating_mul(1_000_000);
        let micros = debt.div_ceil(u128::from(rate_bps.max(1)));
        u64::try_from(micros).unwrap_or(u64::MAX)
    }

    /// A picture of `bytes` started its send at `at_us` under `rate_bps`.
    pub fn on_sent(&mut self, at_us: u64, bytes: u64, rate_bps: u64) {
        let at = self.booked.map_or(at_us, |(last, _)| at_us.max(last));
        let balance = self.balance(at, rate_bps);
        let cost = i128::from(bytes).saturating_mul(8);
        self.booked = Some((at, balance.saturating_sub(cost)));
    }

    /// The path signalled congestion at `now_us` (persistent delay overuse,
    /// a stale stream that lowered the target).
    pub fn on_congestion(&mut self, now_us: u64) {
        self.congested_us = Some(now_us);
    }

    /// The loss of the last reading was above 10 % (or no longer is).
    pub fn set_loss_limited(&mut self, limited: bool) {
        self.loss_limited = limited;
    }

    /// Whether the target E holds the tiles at `now_us`.
    pub fn network_limited(&self, now_us: u64) -> bool {
        self.loss_limited
            || self
                .congested_us
                .is_some_and(|at| now_us.saturating_sub(at) <= CONGESTION_HOLD_US)
    }
}

/// The stream's byte budget, shared by the pictures (the pipeline's rate
/// control) and the large control frames the session holds back for it
/// (review 2026-11-19, Codex 6/8: window lists, icons, family lists and
/// pointer shapes wait for the same budget as the pictures, so their sum
/// keeps „Höchste Datenrate“). `rate` is the budget's rate as the pipeline
/// last set it; `None` without a user ceiling (nothing waits then).
#[derive(Clone, Debug, Default)]
pub struct SharedBudget(std::sync::Arc<std::sync::Mutex<(TileBudget, Option<u64>)>>);

impl SharedBudget {
    fn with<R>(&self, f: impl FnOnce(&mut (TileBudget, Option<u64>)) -> R) -> R {
        f(&mut self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }
    pub fn admits(&self, now_us: u64, rate_bps: u64) -> bool {
        self.with(|(budget, _)| budget.admits(now_us, rate_bps))
    }
    pub fn wait_us(&self, now_us: u64, rate_bps: u64) -> u64 {
        self.with(|(budget, _)| budget.wait_us(now_us, rate_bps))
    }
    pub fn on_sent(&self, at_us: u64, bytes: u64, rate_bps: u64) {
        self.with(|(budget, _)| budget.on_sent(at_us, bytes, rate_bps));
    }
    pub fn credit_bits(&self, now_us: u64, rate_bps: u64) -> u64 {
        self.with(|(budget, _)| budget.credit_bits(now_us, rate_bps))
    }
    pub fn on_congestion(&self, now_us: u64) {
        self.with(|(budget, _)| budget.on_congestion(now_us));
    }
    pub fn set_loss_limited(&self, limited: bool) {
        self.with(|(budget, _)| budget.set_loss_limited(limited));
    }
    pub fn network_limited(&self, now_us: u64) -> bool {
        self.with(|(budget, _)| budget.network_limited(now_us))
    }
    /// The budget's rate for the held-back control frames; `None` lets
    /// them go at once.
    pub fn set_rate(&self, rate_bps: Option<u64>) {
        self.with(|(_, rate)| *rate = rate_bps);
    }
    pub fn rate(&self) -> Option<u64> {
        self.with(|(_, rate)| *rate)
    }
}

/// What a tile stream whose oldest picture is past its stale time does
/// (fix „Diashow“ 2026-11-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileStale {
    /// The picture waits beyond the time the line needs for the bytes in
    /// flight at the target: the path does not carry the target; the
    /// picture is dropped before encoding and the target sinks.
    Late,
    /// More pictures are in flight than the cadence sends within the stale
    /// time, none of them late: back pressure only.
    Crowded,
    /// Nothing moved for `TILE_STUCK_US`: the stream is given up.
    Stuck,
}

/// The tiles never reset a stream that is merely slow: its frames arrive on
/// the reliable QUIC stream, and the reset would throw away the bytes in
/// flight and send every tile again (a full picture, many times the size of
/// the changed tiles), which went stale in turn — a reset per stale time,
/// each one a full picture. `oldest_age_us` is the age of the oldest
/// unacknowledged picture, `stale_us` its stale time (`Pacer::stale_limits`).
pub fn tile_stale(oldest_age_us: Option<u64>, stale_us: u64) -> TileStale {
    match oldest_age_us {
        Some(age) if age > TILE_STUCK_US => TileStale::Stuck,
        Some(age) if age > stale_us => TileStale::Late,
        _ => TileStale::Crowded,
    }
}

#[cfg(test)]
#[path = "diashowfix_task_tests.rs"]
mod diashowfix_task_tests;
