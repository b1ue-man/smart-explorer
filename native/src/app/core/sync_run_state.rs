//! Desktop-only bookkeeping; engine StateKeys remain authoritative.
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use super::*;
use super::support_paths::BisyncCtx;

#[derive(Default)]
pub(in crate::app) struct RunMailbox {
    pub(in crate::app) context: Option<BisyncCtx>,
    pub(in crate::app) persistence_errors: Vec<String>,
    pub(in crate::app) pending: Vec<crate::bisync::Conflict>,
    pub(in crate::app) preparation_error: Option<crate::syncjobs::JobError>,
}
pub(in crate::app) struct DesktopRun {
    pub(in crate::app) mailbox: Arc<Mutex<RunMailbox>>,
}

pub(in crate::app) struct JobConfirmation {
    pub kind:crate::syncjobs::BlockKind, pub source:String, pub target:String,
}

pub(in crate::app) struct SyncWorker {
    worker: std::thread::JoinHandle<()>, cancel:Arc<AtomicBool>,
}
impl App {
    pub(in crate::app) fn track_desktop_sync_worker(&mut self, worker:std::thread::JoinHandle<()>, cancel:Arc<AtomicBool>) {
        self.sync_workers.push(SyncWorker { worker,cancel });
    }
    pub(in crate::app) fn desktop_sync_active(&self) -> bool {
        self.sync_workers.iter().any(|task| !task.worker.is_finished())
            || self.conflict_resolution.as_ref().is_some_and(|task| task.worker_active())
    }
    pub(in crate::app) fn desktop_job_busy(&self, id:&str) -> bool {
        (self.bisync_running && self.running_job.as_deref() == Some(id))
            || (self.apply_one_rx.is_some() && self.preview_job_id.as_deref() == Some(id))
            || (self.bisync_ctx.as_ref().is_some_and(|context| context.job_id.as_deref() == Some(id))
                && (self.conflict_resolution.is_some() || self.merge.is_some()
                    || self.merge_load_rx.is_some() || self.merge_apply_rx.is_some()))
            || self.sync_versions.as_ref().is_some_and(|view| view.protects_job(id))
    }
    pub(in crate::app) fn cancel_desktop_sync(&mut self) {
        for task in &self.sync_workers { task.cancel.store(true, Ordering::Release); }
        for cancel in [&self.sync_cancel,&self.bisync_cancel,&self.preview_cancel].into_iter().flatten() {
            cancel.store(true, Ordering::Release);
        }
        self.cancel_conflict_resolution();
    }
    /// Join only finished workers. Close/update gates keep the window alive meanwhile.
    pub(in crate::app) fn drain_desktop_sync_workers(&mut self) -> usize {
        let mut index = 0;
        while index < self.sync_workers.len() {
            if self.sync_workers[index].worker.is_finished() {
                let task = self.sync_workers.swap_remove(index);
                if task.worker.join().is_err() {
                    self.error_msg = Some("Sync-Worker endete ohne verlässliches Ergebnis; gespeicherten Zustand prüfen.".into());
                }
            } else { index += 1; }
        }
        self.sync_workers.len() + usize::from(self.conflict_resolution.as_ref().is_some_and(|task| task.worker_active()))
    }
}
