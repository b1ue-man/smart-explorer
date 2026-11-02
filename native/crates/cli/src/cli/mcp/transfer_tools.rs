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
    let folders = match args
        .opt_string("folder_conflict")?
        .as_deref()
        .unwrap_or("merge")
    {
        "merge" => crate::transfer::FolderConflict::Merge,
        "keep-both" => crate::transfer::FolderConflict::KeepBoth,
        "fail" => crate::transfer::FolderConflict::Fail,
        _ => return Err("folder_conflict must be merge, keep-both or fail".into()),
    };
    let update = match args.opt_string("file_conflict")?.as_deref() {
        None => false,
        Some("update") => true,
        Some(_) => return Err("file_conflict must be update".into()),
    };
    if update && (overwrite || folders != crate::transfer::FolderConflict::Merge) {
        return Err(
            "file_conflict update cannot be combined with overwrite or another folder_conflict"
                .into(),
        );
    }
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
            _ if update => crate::cli::update_copy::run(
                &source,
                &destination,
                recursive,
                if kind == JobKind::Move {
                    crate::types::CopyMode::Move
                } else {
                    crate::types::CopyMode::Copy
                },
                &observer,
                Some(folders),
            ),
            JobKind::Move => crate::cli::ops::move_observed_with_folders(
                &source,
                &destination,
                recursive,
                overwrite,
                &observer,
                folders,
            ),
            _ => crate::cli::ops::copy_observed_with_folders(
                &source,
                &destination,
                recursive,
                overwrite,
                &observer,
                folders,
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
                    "update": outcome.update.map(|counts| json!({
                        "replaced": counts.replaced,
                        "skipped_equal": counts.skipped_equal,
                        "kept_newer": counts.kept_newer,
                        "kept_both": counts.kept_both,
                    })),
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
        // Engine copies only; `none`/0/`null` on the older path. The text
        // names drives and rates, never a file or folder.
        "rate_bps": progress.rate_bps,
        "limiter": progress.limiter,
        "limiter_text": progress.limiter_text,
        "segments": progress.segments,
        // A hint about the connection, such as the number of SSH
        // connections or why no further one opens; `null` without one.
        "note": progress.note,
        "optical_readers_scope": "process",
        "optical_readers": progress.optical_readers,
    })
}

#[cfg(test)]
mod tests {
    use super::progress_value;
    use crate::cli::tree_observer::{TreeObserver, TreePhase};

    #[test]
    fn speed_task_mcp_copy_progress_has_rate_and_limiter() {
        let observer = TreeObserver::default();
        // Before any verdict: the fields exist with neutral values.
        let idle = progress_value(&observer);
        assert_eq!(idle["rate_bps"], 0);
        assert_eq!(idle["limiter"], "none");
        assert!(idle["limiter_text"].is_null());
        assert_eq!(idle["segments"], 0);
        assert!(idle["note"].is_null());

        observer.begin_engine();
        observer.set_phase(TreePhase::Writing);
        observer.set_engine_totals(3, 0, 3_000);
        observer.set_engine_done(2, 2_000);
        observer.set_engine_stats(1_000_000, "target", Some("Ziellaufwerk sda".to_string()), 2);
        let value = progress_value(&observer);
        assert_eq!(value["rate_bps"], 1_000_000);
        assert_eq!(value["limiter"], "target");
        assert_eq!(value["limiter_text"], "Ziellaufwerk sda");
        assert_eq!(value["segments"], 2);
        observer.set_engine_note(Some("2 Verbindungen zu files.example".to_string()));
        assert_eq!(
            progress_value(&observer)["note"],
            "2 Verbindungen zu files.example"
        );
        // The fields of the older progress stay as they were.
        assert_eq!(value["phase"], "writing");
        assert_eq!(value["files_total"], 3);
        assert_eq!(value["directories_total"], 0);
        assert_eq!(value["bytes_total"], 3_000);
        assert_eq!(value["files_read"], 2);
        assert_eq!(value["bytes_read"], 2_000);
        assert_eq!(value["files_written"], 2);
        assert_eq!(value["bytes_written"], 2_000);
    }
}
