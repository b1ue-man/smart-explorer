use crate::syncjobs::SyncJob;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MIN_RETRY_INTERVAL: Duration = Duration::from_secs(60);

use super::job::run_one;

type JobTask = Box<dyn FnOnce() + Send + 'static>;
type JobRunner = Arc<dyn Fn(&SyncJob, &AtomicBool) + Send + Sync>;
type ThreadSpawner = Arc<dyn Fn(String, JobTask) -> io::Result<JoinHandle<()>> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EnqueueStatus {
    Started,
    Queued,
    AlreadyScheduled,
    RecentlyAttempted,
}

struct ActiveJob {
    id: String,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

/// Owns daemon job threads so expensive sync work is globally serialized and
/// can be cooperatively canceled and joined during daemon shutdown.
pub(super) struct JobSupervisor {
    active: Option<ActiveJob>,
    pending: VecDeque<SyncJob>,
    scheduled: HashSet<String>,
    last_admitted: HashMap<String, Instant>,
    runner: JobRunner,
    spawner: ThreadSpawner,
}

impl JobSupervisor {
    pub(super) fn new() -> Self {
        Self::with_hooks(
            Arc::new(run_one),
            Arc::new(|name, task| std::thread::Builder::new().name(name).spawn(task)),
        )
    }

    fn with_hooks(runner: JobRunner, spawner: ThreadSpawner) -> Self {
        Self {
            active: None,
            pending: VecDeque::new(),
            scheduled: HashSet::new(),
            last_admitted: HashMap::new(),
            runner,
            spawner,
        }
    }

    pub(super) fn enqueue(&mut self, job: &SyncJob) -> Result<EnqueueStatus, String> {
        if self.scheduled.contains(&job.id) {
            return Ok(EnqueueStatus::AlreadyScheduled);
        }
        let now = Instant::now();
        self.last_admitted
            .retain(|_, admitted| now.duration_since(*admitted) < Duration::from_secs(86_400));
        if self
            .last_admitted
            .get(&job.id)
            .is_some_and(|admitted| now.duration_since(*admitted) < MIN_RETRY_INTERVAL)
        {
            return Ok(EnqueueStatus::RecentlyAttempted);
        }
        self.scheduled.insert(job.id.clone());
        self.last_admitted.insert(job.id.clone(), now);
        self.pending.push_back(job.clone());
        if self.active.is_some() {
            Ok(EnqueueStatus::Queued)
        } else {
            self.start_next()?;
            Ok(EnqueueStatus::Started)
        }
    }

    /// Reap a completed worker and start at most one successor. Failures are
    /// returned to the daemon loop so they are visible in its persistent log.
    pub(super) fn poll(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.handle.is_finished())
        {
            if let Some(active) = self.active.take() {
                let id = active.id;
                if active.handle.join().is_err() {
                    errors.push(format!("daemon job '{id}' panicked"));
                }
                self.scheduled.remove(&id);
            }
        }
        if self.active.is_none() {
            if let Err(error) = self.start_next() {
                errors.push(error);
            }
        }
        errors
    }

    /// Cancel active work, discard queued work, and wait until the worker has
    /// observed cancellation and returned. No job thread outlives this call.
    pub(super) fn cancel_and_join(&mut self) -> Vec<String> {
        self.pending.clear();
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Release);
        }
        let mut errors = Vec::new();
        if let Some(active) = self.active.take() {
            if active.handle.join().is_err() {
                errors.push(format!("daemon job '{}' panicked during stop", active.id));
            }
        }
        self.scheduled.clear();
        self.last_admitted.clear();
        errors
    }

    fn start_next(&mut self) -> Result<bool, String> {
        if self.active.is_some() {
            return Ok(false);
        }
        let Some(job) = self.pending.pop_front() else {
            return Ok(false);
        };
        let id = job.id.clone();
        let name = job.name.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let runner = self.runner.clone();
        let task: JobTask = Box::new(move || runner(&job, &worker_cancel));
        match (self.spawner)(format!("daemon-job-{id}"), task) {
            Ok(handle) => {
                self.active = Some(ActiveJob { id, cancel, handle });
                Ok(true)
            }
            Err(error) => {
                self.scheduled.remove(&id);
                self.last_admitted.remove(&id);
                Err(format!("job spawn failed for '{name}': {error}"))
            }
        }
    }

    #[cfg(test)]
    fn is_idle(&self) -> bool {
        self.active.is_none() && self.pending.is_empty()
    }
}

impl Drop for JobSupervisor {
    fn drop(&mut self) {
        let _ = self.cancel_and_join();
    }
}

#[cfg(test)]
mod tests {
    use super::{EnqueueStatus, JobRunner, JobSupervisor, ThreadSpawner};
    use crate::syncjobs::SyncJob;
    use std::io;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn job(id: &str) -> SyncJob {
        let mut job = SyncJob::new(id.to_string(), "/source".into(), "/target".into());
        job.id = id.to_string();
        job
    }

    fn thread_spawner() -> ThreadSpawner {
        Arc::new(|name, task| std::thread::Builder::new().name(name).spawn(task))
    }

    fn poll_until_idle(supervisor: &mut JobSupervisor) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !supervisor.is_idle() && Instant::now() < deadline {
            assert!(supervisor.poll().is_empty());
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(supervisor.is_idle());
    }

    #[test]
    fn serializes_all_daemon_jobs_globally() {
        let running = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let runner: JobRunner = {
            let running = running.clone();
            let maximum = maximum.clone();
            let completed = completed.clone();
            Arc::new(move |_, _| {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
                running.fetch_sub(1, Ordering::SeqCst);
                completed.fetch_add(1, Ordering::SeqCst);
            })
        };
        let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());

        assert_eq!(
            supervisor.enqueue(&job("one")).unwrap(),
            EnqueueStatus::Started
        );
        assert_eq!(
            supervisor.enqueue(&job("two")).unwrap(),
            EnqueueStatus::Queued
        );
        poll_until_idle(&mut supervisor);

        assert_eq!(completed.load(Ordering::SeqCst), 2);
        assert_eq!(maximum.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stop_cancels_and_joins_active_job_without_starting_pending_work() {
        let started = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let runner: JobRunner = {
            let started = started.clone();
            let finished = finished.clone();
            Arc::new(move |_, cancel| {
                started.fetch_add(1, Ordering::SeqCst);
                while !cancel.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                finished.store(true, Ordering::Release);
            })
        };
        let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());
        supervisor.enqueue(&job("active")).unwrap();
        supervisor.enqueue(&job("pending")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while started.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }

        assert!(supervisor.cancel_and_join().is_empty());
        assert!(finished.load(Ordering::Acquire));
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert!(supervisor.is_idle());
    }

    #[test]
    fn spawn_failure_is_returned_and_does_not_reserve_the_job() {
        let runner: JobRunner = Arc::new(|_, _| {});
        let spawner: ThreadSpawner =
            Arc::new(|_, _| Err(io::Error::other("injected spawn failure")));
        let mut supervisor = JobSupervisor::with_hooks(runner, spawner);

        let error = supervisor.enqueue(&job("failed")).unwrap_err();
        assert!(error.contains("injected spawn failure"));
        assert!(supervisor.is_idle());
        assert_eq!(supervisor.enqueue(&job("failed")).unwrap_err(), error);
    }

    #[test]
    fn completed_job_is_not_requeued_in_a_tight_loop() {
        let runner: JobRunner = Arc::new(|_, _| {});
        let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());
        assert_eq!(
            supervisor.enqueue(&job("cooldown")).unwrap(),
            EnqueueStatus::Started
        );
        poll_until_idle(&mut supervisor);
        assert_eq!(
            supervisor.enqueue(&job("cooldown")).unwrap(),
            EnqueueStatus::RecentlyAttempted
        );
    }
}
