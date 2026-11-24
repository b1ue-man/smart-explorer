//! The pace of a picture's send under the stream's byte budget (review
//! 2026-11-19, Codex 4/5 and the rounds after it; `GopWriter::paced`).

/// Bytes of one paced piece of a record: the budget is kept to about the
/// time this many bytes take at its rate.
pub(super) const PACE_CHUNK: usize = 16 * 1024;
/// A wait shorter than this is not taken (the timer's resolution); the
/// line gets at most this much ahead of the rate.
pub(super) const PACE_MIN_WAIT_US: u64 = 1_000;
/// The limit of a whole paced write (round 7 P3/P4): only a safety net; a
/// stream that takes no more bytes ends the write piece by piece
/// (`GopWriter::piece`), and a lowered ceiling only slows it.
pub const WRITE_SAFETY_LIMIT: std::time::Duration = std::time::Duration::from_secs(3600);
/// The longest rest of a picture after a lowered ceiling (round 8 R1): a
/// rest that takes longer at the new rate ends the picture's stream (the
/// next picture is a key frame sized for the new ceiling) instead of
/// crawling for minutes.
pub(super) const LOWERED_REST_US: u64 = 2_000_000;
/// The longest single wait (round 6 L2): a ceiling raised meanwhile, or
/// control traffic that sank, speeds the rest of the picture up within it.
pub(super) const PACE_MAX_WAIT_US: u64 = 100_000;

/// The byte budget of one frame's send (review 2026-11-19, Codex 4/5): its
/// records reach the stream no faster than `rate_bps` after the saved
/// `credit` bytes, so „Höchste Datenrate“ holds on the line too and the
/// budget is in no debt after a large picture (no pause after it).
#[derive(Clone, Debug)]
pub(super) struct Pace {
    pub(super) rate_bps: u64,
    /// The bytes earned up to `anchor` (the saved credit at the start),
    /// and the rate earning since then (Codex 7/5: a rate change never
    /// re-rates the time before it, neither into a pause nor into a burst).
    pub(super) banked: u128,
    pub(super) anchor: tokio::time::Instant,
    pub(super) anchor_rate: Option<u64>,
    pub(super) written: u64,
    /// Bytes of small control frames that went beyond their share while
    /// this picture was written (Codex 11/7, round 12): the picture's pace
    /// counts them as its own, so picture and frames together keep the
    /// rate over short spans; `shared` is where they come from.
    pub(super) absorbed: u64,
    pub(super) shared: Option<crate::screen::core::tiles_rate::SharedBudget>,
    /// The bytes of the whole frame (`write_records_admitted`; 0: unknown).
    pub(super) total: u64,
    /// The ceiling as it is now (0: none, Codex 5/12), the user ceiling the
    /// picture's rate was taken under (Codex 7/4), and the control and sound
    /// rate then.
    pub(super) limit: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    pub(super) ceiling_at_start: u64,
    pub(super) control_bps: u64,
    /// The rate the line was measured to carry: a piece the stream does not
    /// take within the stale time plus its time at it is a stall (Codex 6/9;
    /// the pace's own waits never count).
    pub(super) line_bps: u64,
    /// The picture's rate apart from the user's ceiling (round 12, Codex
    /// 12/14: `RateController::stream_free_bps`): a ceiling raised or
    /// removed during the write lets the rest go up to it. `rate_bps` when
    /// not given.
    pub(super) free_bps: u64,
}

impl Pace {
    /// The rate now: the picture's rate while the ceiling stays or rises
    /// (round 7 P1: a tile picture on a congested line keeps its lower
    /// rate); after a lowered ceiling, the picture's rate at most the new
    /// ceiling less control and sound, never 0 (round 7 P2: the pictures'
    /// floor would put pictures and control above the ceiling; a new picture
    /// below the floor is not made at all, `stream_admit`).
    /// Round 12 (Codex 12/14): a ceiling raised during the write gives the
    /// rest the rate apart from the ceiling (`free_bps`) up to the new
    /// ceiling less control and sound, a removed one `free_bps`; before, the
    /// rest kept the old ceiling's rate for seconds.
    pub(super) fn rate(&self) -> u64 {
        let live = self
            .limit
            .as_ref()
            .map_or(0, |cell| cell.load(std::sync::atomic::Ordering::Acquire));
        if self.ceiling_at_start != 0 && live == 0 {
            self.rate_bps.max(self.free_bps)
        } else if self.ceiling_at_start != 0 && live > self.ceiling_at_start {
            let share = live.saturating_sub(self.control_bps);
            self.rate_bps.max(self.free_bps.min(share))
        } else if !self.lowered(live) {
            self.rate_bps
        } else {
            // Round 8 R1: an admitted picture goes on at least at the
            // pictures' floor of the new ceiling (control and sound measured
            // before it may leave less, down to 1 bit/s, and the rest crawled
            // for hours); `too_slow` ends one whose rest takes too long.
            let floor = crate::screen::core::rate_control::picture_floor_bps(live);
            self.rate_bps
                .min(live.saturating_sub(self.control_bps).max(floor))
        }
    }

    /// Whether `live` is a ceiling below the one the picture's rate was
    /// taken under (or the first one).
    fn lowered(&self, live: u64) -> bool {
        live != 0 && (self.ceiling_at_start == 0 || live < self.ceiling_at_start)
    }

    /// The bytes on the line under this pace: the picture's own and the
    /// small frames' absorbed ones (`absorb`).
    pub(super) fn used(&self) -> u64 {
        self.written.saturating_add(self.absorbed)
    }

    /// Takes in the small frames' bytes sent during this picture's write
    /// (`SharedBudget::absorbed`, booked with the picture when it ends).
    pub(super) fn absorb(&mut self) {
        if let Some(shared) = &self.shared {
            self.absorbed = shared.absorbed();
        }
    }

    /// Whether the rest of the picture after a lowered ceiling takes longer
    /// than `LOWERED_REST_US` at `rate_bps` (round 8 R1).
    pub(super) fn too_slow(&self, rate_bps: u64) -> bool {
        // Round 9 F7: only a ceiling that lowers the picture's own rate; a
        // first ceiling above it (from „Automatisch“) changes nothing.
        if rate_bps >= self.rate_bps || self.total == 0 {
            return false;
        }
        let rest = u128::from(self.total.saturating_sub(self.written));
        rest * 8 * 1_000_000 / u128::from(rate_bps.max(1)) > u128::from(LOWERED_REST_US)
    }

    /// The rate now and the bytes allowed on the line by now: what earlier
    /// stretches earned at their own rates plus the current stretch at the
    /// rate now.
    pub(super) fn allowed(&mut self) -> (u64, u128) {
        let rate = self.rate().max(1);
        if self.anchor_rate != Some(rate) {
            // The first stretch runs from the pace's start. What a stretch
            // earned and the line did not use ends with it (a stretch without
            // a ceiling earned without bound); the saved credit stays.
            let before = self.anchor_rate.unwrap_or(self.rate_bps);
            let earned_before = match self.anchor_rate {
                Some(before) => earned(before, self.anchor),
                None => 0,
            };
            // Gemini 8/10: a lowered rate keeps of the saved credit at most
            // its own burst window, so the old rate's credit does not go on
            // the line at once after a lowered ceiling.
            let saved = if rate < before {
                let window = u128::from(rate)
                    * u128::from(crate::screen::core::tiles_rate::TILE_BURST_US)
                    / 8
                    / 1_000_000;
                self.banked.min(window)
            } else {
                self.banked
            };
            if self.anchor_rate.is_some() || rate < before {
                let cap = u128::from(self.used()).max(saved);
                self.banked = self.banked.saturating_add(earned_before).min(cap);
            }
            if self.anchor_rate.is_some() {
                self.anchor = tokio::time::Instant::now();
            }
            self.anchor_rate = Some(rate);
        }
        (rate, self.banked.saturating_add(earned(rate, self.anchor)))
    }
}

/// The bytes `rate_bps` earns from `since` to now.
fn earned(rate_bps: u64, since: tokio::time::Instant) -> u128 {
    let micros = u128::from(u64::try_from(since.elapsed().as_micros()).unwrap_or(u64::MAX));
    u128::from(rate_bps) * micros / 8 / 1_000_000
}

#[cfg(test)]
mod r12_tests {
    use super::Pace;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn pace(rate_bps: u64, free_bps: u64, cell: &Arc<AtomicU64>) -> Pace {
        Pace {
            rate_bps,
            banked: 0,
            anchor: tokio::time::Instant::now(),
            anchor_rate: None,
            written: 0,
            absorbed: 0,
            shared: None,
            total: 300_000,
            limit: Some(cell.clone()),
            ceiling_at_start: 200_000,
            control_bps: 50_000,
            line_bps: 1,
            free_bps,
        }
    }

    /// Round 12 (Codex 12/14): a video picture started under 200 kbit/s
    /// (150 kbit/s for it, 50 kbit/s control) goes on at the new ceiling less
    /// control when the ceiling rises, unpaced when it is removed; a lowered
    /// ceiling slows it as before; the tiles on a congested path keep their
    /// rate (round 7 P1).
    #[test]
    fn r12_task_raised_ceiling_speeds_the_rest_of_a_picture_up() {
        let cell = Arc::new(AtomicU64::new(200_000));
        let video = pace(150_000, u64::MAX, &cell);
        assert_eq!(video.rate(), 150_000);
        cell.store(1_000_000, Ordering::Release);
        assert_eq!(video.rate(), 950_000);
        cell.store(0, Ordering::Release);
        assert_eq!(video.rate(), u64::MAX);
        cell.store(100_000, Ordering::Release);
        assert_eq!(video.rate(), 50_000);
        let congested = pace(150_000, 150_000, &cell);
        cell.store(1_000_000, Ordering::Release);
        assert_eq!(congested.rate(), 150_000);
        let codec = pace(150_000, 600_000, &cell);
        assert_eq!(codec.rate(), 600_000);
    }
}
