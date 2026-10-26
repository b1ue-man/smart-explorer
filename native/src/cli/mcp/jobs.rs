//! Long-running work of one MCP session (copies, moves, searches, remote and
//! `se` commands). A tool call waits up to its `wait_seconds` and otherwise
//! returns the job id; `job_status`, `job_input` and `job_cancel` continue
//! from there. Jobs live as long as the `se mcp` process; its end cancels
//! every running job.
use super::spool::Spool;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JobKind {
    Copy,
    Move,
    Search,
    Exec,
    Command,
}

impl JobKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Search => "search",
            Self::Exec => "exec",
            Self::Command => "se",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JobStatus {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

impl JobStatus {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct JobEnd {
    pub(super) status: JobStatus,
    /// Kind-specific final fields (exit code, destination, hit count, …).
    pub(super) result: Value,
    pub(super) error: Option<String>,
}

/// How a job's worker is told to stop or fed input.
pub(super) trait JobControl: Send + Sync {
    fn cancel(&self);

    fn stdin(&self, _data: &[u8]) -> Result<(), String> {
        Err("this job does not read input".to_string())
    }

    fn eof(&self) -> Result<(), String> {
        Err("this job does not read input".to_string())
    }

    /// Live progress fields of a running job.
    fn progress(&self) -> Value {
        Value::Null
    }
}

pub(super) struct Job {
    pub(super) id: String,
    pub(super) kind: JobKind,
    pub(super) summary: String,
    started_ms: i64,
    end: Mutex<Option<JobEnd>>,
    control: Mutex<Option<Arc<dyn JobControl>>>,
    operation: Mutex<Option<crate::mcp_launcher::OperationGuard>>,
    cancel_requested: AtomicBool,
    /// Bumped on every new output, progress step and on the end.
    version: watch::Sender<u64>,
    streams: Vec<(&'static str, Spool)>,
}

impl Job {
    pub(super) fn set_control(&self, control: Arc<dyn JobControl>) {
        let cancel = self.cancel_requested.load(Ordering::SeqCst);
        if let Ok(mut slot) = self.control.lock() {
            *slot = Some(control.clone());
        }
        if cancel {
            control.cancel();
        }
    }

    pub(super) fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
        if let Some(control) = self.control() {
            control.cancel();
        }
    }

    pub(super) fn cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }

    pub(super) fn control(&self) -> Option<Arc<dyn JobControl>> {
        self.control.lock().ok().and_then(|slot| slot.clone())
    }

    pub(super) fn stream(&self, name: &str) -> Option<&Spool> {
        self.streams
            .iter()
            .find(|(stream, _)| *stream == name)
            .map(|(_, spool)| spool)
    }

    pub(super) fn stream_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.streams.iter().map(|(name, _)| *name)
    }

    /// Appends output; a failing spool write ends the job as failed, because
    /// output that cannot be kept must not look complete.
    pub(super) fn append(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let spool = self
            .stream(name)
            .ok_or_else(|| format!("job has no `{name}` output"))?;
        spool
            .append(bytes)
            .map_err(|error| format!("cannot keep job output: {error}"))?;
        self.changed();
        Ok(())
    }

    pub(super) fn changed(&self) {
        self.version
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    pub(super) fn release_operation_after_handoff(&self) {
        if let Ok(mut guard) = self.operation.lock() {
            guard.take();
        }
    }

    pub(super) fn finish(&self, end: JobEnd) {
        self.release_operation_after_handoff();
        if let Ok(mut slot) = self.end.lock() {
            if slot.is_none() {
                *slot = Some(end);
            }
        }
        self.changed();
    }

    pub(super) fn end(&self) -> Option<JobEnd> {
        self.end.lock().ok().and_then(|slot| slot.clone())
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }

    /// Identity, state and progress, without output pages.
    pub(super) fn summary_value(&self) -> Value {
        let mut value = json!({
            "job_id": self.id,
            "kind": self.kind.label(),
            "summary": self.summary,
            "started_ms": self.started_ms,
        });
        let object = value.as_object_mut().expect("json object literal");
        match self.end() {
            None => {
                object.insert("status".into(), json!("running"));
                if let Some(progress) = self.control().map(|control| control.progress()) {
                    if !progress.is_null() {
                        object.insert("progress".into(), progress);
                    }
                }
            }
            Some(end) => {
                object.insert("status".into(), json!(end.status.label()));
                if let Some(error) = end.error {
                    object.insert("error".into(), json!(error));
                }
                if let Value::Object(result) = end.result {
                    object.extend(result);
                }
            }
        }
        for (name, spool) in &self.streams {
            object.insert(format!("{name}_bytes"), json!(spool.len()));
        }
        value
    }
}

pub(super) struct JobRegistry {
    directory: std::path::PathBuf,
    next: AtomicU64,
    jobs: Mutex<BTreeMap<u64, Arc<Job>>>,
}

impl JobRegistry {
    pub(super) fn new(directory: &Path) -> Self {
        Self {
            directory: directory.to_path_buf(),
            next: AtomicU64::new(1),
            jobs: Mutex::new(BTreeMap::new()),
        }
    }

    /// Registers a job with the named output streams (spool files in the
    /// session directory).
    pub(super) fn create(
        &self,
        kind: JobKind,
        summary: String,
        streams: &[&'static str],
    ) -> Result<Arc<Job>, String> {
        let number = self.next.fetch_add(1, Ordering::SeqCst);
        let id = format!("j{number}");
        let streams = streams
            .iter()
            .map(|name| {
                Spool::create(self.directory.join(format!("{id}.{name}")))
                    .map(|spool| (*name, spool))
                    .map_err(|error| format!("cannot create job output file: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (version, _) = watch::channel(0);
        let job = Arc::new(Job {
            id,
            kind,
            summary,
            started_ms: now_ms(),
            end: Mutex::new(None),
            control: Mutex::new(None),
            operation: Mutex::new(crate::mcp_launcher::continuation_operation()?),
            cancel_requested: AtomicBool::new(false),
            version,
            streams,
        });
        self.jobs
            .lock()
            .map_err(|_| "job registry is poisoned".to_string())?
            .insert(number, job.clone());
        Ok(job)
    }

    pub(super) fn get(&self, id: &str) -> Option<Arc<Job>> {
        let number = id.trim().strip_prefix('j')?.parse::<u64>().ok()?;
        self.jobs.lock().ok()?.get(&number).cloned()
    }

    pub(super) fn list(&self) -> Vec<Arc<Job>> {
        self.jobs
            .lock()
            .map(|jobs| jobs.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn cancel_running(&self) -> Vec<Arc<Job>> {
        let running = self
            .list()
            .into_iter()
            .filter(|job| job.end().is_none())
            .collect::<Vec<_>>();
        for job in &running {
            job.cancel();
        }
        running
    }
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// Runs `work` on its own thread; a panic ends the job as failed instead of
/// leaving it running forever.
pub(super) fn spawn_worker(
    job: Arc<Job>,
    work: impl FnOnce(&Arc<Job>) -> JobEnd + Send + 'static,
) -> Result<(), String> {
    let thread_job = job.clone();
    std::thread::Builder::new()
        .name(format!("se-mcp-{}", job.id))
        .spawn(move || {
            let end = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&thread_job)))
                .unwrap_or_else(|_| JobEnd {
                    status: JobStatus::Failed,
                    result: Value::Null,
                    error: Some("the job's worker panicked".to_string()),
                });
            thread_job.finish(end);
        })
        .map(|_| ())
        .map_err(|error| format!("cannot start job worker: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{JobControl, JobEnd, JobKind, JobRegistry, JobStatus};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct Flag(AtomicBool);

    impl JobControl for Flag {
        fn cancel(&self) {
            self.0.store(true, Ordering::SeqCst);
        }

        fn progress(&self) -> Value {
            json!({"done": 1})
        }
    }

    #[test]
    fn mcp_task_jobs_keep_output_and_apply_early_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let registry = JobRegistry::new(dir.path());
        let job = registry
            .create(JobKind::Exec, "exec test".into(), &["stdout", "stderr"])
            .unwrap();
        assert_eq!(job.id, "j1");
        job.append("stdout", b"out").unwrap();
        assert_eq!(job.summary_value()["stdout_bytes"], 3);
        // A cancel before the worker installed its control reaches it later.
        job.cancel();
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        job.set_control(flag.clone());
        assert!(flag.0.load(Ordering::SeqCst));
        assert_eq!(job.summary_value()["progress"]["done"], 1);
        job.finish(JobEnd {
            status: JobStatus::Cancelled,
            result: json!({"exit_code": 130}),
            error: None,
        });
        let value = job.summary_value();
        assert_eq!(value["status"], "cancelled");
        assert_eq!(value["exit_code"], 130);
        assert!(registry.get("j1").is_some());
        assert!(registry.get("x1").is_none());
        assert!(registry.cancel_running().is_empty());
    }
}
