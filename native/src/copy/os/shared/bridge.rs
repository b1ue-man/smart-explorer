//! Local copies and moves of the copy dialog, the clipboard and drag & drop
//! run as engine jobs (local → local); their progress and result reach the
//! callers in the older `CopyMsg` form (errors as path and text).
use super::{CopyHandle, CopyMsg};
use crate::transfer::{run_view, JobItems, JobView, Layout, Side, TransferMsg, TransferProgress};
use crate::types::{Conflict, CopyMode, CopyProgress, FilterDef};
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One local job as the copy entry points describe it.
pub(super) struct LocalJob {
    pub items: JobItems,
    pub layout: Layout,
    pub filter: Option<(FilterDef, String)>,
    pub target_dir: String,
    pub conflict: Conflict,
    pub mode: CopyMode,
}

/// Starts `prepare` and then the job on a worker thread; the handle cancels.
pub(super) fn spawn(
    tx: Sender<CopyMsg>,
    prepare: impl FnOnce() -> Result<LocalJob, (String, String)> + Send + 'static,
) -> CopyHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let failure_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || match prepare() {
            Ok(job) => run(&job, &tx, &worker_cancel),
            Err((path, detail)) => send_failure(&tx, path, detail),
        });
    if let Err(error) = spawned {
        send_failure(&failure_tx, "Kopieren".to_string(), error.to_string());
    }
    CopyHandle { cancel }
}

fn run(job: &LocalJob, tx: &Sender<CopyMsg>, cancel: &AtomicBool) {
    let source_label = match &job.items {
        JobItems::Roots {
            base: Some(base), ..
        } => base.clone(),
        JobItems::Roots { paths, .. } => paths
            .first()
            .map(|path| crate::transfer::parent_path(path))
            .unwrap_or_default(),
        JobItems::Pairs(pairs) => pairs
            .first()
            .map(|pair| crate::transfer::parent_path(&pair.source))
            .unwrap_or_default(),
    };
    let view = JobView {
        source: Side::Local,
        target: Side::Local,
        target_dir: &job.target_dir,
        items: &job.items,
        layout: job.layout,
        filter: job.filter.as_ref(),
        conflict: job.conflict,
        mode: job.mode,
        source_label: &source_label,
        target_label: &job.target_dir,
        resume: None,
    };
    run_view(
        view,
        &|message| {
            let _ = tx.send(translate(message, cancel));
        },
        cancel,
    );
}

fn copy_progress(progress: &TransferProgress, canceled: bool, done: bool) -> CopyProgress {
    // The copy dialog counts every file it is finished with, also skipped
    // and failed ones.
    let processed = progress
        .files_done
        .saturating_add(progress.skipped)
        .saturating_add(progress.errors)
        .min(progress.files_total.max(progress.files_done));
    CopyProgress {
        files_done: processed,
        files_total: progress.files_total,
        bytes_done: progress.bytes_done,
        bytes_total: progress.bytes_total,
        elapsed_ms: progress.elapsed_ms,
        errors: progress.errors,
        canceled,
        done,
    }
}

fn translate(message: TransferMsg, cancel: &AtomicBool) -> CopyMsg {
    match message {
        TransferMsg::Progress(progress) => {
            CopyMsg::Progress(copy_progress(&progress, false, false))
        }
        TransferMsg::Done {
            progress,
            canceled,
            issues,
            ..
        } => CopyMsg::Done {
            progress: copy_progress(&progress, canceled || cancel.load(Ordering::Acquire), true),
            errors: issues
                .into_iter()
                .map(|issue| (issue.path, issue.message))
                .collect(),
        },
    }
}

pub(super) fn send_failure(tx: &Sender<CopyMsg>, path: String, detail: String) {
    let _ = tx.send(CopyMsg::Done {
        progress: CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            errors: 1,
            canceled: false,
            done: true,
        },
        errors: vec![(path, detail)],
    });
}
