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
pub struct SharedBudget(
    std::sync::Arc<std::sync::Mutex<(TileBudget, Option<u64>)>>,
    std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// A picture is being written; it is booked when its write ends, so
    /// held control frames wait meanwhile (round 9 F4: they took the credit
    /// the picture was about to use, briefly up to twice the ceiling).
    std::sync::Arc<Turns>,
);

/// Whose turn the line is between pictures and held control frames (round
/// 9 F4, round 10): a picture being written holds the frames back, and a
/// frame held back by a picture gets one piece before the next picture.
#[derive(Debug, Default)]
pub struct Turns {
    /// The control rate the pictures leave free (`set_reserved`).
    reserved: std::sync::atomic::AtomicU64,
    /// Small frames' excess sent while a picture is written (`absorb_small`).
    absorbed: std::sync::atomic::AtomicU64,
    writing: std::sync::atomic::AtomicBool,
    owed: std::sync::atomic::AtomicBool,
    changed: tokio::sync::Notify,
}

/// The longest wait for the other side's turn: a missed wake-up never
/// stalls either side, and a picture never waits longer for control.
const TURN_WAIT: std::time::Duration = std::time::Duration::from_millis(20);

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
    /// A held-back control frame of `bytes` went at `at_us`: booked like a
    /// picture and counted for the statistics' sent rate (`take_held_bytes`).
    pub fn on_held_sent(&self, at_us: u64, bytes: u64, rate_bps: u64) {
        self.on_sent(at_us, bytes, rate_bps);
        self.1.fetch_add(bytes, std::sync::atomic::Ordering::AcqRel);
    }
    /// Under the budget's lock (Codex 10/9): a held piece books and checks
    /// the flag in one step (`take_held_piece`), so no piece takes credit a
    /// picture has started on.
    pub fn set_picture_writing(&self, writing: bool) {
        self.with(|_| {
            self.2
                .writing
                .store(writing, std::sync::atomic::Ordering::Release);
        });
        self.2.changed.notify_waiters();
    }
    /// A held control piece of `bytes` takes its credit at `at_us` unless a
    /// picture is being written (then `false`, nothing booked; Codex 10/9).
    pub fn take_held_piece(&self, at_us: u64, bytes: u64, rate_bps: u64) -> bool {
        let taken = self.with(|(budget, _)| {
            if self.2.writing.load(std::sync::atomic::Ordering::Acquire) {
                return false;
            }
            budget.on_sent(at_us, bytes, rate_bps);
            true
        });
        if taken {
            self.1.fetch_add(bytes, std::sync::atomic::Ordering::AcqRel);
        }
        taken
    }
    /// The control stream's measured rate, which the pictures leave free
    /// (`RateController::user_picture_bps`); small control frames beyond it
    /// take this budget (Codex 11/7).
    pub fn set_reserved(&self, bps: u64) {
        self.2
            .reserved
            .store(bps, std::sync::atomic::Ordering::Release);
    }
    /// A small frame's `bytes` beyond its share went while a picture is
    /// written (Codex 11/7, round 12): its pace counts them (`take_absorbed`).
    pub fn absorb_small(&self, bytes: u64) {
        self.2
            .absorbed
            .fetch_add(bytes, std::sync::atomic::Ordering::AcqRel);
    }
    /// The absorbed bytes since the last call.
    pub fn take_absorbed(&self) -> u64 {
        self.2.absorbed.swap(0, std::sync::atomic::Ordering::AcqRel)
    }
    pub fn reserved(&self) -> u64 {
        self.2.reserved.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn picture_writing(&self) -> bool {
        self.2.writing.load(std::sync::atomic::Ordering::Acquire)
    }
    /// Held control frames wait for the picture being written; the next
    /// picture leaves them one piece first (`control_turn`).
    pub async fn wait_picture(&self) {
        self.2
            .owed
            .store(true, std::sync::atomic::Ordering::Release);
        while self.picture_writing() {
            let _ = tokio::time::timeout(TURN_WAIT, self.2.changed.notified()).await;
        }
    }
    /// A held control piece went: the picture may go.
    pub fn control_went(&self) {
        if self.2.owed.swap(false, std::sync::atomic::Ordering::AcqRel) {
            self.2.changed.notify_waiters();
        }
    }
    /// Before a picture's write: a control frame the last picture held back
    /// gets its piece first, for at most `TURN_WAIT` (no frame starves, and
    /// no picture waits long for control).
    pub async fn control_turn(&self) {
        if self.2.owed.load(std::sync::atomic::Ordering::Acquire) {
            let _ = tokio::time::timeout(TURN_WAIT, self.2.changed.notified()).await;
            self.2
                .owed
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }
    /// The bytes of held-back control frames sent since the last call.
    pub fn take_held_bytes(&self) -> u64 {
        self.1.swap(0, std::sync::atomic::Ordering::AcqRel)
    }
}

/// A picture's write in the shared budget (`RateController::picture_write`).
pub struct PictureWrite(SharedBudget);

impl PictureWrite {
    pub fn start(budget: SharedBudget) -> Self {
        budget.set_picture_writing(true);
        // Bytes absorbed by an earlier picture's end are booked already.
        let _ = budget.take_absorbed();
        Self(budget)
    }
}

impl Drop for PictureWrite {
    fn drop(&mut self) {
        self.0.set_picture_writing(false);
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
