//! Runs engine jobs in tests: to their terminal message on this thread, with
//! the job shape most tests need.
use super::run_job;
use super::test_backend::fwd;
use crate::transfer::{
    Endpoint, JobItems, Layout, ResolvedRoot, TransferIssue, TransferJob, TransferMsg,
    TransferProgress,
};
use crate::types::{Conflict, CopyMode};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// The terminal message of a job.
pub(super) struct Finished {
    pub progress: TransferProgress,
    pub errors: Vec<String>,
    pub canceled: bool,
    pub issues: Vec<TransferIssue>,
    pub roots: Vec<ResolvedRoot>,
    pub updates: usize,
}

impl Finished {
    pub(super) fn has_issue(&self, text: &str) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.message.contains(text) || issue.path.contains(text))
    }
}

pub(super) fn collect(rx: &crossbeam_channel::Receiver<TransferMsg>) -> Finished {
    let mut updates = 0;
    let mut finished = None;
    for message in rx.try_iter() {
        match message {
            TransferMsg::Progress(_) => updates += 1,
            TransferMsg::Done {
                progress,
                errors,
                canceled,
                issues,
                roots,
            } => {
                assert!(finished.is_none(), "exactly one terminal message");
                finished = Some((progress, errors, canceled, issues, roots));
            }
        }
    }
    let (progress, errors, canceled, issues, roots) =
        finished.expect("the job ended with its terminal message");
    Finished {
        progress,
        errors,
        canceled,
        issues,
        roots,
        updates,
    }
}

/// Runs `job` to its end on this thread.
pub(super) fn run(job: TransferJob) -> Finished {
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);
    run_job(job, &tx, &cancel);
    collect(&rx)
}

/// A job copying whole entries into `target_dir` ("keep both").
pub(super) fn job(
    source: Endpoint,
    target: Endpoint,
    target_dir: &Path,
    paths: &[&Path],
) -> TransferJob {
    TransferJob {
        source,
        target,
        target_dir: fwd(target_dir),
        items: JobItems::Roots {
            paths: paths.iter().map(|path| fwd(path)).collect(),
            base: None,
        },
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        mode: CopyMode::Copy,
        source_label: "Quelle".to_string(),
        target_label: "Ziel".to_string(),
        resume: None,
    }
}
