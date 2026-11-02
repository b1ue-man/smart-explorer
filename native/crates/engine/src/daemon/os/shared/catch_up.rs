//! Catch-up runs ("Nachhol-Lauf"): a host that woke the app process (Android
//! periodic work) asks the running worker to catch up once. The run uses the
//! worker's own supervisor, so it shares its serialization, dedupe and retry
//! cooldown with the regular schedule instead of being a second scheduler.
//!
//! A run admits the due interval jobs, calendar occurrences missed since the
//! job's last run (regardless of the job's own catch-up setting) and every
//! enabled real-time job once. A selected job the regular schedule already
//! holds is awaited but stays the schedule's: cancelling the run leaves it
//! running. A run finishes when all its admitted and awaited jobs have left
//! the supervisor; supervisor rejections are listed with their reason.

use std::collections::{HashSet, VecDeque};

use crate::syncjobs::{AttemptOutcome, JobState, RunCause, SyncJob, Trigger};

use super::job_supervisor::{EnqueueStatus, JobSupervisor};

#[path = "catch_up_attempt.rs"]
mod attempt;
use attempt::{start_run, update_progress};

/// Finished runs kept for status queries; open runs are never dropped.
const MAX_FINISHED_RUNS: usize = 16;
/// Upper bound for simultaneously open runs (each is serviced within seconds).
const MAX_OPEN_RUNS: usize = 32;

#[path = "catch_up_types.rs"]
mod types;
pub use types::{CatchUpRecord, CatchUpSkip, CatchUpStatus};
/// Whether scheduled work may run right now; `Closed` ends open runs at once.
pub(super) enum CatchUpGate {
    Open,
    Closed(String),
}

/// The supervisor operations a run needs (implemented by `JobSupervisor`).
pub(super) trait CatchUpQueue {
    fn admit(&mut self, job: &SyncJob) -> Result<EnqueueStatus, String>;
    fn cancel_jobs(&mut self, ids: &HashSet<String>);
    fn take_completed(&mut self) -> Vec<String>;
    fn active_job_id(&self) -> Option<&str>;
    fn completion(&mut self, _id: &str) -> (AttemptOutcome, bool) {
        (AttemptOutcome::Success, true)
    }
    fn active_ids(&self) -> Vec<&str> {
        self.active_job_id().into_iter().collect()
    }
    fn eligible(&self, job: &SyncJob, now: i64) -> bool {
        catch_up_due(job, now)
    }
}

impl CatchUpQueue for JobSupervisor {
    fn admit(&mut self, job: &SyncJob) -> Result<EnqueueStatus, String> {
        let state = crate::syncjobs::load_job_state(&job.id).map_err(|error| error.to_string())?;
        let cause = catch_up_cause(job, &state, super::state::now_secs())
            .ok_or_else(|| "nicht mehr fällig oder auf Nutzeraktion wartend".to_string())?;
        self.enqueue_cause(job, cause)
    }

    fn cancel_jobs(&mut self, ids: &HashSet<String>) {
        JobSupervisor::cancel_jobs(self, ids);
    }

    fn take_completed(&mut self) -> Vec<String> {
        JobSupervisor::take_completed(self)
    }

    fn active_job_id(&self) -> Option<&str> {
        JobSupervisor::active_job_id(self)
    }
    fn completion(&mut self, id: &str) -> (AttemptOutcome, bool) {
        JobSupervisor::completion(self, id)
    }
    fn active_ids(&self) -> Vec<&str> {
        JobSupervisor::active_ids(self)
    }
    fn eligible(&self, job: &SyncJob, now: i64) -> bool {
        let Ok(state) = crate::syncjobs::load_job_state(&job.id) else {
            return false;
        };
        catch_up_cause(job, &state, now).is_some()
    }
}

fn catch_up_cause(job: &SyncJob, state: &JobState, now: i64) -> Option<RunCause> {
    if !job.enabled
        || !job.active_now(now)
        || state.load_error.is_some()
        || state
            .running_now(now)
            .is_some_and(|mark| mark.runner != crate::syncjobs::Runner::Daemon)
    {
        return None;
    }
    // A host-deferred worker also handles its durable startup/volume triggers
    // in this window. They must keep their actual cause, especially volume
    // identity, confirmation and failure backoff.
    if let Some(cause) = super::due::due_now(job, state, now, None) {
        return Some(
            if matches!(
                cause,
                RunCause::Interval | RunCause::Calendar | RunCause::Change | RunCause::Poll
            ) {
                RunCause::CatchUp
            } else {
                cause
            },
        );
    }
    if state.blocked.is_some() || state.consecutive_failures > 0 {
        return None;
    }
    match job.trigger {
        Trigger::RealTime => Some(RunCause::CatchUp),
        Trigger::Interval | Trigger::Calendar => {
            let mut missed = job.clone();
            missed.catch_up = true;
            missed
                .due_at(super::due::anchor(job, state, now), now, None)
                .then_some(RunCause::CatchUp)
        }
        _ => None,
    }
}

/// Jobs a catch-up run selects at `now`, in saved order.
pub(super) fn select_catch_up_jobs(jobs: &[SyncJob], now: i64) -> Vec<&SyncJob> {
    jobs.iter().filter(|job| catch_up_due(job, now)).collect()
}

fn catch_up_due(job: &SyncJob, now: i64) -> bool {
    if !job.enabled || !job.active_now(now) {
        return false;
    }
    match job.trigger {
        Trigger::Interval => job.due(now),
        Trigger::Calendar => {
            // Any occurrence after the last run counts, even when the job
            // itself would not catch up a missed occurrence.
            let mut missed = job.clone();
            missed.catch_up = true;
            missed.due(now)
        }
        Trigger::RealTime => true,
        Trigger::Manual | Trigger::OnStartup | Trigger::OnConnect => false,
    }
}

/// What one servicing pass started and finished (for the worker log).
#[derive(Debug, Default)]
pub(super) struct ServiceReport {
    pub(super) started: Vec<(u64, usize, usize)>,
    pub(super) finished: Vec<(u64, String)>,
    pub(super) records: Vec<CatchUpRecord>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Requested,
    Running,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cancel {
    None,
    Requested,
    Applied,
}

struct Admitted {
    id: String,
    name: String,
    done: bool,
    /// Enqueued by this run; `false` = awaited, the regular schedule owns it.
    owned: bool,
    outcome: Option<AttemptOutcome>,
    ran: bool,
}

struct Run {
    id: u64,
    phase: Phase,
    cancel: Cancel,
    admitted: Vec<Admitted>,
    skipped: Vec<CatchUpSkip>,
    message: Option<String>,
    running_job: Option<String>,
    queued: usize,
}

impl Run {
    fn is_open(&self) -> bool {
        self.phase != Phase::Finished
    }

    /// This run's own unfinished jobs (awaited ones belong to the schedule).
    fn undone_ids(&self) -> HashSet<String> {
        self.admitted
            .iter()
            .filter(|job| job.owned && !job.done)
            .map(|job| job.id.clone())
            .collect()
    }

    fn finish(&mut self, message: String, report: &mut ServiceReport) {
        self.phase = Phase::Finished;
        self.running_job = None;
        self.queued = 0;
        report.finished.push((self.id, message.clone()));
        let ran = self.admitted.iter().filter(|job| job.ran).count();
        if ran > 0 && self.cancel == Cancel::None && self.admitted.iter().all(|job| job.done) {
            report.records.push(CatchUpRecord {
                finished_ms: super::state::now_secs().saturating_mul(1000),
                ran,
                succeeded: self
                    .admitted
                    .iter()
                    .filter(|job| job.ran && matches!(job.outcome, Some(AttemptOutcome::Success)))
                    .count(),
                failed: self
                    .admitted
                    .iter()
                    .filter(|job| matches!(job.outcome, Some(AttemptOutcome::Failed(_))))
                    .count(),
                message: message.clone(),
            });
        }
        self.message = Some(message);
    }

    fn status(&self) -> CatchUpStatus {
        CatchUpStatus {
            finished: !self.is_open(),
            running_job: self.running_job.clone(),
            queued: self.queued,
            message: self.message.clone(),
            admitted: self.admitted.len(),
            skipped: self.skipped.clone(),
            failed: self
                .admitted
                .iter()
                .filter(|job| matches!(job.outcome, Some(AttemptOutcome::Failed(_))))
                .count(),
            retry_suggested: self.admitted.iter().any(|job| {
                matches!(job.outcome.as_ref(),
                Some(AttemptOutcome::Failed(error)) if !error.kind.needs_user())
            }),
        }
    }
}

pub(super) struct CatchUpBook {
    next_id: u64,
    runs: VecDeque<Run>,
}

impl CatchUpBook {
    pub(super) const fn new() -> Self {
        Self {
            next_id: 1,
            runs: VecDeque::new(),
        }
    }

    pub(super) fn request(&mut self) -> Result<u64, String> {
        if self.runs.iter().filter(|run| run.is_open()).count() >= MAX_OPEN_RUNS {
            return Err("Zu viele offene Nachhol-Läufe".into());
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        self.runs.push_back(Run {
            id,
            phase: Phase::Requested,
            cancel: Cancel::None,
            admitted: Vec::new(),
            skipped: Vec::new(),
            message: None,
            running_job: None,
            queued: 0,
        });
        Ok(id)
    }

    pub(super) fn status(&self, id: u64) -> Option<CatchUpStatus> {
        self.runs.iter().find(|run| run.id == id).map(Run::status)
    }

    /// A run that has not started ends at once; a running run cancels its
    /// remaining jobs at the next servicing pass.
    pub(super) fn cancel(&mut self, id: u64) -> ServiceReport {
        let mut report = ServiceReport::default();
        if let Some(run) = self.runs.iter_mut().find(|run| run.id == id) {
            match run.phase {
                Phase::Requested => run.finish("Abgebrochen".into(), &mut report),
                Phase::Running if run.cancel == Cancel::None => run.cancel = Cancel::Requested,
                Phase::Running | Phase::Finished => {}
            }
        }
        self.trim();
        report
    }

    pub(super) fn has_open_runs(&self) -> bool {
        self.runs.iter().any(Run::is_open)
    }

    pub(super) fn has_requested_runs(&self) -> bool {
        self.runs.iter().any(|run| run.phase == Phase::Requested)
    }

    /// Mark admitted jobs done. The supervisor holds an id at most once, so a
    /// completion ends the oldest open run-owned admission of that id and
    /// every admission that only awaited it (several runs may wait on it).
    pub(super) fn observe_completed(&mut self, completed: &[String], queue: &mut dyn CatchUpQueue) {
        for id in completed {
            let (outcome, ran) = queue.completion(id);
            let mut owner_seen = false;
            for job in self
                .runs
                .iter_mut()
                .filter(|run| run.phase == Phase::Running)
                .flat_map(|run| run.admitted.iter_mut())
                .filter(|job| !job.done && &job.id == id)
            {
                if job.owned {
                    if owner_seen {
                        continue;
                    }
                    owner_seen = true;
                }
                job.done = true;
                job.outcome = Some(outcome.clone());
                job.ran = ran;
            }
        }
    }

    /// End every open run with `message` (worker stopping).
    pub(super) fn finish_all(&mut self, message: &str) -> ServiceReport {
        let mut report = ServiceReport::default();
        for run in self.runs.iter_mut().filter(|run| run.is_open()) {
            run.finish(message.to_string(), &mut report);
        }
        self.trim();
        report
    }

    /// One pass in the worker loop. `jobs` is the saved job list, loaded by
    /// the caller when a requested run may start.
    pub(super) fn service(
        &mut self,
        queue: &mut dyn CatchUpQueue,
        gate: &CatchUpGate,
        jobs: Option<&Result<Vec<SyncJob>, String>>,
        now: i64,
    ) -> ServiceReport {
        let mut report = ServiceReport::default();
        let completed = queue.take_completed();
        self.observe_completed(&completed, queue);
        if let CatchUpGate::Closed(reason) = gate {
            for run in self.runs.iter_mut().filter(|run| run.is_open()) {
                queue.cancel_jobs(&run.undone_ids());
                run.finish(reason.clone(), &mut report);
            }
        } else {
            for run in self
                .runs
                .iter_mut()
                .filter(|run| run.phase == Phase::Requested)
            {
                start_run(run, queue, jobs, now, &mut report);
            }
            for run in self
                .runs
                .iter_mut()
                .filter(|run| run.phase == Phase::Running && run.cancel == Cancel::Requested)
            {
                queue.cancel_jobs(&run.undone_ids());
                // Jobs the regular schedule owns keep running; the run only
                // stops waiting for them.
                for job in run.admitted.iter_mut().filter(|job| !job.owned) {
                    job.done = true;
                }
                run.cancel = Cancel::Applied;
                run.message = Some("Abgebrochen".into());
            }
        }
        let completed = queue.take_completed();
        self.observe_completed(&completed, queue);
        let active: Vec<String> = queue.active_ids().into_iter().map(str::to_string).collect();
        for run in self
            .runs
            .iter_mut()
            .filter(|run| run.phase == Phase::Running)
        {
            update_progress(run, &active, &mut report);
        }
        self.trim();
        report
    }

    fn trim(&mut self) {
        let mut finished = self.runs.iter().filter(|run| !run.is_open()).count();
        self.runs.retain(|run| {
            if run.is_open() || finished <= MAX_FINISHED_RUNS {
                return true;
            }
            finished -= 1;
            false
        });
    }
}

#[cfg(test)]
#[path = "catch_up_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catch_up_outcome_tests.rs"]
mod outcome_tests;
