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
    pub(super) credit: u64,
    pub(super) started: tokio::time::Instant,
    pub(super) written: u64,
    /// The ceiling as it is now (0: none, Codex 5/12), the ceiling when
    /// the picture started, and the control and sound rate then.
    pub(super) limit: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    pub(super) ceiling_at_start: u64,
    pub(super) control_bps: u64,
    /// The rate the line was measured to carry: a piece the stream does not
    /// take within the stale time plus its time at it is a stall (Codex 6/9;
    /// the pace's own waits never count).
    pub(super) line_bps: u64,
}

impl Pace {
    /// The rate now: the picture's rate while the ceiling stays or rises
    /// (round 7 P1: a tile picture on a congested line keeps its lower
    /// rate); after a lowered ceiling, the picture's rate at most the new
    /// ceiling less control and sound, never 0 (round 7 P2: the pictures'
    /// floor would put pictures and control above the ceiling; a new picture
    /// below the floor is not made at all, `stream_admit`).
    pub(super) fn rate(&self) -> u64 {
        let live = self
            .limit
            .as_ref()
            .map_or(0, |cell| cell.load(std::sync::atomic::Ordering::Acquire));
        if live == 0 || live >= self.ceiling_at_start && self.ceiling_at_start != 0 {
            self.rate_bps
        } else {
            self.rate_bps
                .min(live.saturating_sub(self.control_bps).max(1))
        }
    }
}
