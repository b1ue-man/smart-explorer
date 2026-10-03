//! What the workers of a mirror's copy pass do to its shared state: copying
//! one file under the pair's transfer permits (again after overload, see
//! `bisync::sync_overload`), and the bookkeeping of the listing workers
//! (errors, omissions, budget, queues). Permits are always taken last and
//! released before any wait on the queues or a backoff (plan rule K2).
use super::imp::record_error;
use super::sync_copy::{copy_stream_scoped, CopyError};
use super::sync_pass::{Pass, WAIT_SLICE};
use super::sync_scan::{decide, Decision, DirTask, FileTask};
use crate::bisync::sync_flows::PairSide;
use crate::bisync::sync_overload::Backoff;
use crate::transfer::engine::folders::FolderError;
use crate::transfer::engine::sleep_unless;
use crate::transfer::{classify_error, FlowPermit, OpOutcome};
use crate::vfs::VfsMeta;
use std::cell::Cell;
use std::io;
use std::sync::atomic::Ordering;

/// How one copy task ended short of an error.
enum Finished {
    Copied(u64),
    /// A confirmed destination turned out unchanged.
    Skipped,
    /// A confirmed destination turned out to be a link.
    Omitted,
    /// A confirmed destination cannot take the file (reported, final).
    Refused(String),
}

/// Files found but not yet copied. Discovery runs at most this far ahead of
/// the copies: 4096 tasks (a few hundred bytes each, about 2 MiB) keep even
/// the flows' highest limit of 256 operations busy through many listing round
/// trips, while a tree of a million files never sits in memory as a plan.
const FILE_QUEUE_LIMIT: usize = 4096;

impl Pass<'_> {
    /// Copies one queued file; its worker was counted as waiting until the
    /// permits are granted. Overload before publication gives the permits
    /// back as overload, waits the peer's delay without them and copies
    /// again (`Backoff`); every other outcome is final.
    pub(super) fn copy_file(&self, task: FileTask) {
        let mut backoff = Backoff::new(&self.progress);
        let mut counted = true;
        loop {
            if !counted {
                self.lock().copiers.waiting += 1;
            }
            let permits = self.flows.transfer(self.cancel);
            {
                let mut state = self.lock();
                state.copiers.waiting -= 1;
                if permits.is_some() {
                    state.current = task.destination.clone();
                }
            }
            counted = false;
            self.changed.notify_all();
            // `None`: canceled before the copy started; nothing to report.
            let Some(permits) = permits else {
                return;
            };
            if self.canceled() {
                permits.finish(OpOutcome::Done);
                return;
            }
            let streamed = Cell::new(0u64);
            let on_block = |bytes: u64| {
                permits.progress(bytes);
                streamed.set(streamed.get().saturating_add(bytes));
                self.streaming.fetch_add(bytes, Ordering::Relaxed);
            };
            let result = self.confirm_and_copy(&task, &on_block);
            permits.finish(match &result {
                Ok(Finished::Refused(_)) => OpOutcome::Failed,
                Ok(_) => OpOutcome::Done,
                Err(failure) => classify_error(&failure.error),
            });
            let delay = match &result {
                Err(failure) if !failure.publishing => backoff.pause(&failure.error),
                _ => None,
            };
            let Some(delay) = delay else {
                self.copied(&task, streamed.get(), result);
                return;
            };
            self.streaming.fetch_sub(streamed.get(), Ordering::Relaxed);
            if !sleep_unless(self.cancel, delay) {
                return;
            }
        }
    }

    /// One attempt of a task under its permits: a listed destination that
    /// needs confirming is looked up first (overload there is retried like
    /// the copy), then the file is copied if the serial rule says so.
    fn confirm_and_copy(
        &self,
        task: &FileTask,
        on_block: &dyn Fn(u64),
    ) -> Result<Finished, CopyError> {
        if self.canceled() {
            return Err(CopyError {
                error: io::Error::new(io::ErrorKind::Interrupted, "mirror stopped"),
                publishing: false,
            });
        }
        crate::bisync::apply_boundary::guard(self.src, self.src_root, &task.rel, false)
            .and_then(|()| {
                crate::bisync::apply_boundary::guard(
                    self.dst,
                    self.dst_root,
                    &task.target_rel,
                    false,
                )
            })
            .map_err(|error| CopyError {
                error,
                publishing: false,
            })?;
        let confirmed: Option<VfsMeta>;
        let expected = if task.confirm {
            let found = match self.dst.stat(&task.destination) {
                Ok(found) => Ok(Some(found)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) if classify_error(&error) == OpOutcome::Overload => {
                    return Err(CopyError {
                        error,
                        publishing: false,
                    })
                }
                Err(error) => Err(error),
            };
            confirmed = match decide(&task.meta, found) {
                Decision::Copy(expected) => expected,
                Decision::Skip => return Ok(Finished::Skipped),
                Decision::Omit => return Ok(Finished::Omitted),
                Decision::Fail(message) => return Ok(Finished::Refused(message)),
            };
            confirmed.as_ref()
        } else {
            task.expected.as_ref()
        };
        copy_stream_scoped(
            self.src,
            self.src_root,
            &task.source,
            &task.rel,
            &task.meta,
            self.dst,
            self.dst_root,
            &task.destination,
            &task.target_rel,
            expected,
            self.versions,
            self.cancel,
            on_block,
        )
        .map(Finished::Copied)
    }

    /// Books one finished task (the bytes it streamed leave `streaming`).
    fn copied(&self, task: &FileTask, streamed: u64, result: Result<Finished, CopyError>) {
        if result.is_ok() {
            self.progress.touch();
        }
        let mut guard = self.lock();
        let state = &mut *guard;
        self.streaming.fetch_sub(streamed, Ordering::Relaxed);
        match result {
            Ok(Finished::Copied(bytes)) => {
                state.report.stats.copied += 1;
                state.report.stats.bytes += bytes;
            }
            Ok(Finished::Skipped) => state.report.stats.skipped += 1,
            Ok(Finished::Omitted) => state.report.omissions.record(&task.rel, true),
            Ok(Finished::Refused(message)) => record_error(
                &mut state.report.stats,
                &mut state.report.errors,
                task.destination.as_str(),
                message,
            ),
            Err(failure) => {
                if let Some(kind) = crate::bisync::apply_boundary::omitted(&failure.error) {
                    state
                        .report
                        .omissions
                        .record_kind(&task.rel, kind, kind.reported_by_default());
                } else if crate::bisync::apply_boundary::deferred(&failure.error) {
                    state.report.omissions.record_kind(
                        &task.rel,
                        crate::bisync::OmissionKind::Unreadable,
                        true,
                    );
                } else if failure.error.kind() != io::ErrorKind::Interrupted
                    || !self.cancel.load(Ordering::Acquire)
                {
                    if terminal(&failure.error) {
                        self.stopped.store(true, Ordering::Release);
                    }
                    record_error(
                        &mut state.report.stats,
                        &mut state.report.errors,
                        task.destination.as_str(),
                        failure.error.to_string(),
                    );
                }
            }
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

    pub(super) fn io_error(&self, path: impl Into<String>, error: io::Error) {
        if terminal(&error) {
            self.stopped.store(true, Ordering::Release);
            self.changed.notify_all();
        }
        if error.kind() == io::ErrorKind::Interrupted && self.canceled() {
            return;
        }
        self.error(path, error.to_string());
    }
    pub(super) fn omit_kind(&self, rel: &str, kind: crate::bisync::OmissionKind) {
        self.lock()
            .report
            .omissions
            .record_kind(rel, kind, kind.reported_by_default());
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

    /// Queues a folder to list, unless discovery stopped (budget) or the pass
    /// ended meanwhile: then the folder is dropped, never left waiting for a
    /// listing worker that will not come.
    pub(super) fn queue_dir(&self, task: DirTask) {
        let mut state = self.lock();
        if state.scan_stopped || state.finished {
            return;
        }
        state.dirs.push_back(task);
        drop(state);
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

fn terminal(error: &io::Error) -> bool {
    crate::vfs::is_target_refusal(error)
        || matches!(
            error.kind(),
            io::ErrorKind::ConnectionRefused
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::NotConnected
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::HostUnreachable
                | io::ErrorKind::NetworkUnreachable
                | io::ErrorKind::NetworkDown
                | io::ErrorKind::TimedOut
        )
}
