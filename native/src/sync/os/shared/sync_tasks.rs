//! What the workers of a mirror's copy pass do to its shared state: copying
//! one file under the pair's transfer permits, and the bookkeeping of the
//! listing workers (errors, omissions, budget, queues). Permits are always
//! taken last and released before any wait on the queues (plan rule K2).
use super::imp::record_error;
use super::sync_copy::copy_stream;
use super::sync_pass::{Pass, WAIT_SLICE};
use super::sync_scan::{DirTask, FileTask};
use crate::bisync::sync_flows::{outcome, PairSide};
use crate::transfer::engine::folders::FolderError;
use crate::transfer::FlowPermit;
use std::cell::Cell;
use std::sync::atomic::Ordering;

/// Files found but not yet copied. Discovery runs at most this far ahead of
/// the copies: 4096 tasks (a few hundred bytes each, about 2 MiB) keep even
/// the flows' highest limit of 256 operations busy through many listing round
/// trips, while a tree of a million files never sits in memory as a plan.
const FILE_QUEUE_LIMIT: usize = 4096;

impl Pass<'_> {
    /// Copies one queued file; its worker was counted as waiting until the
    /// permits are granted.
    pub(super) fn copy_file(&self, task: FileTask) {
        let permits = self.flows.transfer(self.cancel);
        {
            let mut state = self.lock();
            state.copiers.waiting -= 1;
            if permits.is_some() {
                state.current = task.destination.clone();
            }
        }
        self.changed.notify_all();
        // `None`: canceled before the copy started; nothing to report.
        let Some(permits) = permits else {
            return;
        };
        let streamed = Cell::new(0u64);
        let on_block = |bytes: u64| {
            permits.progress(bytes);
            streamed.set(streamed.get().saturating_add(bytes));
            self.streaming.fetch_add(bytes, Ordering::Relaxed);
        };
        let result = copy_stream(
            self.src,
            &task.source,
            &task.meta,
            self.dst,
            &task.destination,
            task.expected.as_ref(),
            self.cancel,
            &on_block,
        );
        permits.finish(outcome(&result));
        let mut guard = self.lock();
        let state = &mut *guard;
        self.streaming.fetch_sub(streamed.get(), Ordering::Relaxed);
        match result {
            Ok(bytes) => {
                state.report.stats.copied += 1;
                state.report.stats.bytes += bytes;
            }
            Err(error) => record_error(
                &mut state.report.stats,
                &mut state.report.errors,
                task.source,
                error.to_string(),
            ),
        }
    }

    /// Runs `blocking` (a permit, a folder creation) counted as waiting, so
    /// the coordinator starts no further listing worker meanwhile.
    fn waiting<T>(&self, blocking: impl FnOnce() -> T) -> T {
        self.lock().scanners.waiting += 1;
        let result = blocking();
        self.lock().scanners.waiting -= 1;
        self.changed.notify_all();
        result
    }

    /// A listing permit on `side` (`Some(None)`: that side is not
    /// regulated); `None` once canceled.
    pub(super) fn listing_permit(&self, side: PairSide) -> Option<Option<FlowPermit>> {
        self.waiting(|| self.flows.listing(side, self.cancel))
    }

    /// Creates the destination folder `rel` exactly once (parents first,
    /// under a metadata permit the register takes itself).
    pub(super) fn ensure_folder(&self, rel: &str) -> Result<bool, FolderError> {
        self.waiting(|| self.folders.ensure(rel, self.cancel))
    }

    pub(super) fn error(&self, path: impl Into<String>, message: impl Into<String>) {
        let mut guard = self.lock();
        let state = &mut *guard;
        record_error(
            &mut state.report.stats,
            &mut state.report.errors,
            path,
            message,
        );
    }

    /// A protected omission (link, junction, app trash): reported, never
    /// copied into, and its destination counterpart is kept.
    pub(super) fn omit(&self, rel: &str) {
        self.lock().report.omissions.record(rel, true);
    }

    pub(super) fn skipped(&self) {
        self.lock().report.stats.skipped += 1;
    }

    /// A dry run counts the file it would copy.
    pub(super) fn would_copy(&self, destination: &str) {
        let mut state = self.lock();
        state.report.stats.copied += 1;
        state.current = destination.to_string();
    }

    /// Counts `path` against the tree budget; over budget, discovery stops
    /// for the whole pass (files already found are still copied).
    pub(super) fn within_budget(&self, path: &str, depth: usize) -> bool {
        let mut guard = self.lock();
        let state = &mut *guard;
        let Err(error) = state.budget.record(path, depth) else {
            return true;
        };
        record_error(
            &mut state.report.stats,
            &mut state.report.errors,
            path,
            error,
        );
        state.scan_stopped = true;
        state.dirs.clear();
        drop(guard);
        self.changed.notify_all();
        false
    }

    pub(super) fn queue_dir(&self, task: DirTask) {
        self.lock().dirs.push_back(task);
        self.changed.notify_all();
    }

    /// Queues a file for the copy workers, waiting (without any permit) while
    /// the queue is full; false when the pass ended meanwhile.
    pub(super) fn queue_file(&self, task: FileTask) -> bool {
        let mut state = self.lock();
        let mut counted = false;
        while state.files.len() >= FILE_QUEUE_LIMIT && !state.finished && !self.canceled() {
            if !counted {
                state.scanners.waiting += 1;
                counted = true;
            }
            state = self.wait(state, WAIT_SLICE);
        }
        if counted {
            state.scanners.waiting -= 1;
        }
        let open = !state.finished && !self.canceled();
        if open {
            state.files.push_back(task);
        }
        drop(state);
        self.changed.notify_all();
        open
    }
}
