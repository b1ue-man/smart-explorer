//! Live numbers of one job, updated by discovery and workers without locks on
//! the hot path, and turned into a `TransferProgress` by the dispatcher.
use super::super::engine_policy::RateWindow;
use super::super::types::{TransferKind, TransferProgress};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Names of running files shown per transfer (spec: up to three).
const ACTIVE_SHOWN: usize = 3;

pub(crate) struct Stats {
    started: Instant,
    files_total: AtomicU64,
    bytes_total: AtomicU64,
    files_done: AtomicU64,
    bytes_done: AtomicU64,
    /// Every byte moved, retried ones included: the rate's measure.
    bytes_moved: AtomicU64,
    skipped: AtomicU64,
    omitted: AtomicU64,
    running: AtomicU64,
    /// Highest `bytes_done` so far: bytes beyond it (or a finished file) are
    /// progress, bytes a retry sends again are not.
    high_water: AtomicU64,
    /// When the job last made progress (ms since `started`).
    last_progress_ms: AtomicU64,
    discovering: AtomicBool,
    next_token: AtomicU64,
    active: Mutex<Vec<(u64, String)>>,
    rate: Mutex<RateWindow>,
}

/// A running file; its name leaves the list when this is dropped.
pub(crate) struct ActiveFile<'a> {
    stats: &'a Stats,
    token: u64,
}

impl Drop for ActiveFile<'_> {
    fn drop(&mut self) {
        self.stats.running.fetch_sub(1, Ordering::AcqRel);
        self.stats
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(token, _)| *token != self.token);
    }
}

impl Stats {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            files_total: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            files_done: AtomicU64::new(0),
            bytes_done: AtomicU64::new(0),
            bytes_moved: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            omitted: AtomicU64::new(0),
            running: AtomicU64::new(0),
            high_water: AtomicU64::new(0),
            last_progress_ms: AtomicU64::new(0),
            discovering: AtomicBool::new(true),
            next_token: AtomicU64::new(1),
            active: Mutex::new(Vec::new()),
            rate: Mutex::new(RateWindow::default()),
        }
    }

    pub(crate) fn found(&self, size: u64) {
        self.files_total.fetch_add(1, Ordering::AcqRel);
        self.bytes_total.fetch_add(size, Ordering::AcqRel);
    }

    /// A found file turned out `new` bytes long instead of `old`.
    pub(crate) fn resized(&self, old: u64, new: u64) {
        let _ = self
            .bytes_total
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |total| {
                Some(total.saturating_sub(old).saturating_add(new))
            });
    }

    pub(crate) fn discovery_finished(&self) {
        self.discovering.store(false, Ordering::Release);
    }

    pub(crate) fn omitted(&self) {
        self.omitted.fetch_add(1, Ordering::AcqRel);
    }

    /// Streamed bytes of a running file.
    pub(crate) fn moved(&self, bytes: u64) {
        let done = self
            .bytes_done
            .fetch_add(bytes, Ordering::AcqRel)
            .saturating_add(bytes);
        self.bytes_moved.fetch_add(bytes, Ordering::AcqRel);
        if self.high_water.fetch_max(done, Ordering::AcqRel) < done {
            self.progressed();
        }
    }

    /// Bytes delivered by an earlier attempt that a resumed one keeps.
    pub(crate) fn credit(&self, bytes: u64) {
        self.bytes_done.fetch_add(bytes, Ordering::AcqRel);
    }

    /// A failed attempt gives back the bytes it counted (the file is not
    /// done); never more than were counted.
    pub(crate) fn unmoved(&self, bytes: u64) {
        let _ = self
            .bytes_done
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |done| {
                Some(done.saturating_sub(bytes))
            });
    }

    pub(crate) fn file_done(&self) {
        self.files_done.fetch_add(1, Ordering::AcqRel);
        self.progressed();
    }

    /// A folder moved as a whole counts as one entry done.
    pub(crate) fn entry_moved_whole(&self) {
        self.files_total.fetch_add(1, Ordering::AcqRel);
        self.files_done.fetch_add(1, Ordering::AcqRel);
        self.progressed();
    }

    pub(crate) fn skipped(&self) {
        self.skipped.fetch_add(1, Ordering::AcqRel);
        self.progressed();
    }

    fn progressed(&self) {
        self.last_progress_ms
            .fetch_max(self.elapsed_ms(), Ordering::AcqRel);
    }

    /// How long the job has not moved since `since` (from the later of
    /// `since` and its last progress).
    pub(crate) fn quiet_since(&self, since: Instant) -> Duration {
        let last =
            self.started + Duration::from_millis(self.last_progress_ms.load(Ordering::Acquire));
        since.max(last).elapsed()
    }

    pub(crate) fn running(&self) -> usize {
        self.running.load(Ordering::Acquire) as usize
    }

    pub(crate) fn begin(&self, name: &str) -> ActiveFile<'_> {
        let token = self.next_token.fetch_add(1, Ordering::AcqRel);
        self.running.fetch_add(1, Ordering::AcqRel);
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((token, name.to_string()));
        ActiveFile { stats: self, token }
    }

    pub(crate) fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// The numbers as a progress value (the dispatcher adds the note).
    pub(crate) fn snapshot(&self, kind: TransferKind, errors: u64) -> TransferProgress {
        let elapsed_ms = self.elapsed_ms();
        let rate_bps = {
            let mut rate = self
                .rate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            rate.record(elapsed_ms, self.bytes_moved.load(Ordering::Acquire));
            rate.rate()
        };
        let active = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .take(ACTIVE_SHOWN)
            .map(|(_, name)| name.clone())
            .collect::<Vec<_>>();
        let mut progress = TransferProgress::new(
            kind,
            kind.label(),
            self.files_total.load(Ordering::Acquire),
            self.bytes_total.load(Ordering::Acquire),
        );
        progress.current = active.first().cloned().unwrap_or_default();
        progress.files_done = self.files_done.load(Ordering::Acquire);
        progress.bytes_done = self.bytes_done.load(Ordering::Acquire);
        progress.elapsed_ms = elapsed_ms;
        progress.errors = errors;
        progress.omitted = self.omitted.load(Ordering::Acquire);
        progress.discovering = self.discovering.load(Ordering::Acquire);
        progress.skipped = self.skipped.load(Ordering::Acquire);
        progress.rate_bps = rate_bps;
        progress.active = active;
        progress.parallel = self
            .running
            .load(Ordering::Acquire)
            .min(u64::from(u32::MAX)) as u32;
        progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_engine_task_stats_give_back_only_counted_bytes() {
        let stats = Stats::new();
        stats.found(100);
        stats.moved(10);
        stats.unmoved(25);
        let progress = stats.snapshot(TransferKind::Upload, 0);
        assert_eq!(progress.bytes_done, 0, "saturates instead of wrapping");
        stats.resized(100, 40);
        assert_eq!(stats.snapshot(TransferKind::Upload, 0).bytes_total, 40);
    }

    #[test]
    fn transfer_engine_task_stats_progress_is_new_bytes_or_a_finished_file() {
        let stats = Stats::new();
        stats.moved(10);
        let after_first = stats.last_progress_ms.load(Ordering::Acquire);
        std::thread::sleep(Duration::from_millis(30));
        // A retry sends the same bytes again: no progress.
        stats.unmoved(10);
        stats.moved(10);
        assert_eq!(stats.last_progress_ms.load(Ordering::Acquire), after_first);
        stats.moved(1);
        assert!(stats.last_progress_ms.load(Ordering::Acquire) > after_first);
        let waiting_since = Instant::now();
        std::thread::sleep(Duration::from_millis(30));
        assert!(stats.quiet_since(waiting_since) >= Duration::from_millis(30));
        stats.file_done();
        assert!(stats.quiet_since(waiting_since) < Duration::from_millis(30));
    }
}
