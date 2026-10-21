//! Parts of one large file as separate operations: when a file is split, how
//! long each part is, and which parts are done. Pure logic; the engine
//! (`os/shared/engine/segments*.rs`) runs the parts under the flow's permits.
use std::collections::BTreeMap;

/// Shortest part. Setting a part up costs three round trips (open, first
/// answer, close); at 8 MiB and 12.5 MB/s a part lasts 0.67 s against 0.15 s
/// of set-up. Drive and rclone use 8 MiB blocks, gcloud and Nextcloud at
/// least 5 MiB; on rotating disks 8 MiB written side by side spread the head
/// jumps over ~56 ms of writing against ~10 ms of seeking.
pub(crate) const MIN_SEGMENT: u64 = 8 * 1024 * 1024;
/// Smallest file that is split: two shortest parts.
pub(crate) const SPLIT_FROM: u64 = 2 * MIN_SEGMENT;
/// A part may spend at most a tenth of its time on set-up.
const SETUP_SHARE: f64 = 10.0;
/// The part repeated after a failure stays under this many seconds of one
/// stream.
const MAX_SEGMENT_SECONDS: f64 = 30.0;
/// Weight of the newest measurement (like `BatchSizer::record`).
const NEWEST_WEIGHT: f64 = 0.3;

/// Whether a file of `size` bytes is split into parts.
pub(crate) fn splits(size: u64) -> bool {
    size >= SPLIT_FROM
}

/// Length of the next part from what the finished parts of this connection
/// measured: time until the first byte (set-up) and stream rate after it.
#[derive(Debug, Default)]
pub(crate) struct SegmentSizer {
    setup_ms: Option<f64>,
    rate_bps: Option<f64>,
}

impl SegmentSizer {
    /// A finished part of `bytes`: first byte after `first_byte_ms`, last
    /// byte after `total_ms`. Without time after the first byte there is no
    /// rate to learn from.
    pub(crate) fn record(&mut self, bytes: u64, first_byte_ms: u64, total_ms: u64) {
        if bytes == 0 || total_ms <= first_byte_ms {
            return;
        }
        let rate = bytes as f64 * 1000.0 / (total_ms - first_byte_ms) as f64;
        self.rate_bps = Some(blend(self.rate_bps, rate));
        self.setup_ms = Some(blend(self.setup_ms, first_byte_ms as f64));
    }

    /// `clamp(10 × set-up × rate, MIN_SEGMENT, max(MIN_SEGMENT, 30 s × rate))`;
    /// `MIN_SEGMENT` without measurements. Never more than `remaining`, and
    /// a rest shorter than `MIN_SEGMENT` goes into this part.
    pub(crate) fn next_len(&self, remaining: u64) -> u64 {
        let wanted = match (self.setup_ms, self.rate_bps) {
            (Some(setup_ms), Some(rate)) => {
                let longest = (MAX_SEGMENT_SECONDS * rate).max(MIN_SEGMENT as f64);
                (SETUP_SHARE * setup_ms / 1000.0 * rate).clamp(MIN_SEGMENT as f64, longest) as u64
            }
            _ => MIN_SEGMENT,
        };
        let wanted = wanted.min(remaining);
        if remaining - wanted < MIN_SEGMENT {
            remaining
        } else {
            wanted
        }
    }
}

fn blend(previous: Option<f64>, newest: f64) -> f64 {
    match previous {
        Some(previous) => previous * (1.0 - NEWEST_WEIGHT) + newest * NEWEST_WEIGHT,
        None => newest,
    }
}

/// Which parts of a file were handed out, are running, or failed.
#[derive(Debug)]
pub(crate) struct Ledger {
    size: u64,
    next: u64,
    /// Start → length of the parts handed out and not finished.
    in_flight: BTreeMap<u64, u64>,
    /// Smallest start of a part that failed: nothing from there on counts as
    /// written.
    failed_from: Option<u64>,
}

impl Ledger {
    /// A file of `size` bytes whose first `resume_at` bytes are written
    /// already (the safe start of an earlier attempt).
    pub(crate) fn new(size: u64, resume_at: u64) -> Self {
        Self {
            size,
            next: resume_at.min(size),
            in_flight: BTreeMap::new(),
            failed_from: None,
        }
    }

    /// The next part from `next` on, measured by `sizer`; `None` once every
    /// byte is handed out.
    pub(crate) fn claim(&mut self, sizer: &SegmentSizer) -> Option<(u64, u64)> {
        if self.next >= self.size {
            return None;
        }
        let start = self.next;
        let len = sizer.next_len(self.size - start);
        self.in_flight.insert(start, len);
        self.next = start + len;
        Some((start, len))
    }

    /// The part starting at `start` is written completely.
    pub(crate) fn complete(&mut self, start: u64) {
        self.in_flight.remove(&start);
    }

    /// The part starting at `start` stopped before its end (error, abort):
    /// it is not done, and `prefix` stays before it.
    pub(crate) fn release_failed(&mut self, start: u64) {
        self.in_flight.remove(&start);
        self.failed_from = Some(self.failed_from.map_or(start, |known| known.min(start)));
    }

    /// The safe start: every byte before it is written. A part that is still
    /// running or failed holds it back.
    pub(crate) fn prefix(&self) -> u64 {
        let running = self.in_flight.keys().next().copied();
        match (running, self.failed_from) {
            (Some(running), Some(failed)) => running.min(failed),
            (Some(only), None) | (None, Some(only)) => only,
            (None, None) => self.next,
        }
    }

    /// Whether a part is still waiting to be handed out.
    pub(crate) fn has_unclaimed(&self) -> bool {
        self.next < self.size
    }

    /// Whether no part is running (finished or failed ones are not).
    pub(crate) fn in_flight_empty(&self) -> bool {
        self.in_flight.is_empty()
    }

    /// Every byte is handed out, finished, and no part failed.
    pub(crate) fn all_done(&self) -> bool {
        self.next == self.size && self.in_flight.is_empty() && self.failed_from.is_none()
    }
}
