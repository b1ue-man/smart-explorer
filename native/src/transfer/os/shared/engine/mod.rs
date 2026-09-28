//! The streaming transfer engine: runs one `TransferJob` from the first
//! listing to the last published file. Block A of the transfer-engine plan
//! fills this module; until then a job ends with a clear message instead of
//! doing anything.
use super::cancel::send_done;
use super::job::TransferJob;
use super::types::{TransferMsg, TransferProgress};
use std::sync::atomic::AtomicBool;

pub(crate) fn run_job(
    job: TransferJob,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    let mut progress = TransferProgress::new(job.kind(), job.kind().label(), 0, 0);
    progress.source = job.source_label.clone();
    progress.target = job.target_label.clone();
    let error = match job.validate() {
        Err(error) => error,
        Ok(()) => "Diese Übertragungsart ist in dieser Version noch nicht verfügbar".to_string(),
    };
    send_done(tx, progress, vec![error], cancel);
}
