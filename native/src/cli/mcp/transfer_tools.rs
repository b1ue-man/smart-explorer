//! `copy` and `move` as jobs over the `se cp`/`se mv` pipeline: the same
//! destination rules and guards, plus cancellation and progress.
use super::args::Args;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use crate::cli::tree_observer::TreeObserver;
use serde_json::{json, Value};
use std::sync::Arc;

struct TransferControl(Arc<TreeObserver>);

impl JobControl for TransferControl {
    fn cancel(&self) {
        self.0.cancel();
    }

    fn progress(&self) -> Value {
        progress_value(&self.0)
    }
}

pub(super) fn start(session: &Session, kind: JobKind, args: &Args<'_>) -> Result<Arc<Job>, String> {
    let source_spec = args.string("source")?;
    let destination_spec = args.string("destination")?;
    let recursive = args.flag("recursive")?;
    let overwrite = args.flag("overwrite")?;
    // Resolve both ends before a job exists, so a typo fails immediately.
    let source = session.targets.resolve(&source_spec)?;
    let destination = session.targets.resolve(&destination_spec)?;
    let job = session.jobs.create(
        kind,
        format!("{} {source_spec} -> {destination_spec}", kind.label()),
        &[],
    )?;
    let observer = Arc::new(TreeObserver::default());
    job.set_control(Arc::new(TransferControl(observer.clone())));
    spawn_worker(job.clone(), move |_| {
        let outcome = match kind {
            JobKind::Move => crate::cli::ops::move_observed(
                &source,
                &destination,
                recursive,
                overwrite,
                &observer,
            ),
            _ => crate::cli::ops::copy_observed(
                &source,
                &destination,
                recursive,
                overwrite,
                &observer,
            ),
        };
        let progress = progress_value(&observer);
        match outcome {
            Ok(outcome) => JobEnd {
                status: JobStatus::Completed,
                result: json!({
                    "destination": outcome.destination,
                    "omitted": outcome
                        .omitted
                        .iter()
                        .map(|omission| json!({
                            "path": omission.path,
                            "type": omission.kind.label(),
                        }))
                        .collect::<Vec<_>>(),
                    "kept_directories": outcome.kept,
                    "progress": progress,
                }),
                error: None,
            },
            Err(error) => JobEnd {
                status: if observer.is_cancelled() {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                },
                result: json!({ "progress": progress }),
                error: Some(error),
            },
        }
    })?;
    Ok(job)
}

fn progress_value(observer: &TreeObserver) -> Value {
    let progress = observer.progress();
    json!({
        "phase": progress.phase.label(),
        "files_total": progress.files_total,
        "directories_total": progress.directories_total,
        "bytes_total": progress.bytes_total,
        "files_read": progress.files_read,
        "bytes_read": progress.bytes_read,
        "files_written": progress.files_written,
        "bytes_written": progress.bytes_written,
    })
}
