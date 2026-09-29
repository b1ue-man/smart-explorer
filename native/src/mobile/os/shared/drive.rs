//! Runs the transfer engine and the local copy worker inside a task:
//! forwards their progress (growing totals while folders are still being
//! walked), notes, errors and cancellation, and turns the terminal message
//! into the task result.
use super::error::ApiError;
use super::runtime::TaskCtx;
use crate::copy::{CopyHandle, CopyMsg};
use crate::transfer::{
    ActiveTransfer, Endpoint, JobItems, Layout, TransferJob, TransferMsg, TransferProgress,
    TransferRequest,
};
use crate::types::{Conflict, CopyMode};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(200);

/// The message shown when the active app trash was left out.
pub(crate) const TRASH_OMITTED: &str = "Papierkorb-Ordner ausgelassen";

/// Summary of a finished transfer; errors make the task fail with this
/// summary as its result.
pub(crate) struct Outcome {
    pub files: u64,
    pub bytes: u64,
    pub errors: u64,
    pub omitted: u64,
    pub canceled: bool,
}

impl Outcome {
    pub(crate) fn into_result(self, ctx: &TaskCtx) -> Result<Value, ApiError> {
        let summary = json!({
            "files": self.files,
            "bytes": self.bytes,
            "errors": self.errors,
            "omitted": self.omitted,
        });
        if self.omitted > 0 {
            ctx.message(TRASH_OMITTED);
        }
        if self.canceled || ctx.cancelled() {
            ctx.set_failure_result(summary);
            return Err(ApiError::canceled());
        }
        if self.errors > 0 {
            ctx.set_failure_result(summary);
            return Err(ApiError::internal(format!(
                "{} Fehler – Details in der Fehlerliste",
                self.errors
            )));
        }
        Ok(summary)
    }
}

/// Whole entries from `source` into `target_dir` of `target`, each under its
/// own name; occupied names become "Name (2)", nothing is replaced.
pub(crate) fn transfer_job(
    source: Endpoint,
    paths: Vec<String>,
    target: Endpoint,
    target_dir: String,
    source_label: String,
    target_label: String,
) -> TransferJob {
    TransferJob {
        source,
        target,
        target_dir,
        items: JobItems::Roots { paths, base: None },
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        mode: CopyMode::Copy,
        source_label,
        target_label,
        resume: None,
    }
}

/// Runs `job` through the transfer engine and waits for its end.
pub(crate) fn run_job(ctx: &TaskCtx, job: TransferJob) -> Result<Outcome, ApiError> {
    job.validate().map_err(ApiError::invalid)?;
    run_transfer(ctx, TransferRequest::Job(Box::new(job)))
}

/// Launches `request` like the desktop lane and waits for its end.
pub(crate) fn run_transfer(ctx: &TaskCtx, request: TransferRequest) -> Result<Outcome, ApiError> {
    let active = crate::transfer::launch_transfer(request).map_err(ApiError::internal)?;
    Ok(drain_transfer(ctx, active))
}

/// What the task list shows next to the numbers: the search while folders
/// are still being walked, otherwise the engine's note (a connection that is
/// busy, a provider's limit).
pub(crate) fn task_message(progress: &TransferProgress) -> Option<String> {
    if progress.discovering {
        return Some(format!("Suche Dateien… {} gefunden", progress.files_total));
    }
    progress
        .note
        .as_ref()
        .filter(|note| !note.trim().is_empty())
        .cloned()
}

fn report_transfer(ctx: &TaskCtx, progress: &TransferProgress, shown: &mut Option<String>) {
    ctx.progress(
        progress.bytes_done,
        progress.bytes_total,
        progress.files_done,
        progress.files_total,
    );
    let message = task_message(progress);
    if message != *shown {
        ctx.message(message.as_deref().unwrap_or_default());
        *shown = message;
    }
}

pub(crate) fn drain_transfer(ctx: &TaskCtx, mut active: ActiveTransfer) -> Outcome {
    let mut last = active.progress.clone();
    let mut shown = None;
    loop {
        if ctx.cancelled() && !active.canceling() {
            active.request_cancel();
        }
        match active.rx.recv_timeout(POLL) {
            Ok(TransferMsg::Progress(progress)) => {
                report_transfer(ctx, &progress, &mut shown);
                last = progress;
            }
            Ok(TransferMsg::Done {
                progress,
                errors,
                canceled,
                issues,
                ..
            }) => {
                if let Some(worker) = active.worker.take() {
                    let _ = worker.join();
                }
                report_transfer(ctx, &progress, &mut shown);
                // The issues carry their paths; older workers report lines.
                if issues.is_empty() {
                    for error in &errors {
                        ctx.error("", error);
                    }
                } else {
                    for issue in &issues {
                        ctx.error(&issue.path, &issue.message);
                    }
                }
                let listed = issues.len().max(errors.len()) as u64;
                return Outcome {
                    files: progress.files_done,
                    bytes: progress.bytes_done,
                    errors: progress.errors.max(listed),
                    omitted: progress.omitted,
                    canceled,
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(worker) = active.worker.take() {
                    let _ = worker.join();
                }
                ctx.error("", "Übertragung endete ohne Abschlussmeldung");
                return Outcome {
                    files: last.files_done,
                    bytes: last.bytes_done,
                    errors: last.errors.saturating_add(1),
                    omitted: last.omitted,
                    canceled: active.canceling(),
                };
            }
        }
    }
}

/// Waits for a local copy/move started with `crate::copy`.
pub(crate) fn drain_copy(ctx: &TaskCtx, handle: CopyHandle, rx: Receiver<CopyMsg>) -> Outcome {
    let mut files = 0;
    let mut bytes = 0;
    loop {
        if ctx.cancelled() {
            handle.cancel.store(true, Ordering::Relaxed);
        }
        match rx.recv_timeout(POLL) {
            Ok(CopyMsg::Progress(progress)) => {
                files = progress.files_done;
                bytes = progress.bytes_done;
                ctx.progress(
                    progress.bytes_done,
                    progress.bytes_total,
                    progress.files_done,
                    progress.files_total,
                );
            }
            Ok(CopyMsg::Done { progress, errors }) => {
                ctx.progress(
                    progress.bytes_done,
                    progress.bytes_total,
                    progress.files_done,
                    progress.files_total,
                );
                for (path, message) in &errors {
                    ctx.error(path, message);
                }
                return Outcome {
                    files: progress.files_done,
                    bytes: progress.bytes_done,
                    errors: progress.errors.max(errors.len() as u64),
                    omitted: 0,
                    canceled: progress.canceled,
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                ctx.error("", "Kopieren endete ohne Abschlussmeldung");
                return Outcome {
                    files,
                    bytes,
                    errors: 1,
                    omitted: 0,
                    canceled: handle.cancel.load(Ordering::Relaxed),
                };
            }
        }
    }
}
