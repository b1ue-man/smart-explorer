//! Runs the desktop transfer and copy workers inside a task: forwards their
//! progress, errors and cancellation, and turns the terminal message into the
//! task result.
use super::error::ApiError;
use super::runtime::TaskCtx;
use crate::copy::{CopyHandle, CopyMsg};
use crate::transfer::{ActiveTransfer, TransferMsg, TransferProgress, TransferRequest};
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

/// Launches `request` like the desktop lane and waits for its end.
pub(crate) fn run_transfer(ctx: &TaskCtx, request: TransferRequest) -> Result<Outcome, ApiError> {
    let active = crate::transfer::launch_transfer(request).map_err(ApiError::internal)?;
    Ok(drain_transfer(ctx, active))
}

fn report_transfer(ctx: &TaskCtx, progress: &TransferProgress) {
    ctx.progress(
        progress.bytes_done,
        progress.bytes_total,
        progress.files_done,
        progress.files_total,
    );
}

pub(crate) fn drain_transfer(ctx: &TaskCtx, mut active: ActiveTransfer) -> Outcome {
    let mut last = active.progress.clone();
    loop {
        if ctx.cancelled() && !active.canceling() {
            active.request_cancel();
        }
        match active.rx.recv_timeout(POLL) {
            Ok(TransferMsg::Progress(progress)) => {
                report_transfer(ctx, &progress);
                last = progress;
            }
            Ok(TransferMsg::Done {
                progress,
                errors,
                canceled,
            }) => {
                if let Some(worker) = active.worker.take() {
                    let _ = worker.join();
                }
                report_transfer(ctx, &progress);
                for error in &errors {
                    ctx.error("", error);
                }
                return Outcome {
                    files: progress.files_done,
                    bytes: progress.bytes_done,
                    errors: progress.errors.max(errors.len() as u64),
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
