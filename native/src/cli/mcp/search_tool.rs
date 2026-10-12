//! `search` as a job: the `se search` walk (server-side where available)
//! streams hits as JSON lines into the job's `hits` output. Links and special
//! files are hits of their own type (`symlink`, `special`) and never entered.
use super::args::Args;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

struct SearchControl {
    cancel: Arc<AtomicBool>,
    hits: Arc<AtomicU64>,
}

impl JobControl for SearchControl {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn progress(&self) -> Value {
        json!({ "hits": self.hits.load(Ordering::SeqCst) })
    }
}

pub(super) fn start(session: &Session, args: &Args<'_>) -> Result<Arc<Job>, String> {
    let target_spec = args.string("target")?;
    let query = args.opt_string("query")?.unwrap_or_default();
    let spec = crate::cli::search::search_spec(
        &query,
        args.flag("glob")?,
        args.opt_u64("max_results")?.unwrap_or(0),
        args.flag("directories")?,
    );
    let target = session.targets.resolve(&target_spec)?;
    let job = session.jobs.create(
        JobKind::Search,
        format!("search {target_spec} for {query:?}"),
        &["hits"],
    )?;
    let cancel = Arc::new(AtomicBool::new(false));
    let hits = Arc::new(AtomicU64::new(0));
    job.set_control(Arc::new(SearchControl {
        cancel: cancel.clone(),
        hits: hits.clone(),
    }));
    spawn_worker(job.clone(), move |job| {
        let mut write_error = None;
        let outcome = crate::cli::search::search_with(&target, &spec, &cancel, &mut |hit| {
            if write_error.is_some() {
                return;
            }
            let mut line = json!({
                "path": crate::cli::ops::join(&target.path, &hit.rel),
                "relative": hit.rel,
                "type": hit.kind.label(),
                "size": hit.size,
                "mtime_ms": hit.mtime_ms,
            })
            .to_string()
            .into_bytes();
            line.push(b'\n');
            match job.append("hits", &line) {
                Ok(()) => {
                    hits.fetch_add(1, Ordering::SeqCst);
                }
                Err(error) => {
                    write_error = Some(error);
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        });
        let result = json!({ "root": target.path, "hits_total": hits.load(Ordering::SeqCst) });
        match (outcome, write_error) {
            (_, Some(error)) => JobEnd {
                status: JobStatus::Failed,
                result,
                error: Some(error),
            },
            (Ok(()), None) => JobEnd {
                status: JobStatus::Completed,
                result,
                error: None,
            },
            (Err(error), None) => JobEnd {
                status: if job.cancel_requested() {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                },
                result,
                error: Some(error),
            },
        }
    })?;
    Ok(job)
}
