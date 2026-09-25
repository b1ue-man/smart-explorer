use std::collections::{HashSet, VecDeque};

use super::{select_catch_up_jobs, CatchUpBook, CatchUpGate, CatchUpQueue, MAX_FINISHED_RUNS};
use crate::daemon::job_supervisor::EnqueueStatus;
use crate::syncjobs::{SyncJob, Trigger};

const NOW: i64 = 1_750_000_000;

fn job(id: &str, trigger: Trigger) -> SyncJob {
    let mut job = SyncJob::new(format!("Job {id}"), "/a".into(), "/b".into());
    job.id = id.to_string();
    job.trigger = trigger;
    job
}

/// Minutes after local midnight at `NOW`, for an active-hours window that
/// excludes it in any time zone.
fn local_minute(now: i64) -> i32 {
    use chrono::{Local, TimeZone, Timelike};
    let time = Local.timestamp_opt(now, 0).single().unwrap();
    (time.hour() * 60 + time.minute()) as i32
}

#[derive(Default)]
struct FakeQueue {
    active: Option<String>,
    pending: VecDeque<String>,
    recent: HashSet<String>,
    completed: Vec<String>,
    canceled: Vec<HashSet<String>>,
}

impl FakeQueue {
    fn scheduled(&self, id: &str) -> bool {
        self.active.as_deref() == Some(id) || self.pending.iter().any(|queued| queued == id)
    }

    /// The active job returns; the next queued one starts.
    fn finish_active(&mut self) {
        if let Some(id) = self.active.take() {
            self.completed.push(id);
        }
        self.active = self.pending.pop_front();
    }
}

impl CatchUpQueue for FakeQueue {
    fn admit(&mut self, job: &SyncJob) -> Result<EnqueueStatus, String> {
        if self.scheduled(&job.id) {
            return Ok(EnqueueStatus::AlreadyScheduled);
        }
        if self.recent.contains(&job.id) {
            return Ok(EnqueueStatus::RecentlyAttempted);
        }
        if job.id == "broken" {
            return Err("job spawn failed for 'broken'".into());
        }
        if self.active.is_none() {
            self.active = Some(job.id.clone());
            Ok(EnqueueStatus::Started)
        } else {
            self.pending.push_back(job.id.clone());
            Ok(EnqueueStatus::Queued)
        }
    }

    fn cancel_jobs(&mut self, ids: &HashSet<String>) {
        if ids.is_empty() {
            return;
        }
        self.canceled.push(ids.clone());
        let completed = &mut self.completed;
        self.pending.retain(|id| {
            if ids.contains(id) {
                completed.push(id.clone());
                false
            } else {
                true
            }
        });
    }

    fn take_completed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.completed)
    }

    fn active_job_id(&self) -> Option<&str> {
        self.active.as_deref()
    }
}

fn service(book: &mut CatchUpBook, queue: &mut FakeQueue, jobs: Vec<SyncJob>) {
    book.service(queue, &CatchUpGate::Open, Some(&Ok(jobs)), NOW);
}

#[test]
fn android_task_catch_up_selects_due_timers_missed_calendar_and_realtime_once() {
    let mut interval_due = job("interval-due", Trigger::Interval);
    interval_due.interval_min = 15;
    interval_due.last_run = NOW - 16 * 60;
    let mut interval_recent = job("interval-recent", Trigger::Interval);
    interval_recent.interval_min = 15;
    interval_recent.last_run = NOW - 60;

    // Daily at midnight, missed since the last run, no own catch-up.
    let mut calendar_missed = job("calendar-missed", Trigger::Calendar);
    calendar_missed.cal_time_min = 0;
    calendar_missed.cal_weekdays = 0;
    calendar_missed.catch_up = false;
    calendar_missed.last_run = NOW - 3 * 86_400;
    let mut calendar_done = calendar_missed.clone();
    calendar_done.id = "calendar-done".into();
    calendar_done.last_run = NOW;

    let realtime = job("realtime", Trigger::RealTime);
    let mut realtime_off = job("realtime-off", Trigger::RealTime);
    realtime_off.enabled = false;
    let mut realtime_inactive = job("realtime-inactive", Trigger::RealTime);
    let minute = local_minute(NOW);
    realtime_inactive.active_from_min = (minute + 1) % 1440;
    realtime_inactive.active_to_min = (minute + 2) % 1440;

    let jobs = vec![
        interval_due,
        interval_recent,
        calendar_missed,
        calendar_done,
        realtime,
        realtime_off,
        realtime_inactive,
        job("manual", Trigger::Manual),
        job("startup", Trigger::OnStartup),
        job("connect", Trigger::OnConnect),
    ];
    let selected: Vec<&str> = select_catch_up_jobs(&jobs, NOW)
        .into_iter()
        .map(|job| job.id.as_str())
        .collect();
    assert_eq!(selected, ["interval-due", "calendar-missed", "realtime"]);
}

#[test]
fn android_task_catch_up_run_finishes_when_admitted_jobs_are_done() {
    let mut book = CatchUpBook::new();
    let mut queue = FakeQueue::default();
    let id = book.request().unwrap();
    assert!(!book.status(id).unwrap().finished);

    let jobs = vec![job("a", Trigger::RealTime), job("b", Trigger::RealTime)];
    service(&mut book, &mut queue, jobs.clone());
    let status = book.status(id).unwrap();
    assert!(!status.finished);
    assert_eq!(status.admitted, 2);
    assert_eq!(status.running_job.as_deref(), Some("Job a"));
    assert_eq!(status.queued, 1);

    queue.finish_active();
    service(&mut book, &mut queue, jobs.clone());
    let status = book.status(id).unwrap();
    assert_eq!(status.running_job.as_deref(), Some("Job b"));
    assert_eq!(status.queued, 0);

    queue.finish_active();
    service(&mut book, &mut queue, jobs);
    let status = book.status(id).unwrap();
    assert!(status.finished);
    assert_eq!(status.message.as_deref(), Some("2 Jobs ausgeführt"));
    assert!(!book.has_open_runs());
}

#[test]
fn android_task_catch_up_lists_supervisor_rejections_with_reason() {
    let mut book = CatchUpBook::new();
    let mut queue = FakeQueue {
        active: Some("scheduled".into()),
        recent: ["recent".to_string()].into_iter().collect(),
        ..FakeQueue::default()
    };
    let id = book.request().unwrap();
    let jobs = vec![
        job("scheduled", Trigger::RealTime),
        job("recent", Trigger::RealTime),
        job("broken", Trigger::RealTime),
    ];
    service(&mut book, &mut queue, jobs);

    let status = book.status(id).unwrap();
    assert!(status.finished);
    assert_eq!(status.admitted, 0);
    let reasons: Vec<(&str, &str)> = status
        .skipped
        .iter()
        .map(|skip| (skip.job_id.as_str(), skip.reason.as_str()))
        .collect();
    assert_eq!(
        reasons,
        [
            ("scheduled", "bereits geplant oder läuft"),
            ("recent", "kürzlich versucht"),
            ("broken", "job spawn failed for 'broken'"),
        ]
    );
    assert_eq!(
        status.message.as_deref(),
        Some("Kein Job gestartet, 3 übersprungen")
    );
}

#[test]
fn android_task_catch_up_cancel_touches_only_this_runs_jobs() {
    let mut book = CatchUpBook::new();
    // A regular scheduled job is running before the catch-up run starts.
    let mut queue = FakeQueue {
        active: Some("tick".into()),
        ..FakeQueue::default()
    };
    let id = book.request().unwrap();
    let jobs = vec![job("a", Trigger::RealTime), job("b", Trigger::RealTime)];
    service(&mut book, &mut queue, jobs.clone());
    assert_eq!(book.status(id).unwrap().queued, 2);

    book.cancel(id);
    service(&mut book, &mut queue, jobs);
    let expected: HashSet<String> = ["a", "b"].map(String::from).into_iter().collect();
    assert_eq!(queue.canceled, vec![expected]);
    assert_eq!(queue.active.as_deref(), Some("tick"));
    let status = book.status(id).unwrap();
    assert!(status.finished);
    assert_eq!(status.message.as_deref(), Some("Abgebrochen"));

    let pending = book.request().unwrap();
    book.cancel(pending);
    let status = book.status(pending).unwrap();
    assert!(status.finished);
    assert_eq!(status.admitted, 0);
    assert_eq!(status.message.as_deref(), Some("Abgebrochen"));
}

#[test]
fn android_task_catch_up_closed_gate_ends_open_runs_with_reason() {
    let mut book = CatchUpBook::new();
    let mut queue = FakeQueue::default();
    let running = book.request().unwrap();
    service(&mut book, &mut queue, vec![job("a", Trigger::RealTime)]);
    let requested = book.request().unwrap();

    let gate = CatchUpGate::Closed("Hintergrund-Sync ist pausiert".into());
    let report = book.service(&mut queue, &gate, None, NOW);
    assert_eq!(report.finished.len(), 2);
    for id in [running, requested] {
        let status = book.status(id).unwrap();
        assert!(status.finished);
        assert_eq!(
            status.message.as_deref(),
            Some("Hintergrund-Sync ist pausiert")
        );
    }

    let stopping = book.request().unwrap();
    book.finish_all("Background-Worker wurde beendet");
    assert!(book.status(stopping).unwrap().finished);
}

#[test]
fn android_task_catch_up_reports_load_failures_and_bounds_history() {
    let mut book = CatchUpBook::new();
    let mut queue = FakeQueue::default();
    let failed = book.request().unwrap();
    let error: Result<Vec<SyncJob>, String> = Err("gesperrt".into());
    book.service(&mut queue, &CatchUpGate::Open, Some(&error), NOW);
    assert_eq!(
        book.status(failed).unwrap().message.as_deref(),
        Some("Sync-Jobs konnten nicht geladen werden: gesperrt")
    );
    assert!(book.status(u64::MAX).is_none());

    let mut last = failed;
    for _ in 0..MAX_FINISHED_RUNS + 4 {
        last = book.request().unwrap();
        service(&mut book, &mut queue, Vec::new());
    }
    assert_eq!(
        book.status(last).unwrap().message.as_deref(),
        Some("Keine fälligen Jobs")
    );
    assert!(book.status(failed).is_none());
}
