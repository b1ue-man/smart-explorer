use super::types::{TransferIssue, TransferMsg, TransferProgress};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const CANCELED_ERROR: &str = "Übertragung abgebrochen";

pub(super) fn requested(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Acquire)
}

pub(super) fn check(cancel: &AtomicBool) -> Result<(), String> {
    if requested(cancel) {
        Err(CANCELED_ERROR.to_string())
    } else {
        Ok(())
    }
}

pub(super) fn check_optional(cancel: Option<&AtomicBool>) -> Result<(), String> {
    match cancel {
        Some(cancel) => check(cancel),
        None => Ok(()),
    }
}

pub(super) fn send_done(
    tx: &crossbeam_channel::Sender<TransferMsg>,
    mut progress: TransferProgress,
    errors: Vec<String>,
    cancel: &AtomicBool,
) {
    progress.done = true;
    let issues = errors
        .iter()
        .map(|message| TransferIssue {
            path: String::new(),
            message: message.clone(),
        })
        .collect();
    let _ = tx.send(TransferMsg::Done {
        progress,
        errors,
        canceled: requested(cancel),
        issues,
        roots: Vec::new(),
    });
}
