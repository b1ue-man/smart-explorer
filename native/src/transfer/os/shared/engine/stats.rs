//! Live numbers of one job, updated by discovery and workers without locks on
//! the hot path, and turned into a `TransferProgress` by the dispatcher.
use super::super::engine_policy::RateWindow;
use super::super::limiter::OpAccounts;
use super::super::types::{TransferKind, TransferProgress};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Names of running files shown per transfer (spec: up to three).
const ACTIVE_SHOWN: usize = 3;

/// What the file rule "neuere ersetzen ältere" decided for one file; the
/// counters of `TransferProgress` (`replaced`, `skipped_equal`, `kept_newer`,
/// `kept_both`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Replaced = 0,
    Equal = 1,
    TargetNewer = 2,
    Both = 3,
}

pub(crate) struct Stats {
    started: Instant,
    files_total: AtomicU64,
    bytes_total: AtomicU64,
    files_done: AtomicU64,
    bytes_done: AtomicU64,
    /// Every byte moved, retried ones included: the rate's measure.
    bytes_moved: AtomicU64,
    skipped: AtomicU64,
    /// Decisions of the file rule "neuere ersetzen ältere" (`update.rs`).
    update: [AtomicU64; 4],
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
    /// Time of all operations of the job by account (nanoseconds), booked
    /// as it is spent.
    source_ns: AtomicU64,
    target_ns: AtomicU64,
    cpu_ns: AtomicU64,
    wait_ns: AtomicU64,
    /// The part of `wait_ns` spent waiting for the memory budget.
    memory_wait_ns: AtomicU64,
    /// Ranges of large files running right now.
    segments_running: AtomicU64,
    /// Bytes read back to prove files; shared with `file_evidence`.
    checked: Arc<AtomicU64>,
    /// `checked` at the last snapshot: a change labels the file „prüft“.
    checked_seen: AtomicU64,
}

/// The job's totals at one moment; the verdict works on the difference of two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    pub accounts: OpAccounts,
    pub memory_wait_ns: u64,
    /// Every byte moved, retried ones included.
    pub moved: u64,
}

impl Totals {
    /// What happened since `earlier`.
    pub(crate) fn since(self, earlier: Totals) -> Totals {
        let minus = |now: u64, then: u64| now.saturating_sub(then);
        Totals {
            accounts: OpAccounts {
                source_ns: minus(self.accounts.source_ns, earlier.accounts.source_ns),
                target_ns: minus(self.accounts.target_ns, earlier.accounts.target_ns),
                cpu_ns: minus(self.accounts.cpu_ns, earlier.accounts.cpu_ns),
                wait_ns: minus(self.accounts.wait_ns, earlier.accounts.wait_ns),
            },
            memory_wait_ns: minus(self.memory_wait_ns, earlier.memory_wait_ns),
            moved: minus(self.moved, earlier.moved),
        }
    }
}

/// A range of a large file; counted as running while this lives.
pub(crate) struct RunningSegment<'a> {
    stats: &'a Stats,
}

impl Drop for RunningSegment<'_> {
    fn drop(&mut self) {
        self.stats.segments_running.fetch_sub(1, Ordering::AcqRel);
    }
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
            update: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            omitted: AtomicU64::new(0),
            running: AtomicU64::new(0),
            high_water: AtomicU64::new(0),
            last_progress_ms: AtomicU64::new(0),
            discovering: AtomicBool::new(true),
            next_token: AtomicU64::new(1),
            active: Mutex::new(Vec::new()),
            rate: Mutex::new(RateWindow::default()),
            source_ns: AtomicU64::new(0),
            target_ns: AtomicU64::new(0),
            cpu_ns: AtomicU64::new(0),
            wait_ns: AtomicU64::new(0),
            memory_wait_ns: AtomicU64::new(0),
            segments_running: AtomicU64::new(0),
            checked: Arc::new(AtomicU64::new(0)),
            checked_seen: AtomicU64::new(0),
        }
    }

    /// The meter of bytes read back to prove files (`file_evidence`).
    pub(crate) fn check_meter(&self) -> Arc<AtomicU64> {
        self.checked.clone()
    }

    /// Books time spent by account.
    pub(crate) fn book(&self, delta: OpAccounts) {
        self.source_ns.fetch_add(delta.source_ns, Ordering::AcqRel);
        self.target_ns.fetch_add(delta.target_ns, Ordering::AcqRel);
        self.cpu_ns.fetch_add(delta.cpu_ns, Ordering::AcqRel);
        self.wait_ns.fetch_add(delta.wait_ns, Ordering::AcqRel);
    }

    /// Books time waited for the memory budget (it counts as waiting too,
    /// through `Meter::add_wait`; this only marks the memory part).
    pub(crate) fn book_memory_wait(&self, ns: u64) {
        self.memory_wait_ns.fetch_add(ns, Ordering::AcqRel);
    }

    /// A range of a large file starts running.
    pub(crate) fn begin_segment(&self) -> RunningSegment<'_> {
        self.segments_running.fetch_add(1, Ordering::AcqRel);
        RunningSegment { stats: self }
    }

    pub(crate) fn totals(&self) -> Totals {
        Totals {
            accounts: OpAccounts {
                source_ns: self.source_ns.load(Ordering::Acquire),
                target_ns: self.target_ns.load(Ordering::Acquire),
                cpu_ns: self.cpu_ns.load(Ordering::Acquire),
                wait_ns: self.wait_ns.load(Ordering::Acquire),
            },
            memory_wait_ns: self.memory_wait_ns.load(Ordering::Acquire),
            moved: self.bytes_moved.load(Ordering::Acquire),
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

    /// One decision of the file rule "neuere ersetzen ältere".
    pub(crate) fn decided(&self, decision: Decision) {
        self.update[decision as usize].fetch_add(1, Ordering::AcqRel);
    }

    /// A replacement the target could not take: the file is kept beside its
    /// counterpart instead (counted as kept, no longer as replaced).
    pub(crate) fn replace_refused(&self) {
        let _ = self.update[Decision::Replaced as usize].fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |count| Some(count.saturating_sub(1)),
        );
        self.decided(Decision::Both);
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
            // Read-backs that prove published files move bytes too; without
            // them the rate fell to 0 while a large file was being checked.
            let checked = self.checked.load(Ordering::Acquire);
            rate.record(
                elapsed_ms,
                self.bytes_moved
                    .load(Ordering::Acquire)
                    .saturating_add(checked),
            );
            rate.rate()
        };
        // Helpers of a large file run under its name: it is listed once.
        let mut active: Vec<String> = Vec::new();
        for (_, name) in self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
        {
            if active.len() == ACTIVE_SHOWN {
                break;
            }
            if !active.contains(name) {
                active.push(name.clone());
            }
        }
        let mut progress = TransferProgress::new(
            kind,
            kind.label(),
            self.files_total.load(Ordering::Acquire),
            self.bytes_total.load(Ordering::Acquire),
        );
        let checked = self.checked.load(Ordering::Acquire);
        let checking = self.checked_seen.swap(checked, Ordering::AcqRel) != checked;
        progress.current = active
            .first()
            .map(|name| {
                if checking {
                    format!("prüft {name}")
                } else {
                    name.clone()
                }
            })
            .unwrap_or_default();
        progress.files_done = self.files_done.load(Ordering::Acquire);
        progress.bytes_done = self.bytes_done.load(Ordering::Acquire);
        progress.elapsed_ms = elapsed_ms;
        progress.errors = errors;
        progress.omitted = self.omitted.load(Ordering::Acquire);
        progress.discovering = self.discovering.load(Ordering::Acquire);
        progress.skipped = self.skipped.load(Ordering::Acquire);
        progress.replaced = self.update[Decision::Replaced as usize].load(Ordering::Acquire);
        progress.skipped_equal = self.update[Decision::Equal as usize].load(Ordering::Acquire);
        progress.kept_newer = self.update[Decision::TargetNewer as usize].load(Ordering::Acquire);
        progress.kept_both = self.update[Decision::Both as usize].load(Ordering::Acquire);
        progress.rate_bps = rate_bps;
        progress.active = active;
        progress.parallel = self
            .running
            .load(Ordering::Acquire)
            .min(u64::from(u32::MAX)) as u32;
        progress.segments = self
            .segments_running
            .load(Ordering::Acquire)
            .min(u64::from(u32::MAX)) as u32;
        progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-back bytes keep the rate alive and label the file as checked,
    /// without counting as transferred bytes.
    #[test]
    fn screen_metadata_runtime_task_read_back_shows_as_checking() {
        let stats = Stats::new();
        let _file = stats.begin("a.bin");
        assert_eq!(stats.snapshot(TransferKind::Upload, 0).current, "a.bin");
        stats.check_meter().fetch_add(1024, Ordering::AcqRel);
        let checking = stats.snapshot(TransferKind::Upload, 0);
        assert_eq!(checking.current, "prüft a.bin");
        assert_eq!(checking.bytes_done, 0);
        assert_eq!(stats.snapshot(TransferKind::Upload, 0).current, "a.bin");
    }

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
    fn speed_task_stats_list_a_split_file_once() {
        let stats = Stats::new();
        let owner = stats.begin("big.iso");
        let helper_one = stats.begin("big.iso");
        let helper_two = stats.begin("big.iso");
        let other = stats.begin("small.txt");
        let progress = stats.snapshot(TransferKind::Download, 0);
        assert_eq!(progress.active, vec!["big.iso", "small.txt"]);
        assert_eq!(progress.current, "big.iso");
        assert_eq!(progress.parallel, 4, "every running operation counts");
        drop((owner, helper_one, helper_two, other));
        assert_eq!(stats.snapshot(TransferKind::Download, 0).parallel, 0);
    }

    #[test]
    fn speed_task_stats_totals_and_running_ranges() {
        let stats = Stats::new();
        let before = stats.totals();
        stats.book(OpAccounts {
            source_ns: 3,
            target_ns: 5,
            cpu_ns: 7,
            wait_ns: 11,
        });
        stats.book_memory_wait(2);
        stats.moved(100);
        let delta = stats.totals().since(before);
        assert_eq!(delta.accounts.source_ns, 3);
        assert_eq!(delta.accounts.wait_ns, 11);
        assert_eq!((delta.memory_wait_ns, delta.moved), (2, 100));
        // a counter that did not move gives zero, never a wrapped number
        assert_eq!(before.since(stats.totals()), Totals::default());
        let first = stats.begin_segment();
        let second = stats.begin_segment();
        assert_eq!(stats.snapshot(TransferKind::Download, 0).segments, 2);
        drop(first);
        assert_eq!(stats.snapshot(TransferKind::Download, 0).segments, 1);
        drop(second);
        assert_eq!(stats.snapshot(TransferKind::Download, 0).segments, 0);
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
