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

use crate::syncjobs::{SyncJob, Trigger};

use super::job_supervisor::{EnqueueStatus, JobSupervisor};

/// Finished runs kept for status queries; open runs are never dropped.
const MAX_FINISHED_RUNS: usize = 16;
/// Upper bound for simultaneously open runs (each is serviced within seconds).
const MAX_OPEN_RUNS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUpSkip {
    pub job_id: String,
    pub job_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUpStatus {
    pub finished: bool,
    /// Display name of this run's job that is running right now.
    pub running_job: Option<String>,
    /// This run's admitted jobs still waiting in the supervisor queue.
    pub queued: usize,
    pub message: Option<String>,
    /// Jobs admitted to or awaited by this run (done = admitted - queued -
    /// running).
    pub admitted: usize,
    /// Selected jobs the supervisor refused, with the reason.
    pub skipped: Vec<CatchUpSkip>,
}

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
}

impl CatchUpQueue for JobSupervisor {
    fn admit(&mut self, job: &SyncJob) -> Result<EnqueueStatus, String> {
        self.enqueue(job)
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
    pub(super) fn observe_completed(&mut self, completed: &[String]) {
        for id in completed {
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
        self.observe_completed(&queue.take_completed());
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
        self.observe_completed(&queue.take_completed());
        let active = queue.active_job_id().map(str::to_string);
        for run in self
            .runs
            .iter_mut()
            .filter(|run| run.phase == Phase::Running)
        {
            update_progress(run, active.as_deref(), &mut report);
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

fn start_run(
    run: &mut Run,
    queue: &mut dyn CatchUpQueue,
    jobs: Option<&Result<Vec<SyncJob>, String>>,
    now: i64,
    report: &mut ServiceReport,
) {
    let jobs = match jobs {
        Some(Ok(jobs)) => jobs,
        Some(Err(error)) => {
            run.finish(
                format!("Sync-Jobs konnten nicht geladen werden: {error}"),
                report,
            );
            return;
        }
        // Requested after the caller decided not to load the list: the run
        // stays requested, and the next pass loads the list for it.
        None => return,
    };
    for job in select_catch_up_jobs(jobs, now) {
        let name = display_name(job);
        let admission = match queue.admit(job) {
            Ok(EnqueueStatus::Started | EnqueueStatus::Queued) => Ok(true),
            // The regular schedule already holds it (a cold wake enqueues the
            // due jobs first): wait for it, never cancel it.
            Ok(EnqueueStatus::AlreadyScheduled) => Ok(false),
            Ok(EnqueueStatus::RecentlyAttempted) => Err("kürzlich versucht".to_string()),
            Err(error) => Err(error),
        };
        match admission {
            Ok(owned) => run.admitted.push(Admitted {
                id: job.id.clone(),
                name,
                done: false,
                owned,
            }),
            Err(reason) => run.skipped.push(CatchUpSkip {
                job_id: job.id.clone(),
                job_name: name,
                reason,
            }),
        }
    }
    run.phase = Phase::Running;
    report
        .started
        .push((run.id, run.admitted.len(), run.skipped.len()));
}

fn update_progress(run: &mut Run, active: Option<&str>, report: &mut ServiceReport) {
    let running = run
        .admitted
        .iter()
        .find(|job| !job.done && Some(job.id.as_str()) == active);
    run.running_job = running.map(|job| job.name.clone());
    let undone = run.admitted.iter().filter(|job| !job.done).count();
    run.queued = undone - usize::from(running.is_some());
    if undone == 0 {
        let message = run
            .message
            .clone()
            .unwrap_or_else(|| summary(run.admitted.len(), run.skipped.len()));
        run.finish(message, report);
    }
}

fn summary(admitted: usize, skipped: usize) -> String {
    let done = match admitted {
        0 if skipped == 0 => return "Keine fälligen Jobs".into(),
        0 => "Kein Job gestartet".to_string(),
        1 => "1 Job ausgeführt".to_string(),
        count => format!("{count} Jobs ausgeführt"),
    };
    if skipped == 0 {
        done
    } else {
        format!("{done}, {skipped} übersprungen")
    }
}

fn display_name(job: &SyncJob) -> String {
    if job.name.trim().is_empty() {
        job.id.clone()
    } else {
        job.name.clone()
    }
}

#[cfg(test)]
#[path = "catch_up_tests.rs"]
mod tests;
