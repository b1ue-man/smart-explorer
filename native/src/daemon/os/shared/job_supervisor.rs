//! Queue identifiers and reload current configurations at admission. Pair
//! exclusion is enforced by the engine's process-wide/file-backed PairLock.
use crate::syncjobs::{
    AttemptOutcome, FailureKind, JobError, JobResult, RunCause, Runner, SyncJob,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub(super) const STOP_GRACE: Duration = Duration::from_secs(10);
const MIN_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const MAX_PARALLEL_JOBS: usize = 4;
type JobTask = Box<dyn FnOnce() + Send + 'static>;
type JobRunner = Arc<dyn Fn(&SyncJob, &AtomicBool) + Send + Sync>;
type ThreadSpawner = Arc<dyn Fn(String, JobTask) -> io::Result<JoinHandle<()>> + Send + Sync>;
type JobLoader = Arc<dyn Fn(&str) -> Result<Option<SyncJob>, String> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EnqueueStatus {
    Started,
    Queued,
    AlreadyScheduled,
    RecentlyAttempted,
}

struct ActiveJob {
    id: String,
    name: String,
    pair: (String, String),
    started: i64,
    cause: RunCause,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicI64>,
    outcome: Arc<Mutex<Option<AttemptOutcome>>>,
    cancel_since: Option<Instant>,
    handle: JoinHandle<()>,
}

pub(super) struct JobSupervisor {
    active: Vec<ActiveJob>,
    pending: VecDeque<(String, RunCause)>,
    scheduled: HashSet<String>,
    last_admitted: HashMap<String, Instant>,
    runner: Option<JobRunner>,
    loader: JobLoader,
    spawner: ThreadSpawner,
    capacity: usize,
    completed: Vec<String>,
    outcomes: HashMap<String, (AttemptOutcome, bool)>,
    last_beat: Instant,
    cancellation_warning: bool,
}

fn reload(id: &str) -> Result<Option<SyncJob>, String> {
    let report = crate::syncjobs::load_report().map_err(|error| error.to_string())?;
    Ok(report.jobs.into_iter().find(|job| job.id == id))
}

impl JobSupervisor {
    pub(super) fn new() -> Self {
        let mut supervisor = Self::with_hooks(
            Arc::new(super::job::run_one),
            Arc::new(|name, task| std::thread::Builder::new().name(name).spawn(task)),
        );
        supervisor.runner = None;
        supervisor.loader = Arc::new(reload);
        // At most one independent sync per available CPU. Transfer/walk
        // workers share the backend flow controllers; no unbounded job fanout.
        supervisor.capacity = std::thread::available_parallelism()
            .map_or(1, |cpus| cpus.get().min(MAX_PARALLEL_JOBS));
        supervisor
    }

    fn with_hooks(runner: JobRunner, spawner: ThreadSpawner) -> Self {
        Self {
            active: Vec::new(),
            pending: VecDeque::new(),
            scheduled: HashSet::new(),
            last_admitted: HashMap::new(),
            runner: Some(runner),
            spawner,
            capacity: 1,
            loader: Arc::new(|id| {
                let mut job = SyncJob::new(id.to_string(), "/source".into(), "/target".into());
                job.id = id.to_string();
                Ok(Some(job))
            }),
            completed: Vec::new(),
            outcomes: HashMap::new(),
            last_beat: Instant::now(),
            cancellation_warning: false,
        }
    }

    pub(super) fn enqueue(&mut self, job: &SyncJob) -> Result<EnqueueStatus, String> {
        self.enqueue_cause(job, RunCause::CatchUp)
    }

    pub(super) fn enqueue_cause(
        &mut self,
        job: &SyncJob,
        cause: RunCause,
    ) -> Result<EnqueueStatus, String> {
        if self.scheduled.contains(&job.id) {
            return Ok(EnqueueStatus::AlreadyScheduled);
        }
        let now = Instant::now();
        self.last_admitted.retain(|_, allowed_at| now < *allowed_at);
        if self.last_admitted.contains_key(&job.id) {
            return Ok(EnqueueStatus::RecentlyAttempted);
        }
        self.scheduled.insert(job.id.clone());
        self.last_admitted
            .insert(job.id.clone(), now + MIN_RETRY_INTERVAL);
        self.pending.push_back((job.id.clone(), cause));
        if self.active.len() < self.capacity {
            Ok(if self.start_next()? {
                EnqueueStatus::Started
            } else {
                EnqueueStatus::Queued
            })
        } else {
            Ok(EnqueueStatus::Queued)
        }
    }

    pub(super) fn poll(&mut self) -> Vec<String> {
        let mut errors = self.reap();
        if self.last_beat.elapsed() >= Duration::from_secs(30) {
            self.last_beat = Instant::now();
            let now = super::state::now_secs();
            for active in &self.active {
                let progress = active.progress.load(Ordering::Acquire);
                if let Err(error) = crate::syncjobs::update_job_state(&active.id, |state| {
                    if let Some(mark) = state.running.as_mut().filter(|mark| {
                        mark.runner == Runner::Daemon && mark.started == active.started
                    }) {
                        mark.alive = now;
                        // This is a progress indicator, not a destructive
                        // timeout: a single large file may legitimately take
                        // longer than the run-mark liveness interval.
                        mark.stalled_since = (now.saturating_sub(progress)
                            >= crate::syncjobs::RUN_MARK_STALE_SECS)
                            .then_some(progress);
                    }
                }) {
                    errors.push(format!("job '{}' heartbeat failed: {error}", active.id));
                }
            }
        }
        // A pair held by an active job goes back to the queue. Visit each
        // waiting id at most once per tick, allowing independent pairs past it.
        for _ in 0..self.pending.len() {
            if self.active.len() >= self.capacity || self.pending.is_empty() {
                break;
            }
            if let Err(error) = self.start_next() {
                errors.push(error);
            }
        }
        errors
    }

    fn reap(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut index = 0;
        while index < self.active.len() {
            if !self.active[index].handle.is_finished() {
                index += 1;
                continue;
            }
            let active = self.active.remove(index);
            let panicked = active.handle.join().is_err();
            if panicked {
                let message = format!("daemon job '{}' panicked", active.id);
                let mut job = SyncJob::new(active.name, String::new(), String::new());
                job.id = active.id.clone();
                if self.runner.is_none() {
                    super::job::persist(
                        &job,
                        active.started,
                        active.cause,
                        AttemptOutcome::Failed(JobError {
                            kind: FailureKind::Internal,
                            message: message.clone(),
                        }),
                        JobResult {
                            when: super::state::now_secs(),
                            errors: 1,
                            note: message.clone(),
                            ..Default::default()
                        },
                    );
                }
                errors.push(message);
            }
            let outcome = if panicked {
                AttemptOutcome::Failed(JobError {
                    kind: FailureKind::Internal,
                    message: "Sync-Worker abgebrochen".into(),
                })
            } else {
                active
                    .outcome
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                    .unwrap_or(AttemptOutcome::Cancelled)
            };
            let ran = matches!(outcome, AttemptOutcome::Success | AttemptOutcome::Failed(_));
            self.finish(active.id, (outcome, ran));
        }
        errors
    }

    fn finish(&mut self, id: String, outcome: (AttemptOutcome, bool)) {
        if outcome.1
            && matches!(&outcome.0, AttemptOutcome::Failed(error) if error.kind == FailureKind::Internal)
        {
            // A failed state write may be unable to persist its own retry.
            // Keep a process-local brake instead of attempting every tick.
            self.last_admitted
                .insert(id.clone(), Instant::now() + Duration::from_secs(300));
        }
        self.scheduled.remove(&id);
        self.outcomes.insert(id.clone(), outcome);
        self.completed.push(id);
    }

    pub(super) fn cancel_and_join(&mut self) -> Vec<String> {
        let queued: Vec<_> = self.pending.drain(..).map(|(id, _)| id).collect();
        for id in queued {
            self.finish(id, (AttemptOutcome::Cancelled, false));
        }
        let now = Instant::now();
        for active in &mut self.active {
            active.cancel.store(true, Ordering::Release);
            active.cancel_since.get_or_insert(now);
        }
        let deadline = self
            .active
            .iter()
            .filter_map(|active| active.cancel_since)
            .min()
            .and_then(|since| since.checked_add(STOP_GRACE));
        let mut errors = self.reap();
        while !self.active.is_empty() && deadline.is_some_and(|at| Instant::now() < at) {
            std::thread::sleep(Duration::from_millis(20));
            errors.extend(self.reap());
        }
        if !self.active.is_empty() && !self.cancellation_warning {
            errors.push("Sync-Abbruch wartet auf blockierende E/A; Paar bleibt gesperrt.".into());
            self.cancellation_warning = true;
        }
        if self.active.is_empty() {
            self.cancellation_warning = false;
        }
        errors
    }

    pub(super) fn cancel_jobs(&mut self, ids: &HashSet<String>) {
        let mut canceled = Vec::new();
        self.pending.retain(|(id, _)| {
            if ids.contains(id) {
                canceled.push(id.clone());
                false
            } else {
                true
            }
        });
        for id in canceled {
            self.finish(id, (AttemptOutcome::Cancelled, false));
        }
        for active in self
            .active
            .iter_mut()
            .filter(|active| ids.contains(&active.id))
        {
            active.cancel.store(true, Ordering::Release);
            active.cancel_since.get_or_insert_with(Instant::now);
        }
    }
    pub(super) fn take_completed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.completed)
    }
    pub(super) fn completion(&mut self, id: &str) -> (AttemptOutcome, bool) {
        self.outcomes
            .remove(id)
            .unwrap_or((AttemptOutcome::Cancelled, false))
    }
    pub(super) fn active_ids(&self) -> Vec<&str> {
        self.active.iter().map(|a| a.id.as_str()).collect()
    }
    pub(super) fn active_job_id(&self) -> Option<&str> {
        self.active.first().map(|a| a.id.as_str())
    }
    pub(super) fn active_job_name(&self) -> Option<&str> {
        self.active.first().map(|a| {
            if a.name.trim().is_empty() {
                a.id.as_str()
            } else {
                a.name.as_str()
            }
        })
    }
    fn start_next(&mut self) -> Result<bool, String> {
        if self.active.len() >= self.capacity {
            return Ok(false);
        }
        let Some((id, cause)) = self.pending.pop_front() else {
            return Ok(false);
        };
        let job = match (self.loader)(&id) {
            Ok(Some(job))
                if job.enabled
                    && job.active_now(super::state::now_secs())
                    && (self.runner.is_some() || current_trigger(&job, cause)) =>
            {
                job
            }
            Ok(_) => {
                self.finish(id, (AttemptOutcome::Cancelled, false));
                return Ok(false);
            }
            Err(error) => {
                self.finish(id, (AttemptOutcome::Cancelled, false));
                return Err(error);
            }
        };
        let pair = if job.source <= job.target {
            (job.source.clone(), job.target.clone())
        } else {
            (job.target.clone(), job.source.clone())
        };
        if self.active.iter().any(|active| active.pair == pair) {
            // Equal configured locators identify the same logical pair before
            // hooks/opening. Physical aliases still use the engine PairLock.
            self.pending.push_back((id, cause));
            return Ok(false);
        }
        if self.runner.is_none() {
            let state = match crate::syncjobs::load_job_state(&id) {
                Ok(state) => state,
                Err(error) => {
                    self.finish(id, (AttemptOutcome::Cancelled, false));
                    return Err(error.to_string());
                }
            };
            if !admission_allowed(&state, cause, super::state::now_secs()) {
                self.finish(id, (AttemptOutcome::Cancelled, false));
                return Ok(false);
            }
        }
        let name = job.name.clone();
        let started = super::state::now_secs();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let progress = Arc::new(AtomicI64::new(started));
        let worker_progress = progress.clone();
        let runner = self.runner.clone();
        let failed_job = job.clone();
        let outcome = Arc::new(Mutex::new(None));
        let worker_outcome = outcome.clone();
        let task: JobTask = Box::new(move || {
            let _storage =
                super::host_state::register_storage_run(&job.source, &job.target, &worker_cancel);
            let result = match runner {
                Some(runner) => {
                    runner(&job, &worker_cancel);
                    AttemptOutcome::Success
                }
                None => super::job::run_for(&job, &worker_cancel, cause, worker_progress, started),
            };
            *worker_outcome
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(result);
        });
        match (self.spawner)(format!("daemon-job-{id}"), task) {
            Ok(handle) => {
                self.active.push(ActiveJob {
                    id,
                    name,
                    pair,
                    started,
                    cause,
                    cancel,
                    progress,
                    outcome,
                    cancel_since: None,
                    handle,
                });
                Ok(true)
            }
            Err(error) => {
                if self.runner.is_none() {
                    super::job::persist(
                        &failed_job,
                        started,
                        cause,
                        AttemptOutcome::Failed(JobError {
                            kind: FailureKind::Internal,
                            message: error.to_string(),
                        }),
                        JobResult {
                            when: started,
                            errors: 1,
                            note: error.to_string(),
                            ..Default::default()
                        },
                    );
                }
                self.last_admitted.remove(&id);
                self.finish(
                    id,
                    (
                        AttemptOutcome::Failed(JobError {
                            kind: FailureKind::Internal,
                            message: error.to_string(),
                        }),
                        false,
                    ),
                );
                Err(format!("job spawn failed for '{name}': {error}"))
            }
        }
    }
    #[cfg(test)]
    fn is_idle(&self) -> bool {
        self.active.is_empty() && self.pending.is_empty()
    }
}
fn admission_allowed(state: &crate::syncjobs::JobState, cause: RunCause, now: i64) -> bool {
    if state.load_error.is_some()
        || state
            .running_now(now)
            .is_some_and(|mark| mark.runner != Runner::Daemon)
    {
        return false;
    }
    if cause == RunCause::Confirmed {
        return state.blocked.as_ref().is_some_and(|block| block.confirmed)
            || state
                .pending_trigger
                .as_ref()
                .is_some_and(|pending| pending.kind == crate::syncjobs::PendingKind::Confirmed);
    }
    if state.blocked.is_some() {
        return false;
    }
    state.consecutive_failures == 0
        || (cause == RunCause::Retry
            && !state
                .last_error
                .as_ref()
                .is_some_and(|error| error.kind.needs_user())
            && state.retry_at.is_some_and(|at| at <= now))
}
fn current_trigger(job: &SyncJob, cause: RunCause) -> bool {
    use crate::syncjobs::Trigger;
    match cause {
        RunCause::Interval => job.trigger == Trigger::Interval,
        RunCause::Calendar => job.trigger == Trigger::Calendar,
        RunCause::Startup => job.trigger == Trigger::OnStartup,
        RunCause::Connect => job.trigger == Trigger::OnConnect,
        RunCause::Change | RunCause::Poll => job.trigger == Trigger::RealTime,
        RunCause::CatchUp => matches!(
            job.trigger,
            Trigger::Interval | Trigger::Calendar | Trigger::RealTime
        ),
        RunCause::Verify | RunCause::Retry => job.trigger != Trigger::Manual,
        _ => true,
    }
}

impl Drop for JobSupervisor {
    fn drop(&mut self) {
        let _ = self.cancel_and_join();
    }
}

#[cfg(test)]
#[path = "job_supervisor_tests.rs"]
mod tests;
