use super::{DeleteMsg, DeletePhase, DeleteProgress};
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const PROGRESS_ENTRY_INTERVAL: u64 = 1_024;

pub(super) struct DeleteReporter {
    tx: Sender<DeleteMsg>,
    cancel: Arc<AtomicBool>,
    progress: DeleteProgress,
    target_base_planned: u64,
    target_base_deleted: u64,
    last_emit: Instant,
    last_planned: u64,
    last_deleted: u64,
}

impl DeleteReporter {
    pub(super) fn new(
        tx: Sender<DeleteMsg>,
        cancel: Arc<AtomicBool>,
        progress: DeleteProgress,
    ) -> Self {
        Self {
            tx,
            cancel,
            progress,
            target_base_planned: 0,
            target_base_deleted: 0,
            last_emit: Instant::now(),
            last_planned: 0,
            last_deleted: 0,
        }
    }

    pub(super) fn begin_target(&mut self, path: &str, phase: DeletePhase) -> bool {
        self.target_base_planned = self.progress.entries_planned;
        self.target_base_deleted = self.progress.entries_deleted;
        self.progress.phase = phase;
        self.progress.current_path = path.to_string();
        // The first target boundary is also the worker's first opportunity to
        // discover that the UI receiver was dropped. Do not let the progress
        // throttle report success without probing that channel.
        self.emit(true)
    }

    pub(super) fn begin_opaque_apply(&mut self, path: &str) -> bool {
        if !self.begin_target(path, DeletePhase::Applying) {
            return false;
        }
        self.progress.entries_planned = self.progress.entries_planned.saturating_add(1);
        self.emit(false)
    }

    pub(super) fn recursive_progress(&mut self, progress: crate::vfs::RecursiveDeleteProgress) {
        self.progress.phase = match progress.phase {
            crate::vfs::RecursiveDeletePhase::Planning => DeletePhase::Planning,
            crate::vfs::RecursiveDeletePhase::Applying => DeletePhase::Applying,
        };
        self.progress.entries_planned = self.target_base_planned.saturating_add(progress.planned);
        self.progress.entries_deleted = self.target_base_deleted.saturating_add(progress.removed);
        self.progress.current_path = progress.current;
        self.emit(false);
    }

    pub(super) fn finish_target(&mut self, succeeded: bool, opaque_deleted: bool) {
        self.progress.targets_processed = self.progress.targets_processed.saturating_add(1);
        if succeeded {
            self.progress.targets_succeeded = self.progress.targets_succeeded.saturating_add(1);
        }
        if opaque_deleted {
            self.progress.entries_deleted = self.progress.entries_deleted.saturating_add(1);
        }
        self.emit(false);
    }

    pub(super) fn finish(mut self, outcome: super::DeleteOutcome) {
        self.progress.entries_planned = outcome.entries_planned;
        self.progress.entries_deleted = outcome.entries_deleted;
        self.progress.targets_processed = outcome.processed;
        self.progress.targets_succeeded = outcome.succeeded;
        self.emit(true);
        if self.tx.send(DeleteMsg::Finished(outcome)).is_err() {
            self.cancel.store(true, Ordering::Release);
        }
    }

    fn emit(&mut self, force: bool) -> bool {
        if self.cancel.load(Ordering::Acquire) {
            return false;
        }
        let planned_delta = self
            .progress
            .entries_planned
            .saturating_sub(self.last_planned);
        let deleted_delta = self
            .progress
            .entries_deleted
            .saturating_sub(self.last_deleted);
        if !force
            && self.last_emit.elapsed() < PROGRESS_INTERVAL
            && planned_delta < PROGRESS_ENTRY_INTERVAL
            && deleted_delta < PROGRESS_ENTRY_INTERVAL
        {
            return true;
        }
        if self
            .tx
            .send(DeleteMsg::Progress(self.progress.clone()))
            .is_err()
        {
            self.cancel.store(true, Ordering::Release);
            return false;
        }
        self.last_emit = Instant::now();
        self.last_planned = self.progress.entries_planned;
        self.last_deleted = self.progress.entries_deleted;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DeleteKind, DeleteOrigin};
    use super::*;

    #[test]
    fn disconnected_progress_receiver_requests_cancel() {
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = DeleteProgress::new(DeleteKind::Permanent, DeleteOrigin::Explorer, 1);
        let mut reporter = DeleteReporter::new(tx, cancel.clone(), progress);
        assert!(!reporter.begin_target("/root", DeletePhase::Planning));
        assert!(cancel.load(Ordering::Acquire));
    }
}
