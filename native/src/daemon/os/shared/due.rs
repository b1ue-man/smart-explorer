//! When the background worker starts a job (RV1, FS9): a pure policy over the
//! job's configuration and its job state. Schedules count from the last
//! success (never from a failed or cancelled attempt); failed automatic runs
//! are retried with the state's backoff, and failures only the user can fix
//! (login, configuration, access) wait for user action; a safety
//! stop waits for its confirmation; outstanding triggers (startup, connect,
//! confirmation, verification) survive restarts. Real-time changes are timed
//! by the real-time watcher (debounce and maximum wait), not here.

use crate::syncjobs::{JobState, PendingKind, RunCause, Runner, SyncJob, Trigger};

pub(super) const NEEDS_USER_PROBE_SECS: i64 = 86_400; // legacy test constant; no auth probes
/// Plausible creation times encoded in job ids (2020 … 2100).
const CREATED_MIN: i64 = 1_577_836_800;
const CREATED_MAX: i64 = 4_102_444_800;

/// The time a timer schedule counts from: the last success, else the job's
/// creation (so a new calendar job does not run at once for an occurrence
/// before it existed), else the epoch.
pub(super) fn anchor(job: &SyncJob, state: &JobState, now: i64) -> i64 {
    state
        .last_success
        .filter(|at| *at <= now.saturating_add(86_400))
        .or_else(|| created_at(&job.id).filter(|at| *at <= now))
        .unwrap_or(0)
}

/// Creation time of ids made by `SyncJob::new` (hexadecimal nanoseconds).
fn created_at(id: &str) -> Option<i64> {
    let nanos = u128::from_str_radix(id, 16).ok()?;
    let secs = i64::try_from(nanos / 1_000_000_000).ok()?;
    (CREATED_MIN..=CREATED_MAX).contains(&secs).then_some(secs)
}

/// Whether the job runs on its own at all (not only "Jetzt").
fn automatic(job: &SyncJob) -> bool {
    job.enabled && job.trigger != Trigger::Manual
}

/// Why the job should start now (`None`: not now). `since` is the previous
/// evaluation of the timer schedules.
pub(super) fn due_now(
    job: &SyncJob,
    state: &JobState,
    now: i64,
    since: Option<i64>,
) -> Option<RunCause> {
    if !job.enabled || !job.active_now(now) {
        return None;
    }
    if state
        .running_now(now)
        .is_some_and(|mark| mark.runner != Runner::Daemon)
    {
        // The desktop window or the Android facade runs it right now.
        return None;
    }
    if let Some(block) = &state.blocked {
        let confirmed = block.confirmed
            || state
                .pending_trigger
                .as_ref()
                .is_some_and(|trigger| trigger.kind == PendingKind::Confirmed);
        return confirmed.then_some(RunCause::Confirmed);
    }
    if !automatic(job) {
        return None;
    }
    if state.consecutive_failures > 0 {
        if state
            .last_error
            .as_ref()
            .is_some_and(|error| error.kind.needs_user())
        {
            return None;
        }
        let retry = match state.retry_at {
            Some(at) => now >= at,
            None => false,
        };
        return retry.then_some(RunCause::Retry);
    }
    if let Some(trigger) = &state.pending_trigger {
        match trigger.kind {
            PendingKind::Startup => return Some(RunCause::Startup),
            PendingKind::Connect => return Some(RunCause::Connect),
            PendingKind::Confirmed => return Some(RunCause::Confirmed),
            PendingKind::Verify => return Some(RunCause::Verify),
            PendingKind::Other if job.trigger == Trigger::Calendar => {
                return Some(RunCause::Calendar)
            }
            PendingKind::Other if job.trigger == Trigger::Interval => {
                return Some(RunCause::Interval)
            }
            PendingKind::Change | PendingKind::Other => {}
        }
    }
    if job.due_at(anchor(job, state, now), now, since) {
        return Some(if job.trigger == Trigger::Calendar {
            RunCause::Calendar
        } else {
            RunCause::Interval
        });
    }
    None
}

/// When the job is next due by the clock (at the earliest `now`), for the
/// host's wake alarm; `None` when only events start it. The active-hours
/// window is not applied (the worker waits for it when the time comes).
pub(super) fn next_due(job: &SyncJob, state: &JobState, now: i64) -> Option<i64> {
    if !job.enabled {
        return None;
    }
    if let Some(block) = &state.blocked {
        return block.confirmed.then_some(now);
    }
    if !automatic(job) {
        return None;
    }
    if state.consecutive_failures > 0 {
        if state
            .last_error
            .as_ref()
            .is_some_and(|error| error.kind.needs_user())
        {
            return None;
        }
        return state.retry_at.map(|at| at.max(now));
    }
    if let Some(trigger) = &state.pending_trigger {
        if trigger.kind == PendingKind::Change {
            let delay = i64::try_from(job.rt_debounce_secs).unwrap_or(i64::MAX);
            return Some(trigger.since.saturating_add(delay).max(now));
        }
        if trigger.kind != PendingKind::Other
            || matches!(job.trigger, Trigger::Calendar | Trigger::Interval)
        {
            return Some(now);
        }
    }
    let timer = job.next_due_at(anchor(job, state, now), now);
    let verify = (job.trigger == Trigger::RealTime && job.verify_interval_secs > 0).then(|| {
        let interval = i64::try_from(job.verify_interval_secs).unwrap_or(i64::MAX);
        state
            .last_verify
            .map_or(now, |at| at.saturating_add(interval).max(now))
    });
    match (timer, verify) {
        (Some(timer), Some(verify)) => Some(timer.min(verify)),
        (timer, verify) => timer.or(verify),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syncjobs::{BlockKind, Blocked, FailureKind, JobError, PendingTrigger};

    const NOW: i64 = 1_800_000_000;

    fn job(trigger: Trigger) -> SyncJob {
        let mut job = SyncJob::new("x".into(), "/a".into(), "/b".into());
        job.id = "job".into();
        job.trigger = trigger;
        job.interval_min = 60;
        job
    }

    fn failed(kind: FailureKind) -> JobState {
        JobState {
            consecutive_failures: 2,
            last_attempt: Some(NOW - 600),
            last_error: Some(JobError {
                kind,
                message: String::new(),
            }),
            retry_at: (!kind.needs_user()).then_some(NOW + 300),
            ..JobState::default()
        }
    }

    #[test]
    fn review_task_schedules_count_from_the_last_success() {
        let job = job(Trigger::Interval);
        let state = JobState {
            last_success: Some(NOW - 3_000),
            last_attempt: Some(NOW - 10),
            ..JobState::default()
        };
        assert_eq!(due_now(&job, &state, NOW, None), None);
        let state = JobState {
            last_success: Some(NOW - 3_700),
            ..state
        };
        assert_eq!(due_now(&job, &state, NOW, None), Some(RunCause::Interval));
    }

    #[test]
    fn review_task_failures_retry_with_backoff_and_wait_for_the_user() {
        let job = job(Trigger::Interval);
        let transient = failed(FailureKind::Unreachable);
        assert_eq!(due_now(&job, &transient, NOW, None), None);
        assert_eq!(
            due_now(&job, &transient, NOW + 300, None),
            Some(RunCause::Retry)
        );
        assert_eq!(next_due(&job, &transient, NOW), Some(NOW + 300));

        let mut auth = failed(FailureKind::Auth);
        assert_eq!(due_now(&job, &auth, NOW + 3_600, None), None);
        assert_eq!(
            due_now(&job, &auth, NOW - 600 + NEEDS_USER_PROBE_SECS, None),
            None
        );
        assert_eq!(next_due(&job, &auth, NOW), None);
        // Older state files may still contain a probe deadline. Its presence
        // must not authorize another automatic authentication attempt.
        auth.retry_at = Some(NOW - 1);
        assert_eq!(due_now(&job, &auth, NOW, None), None);
        assert_eq!(next_due(&job, &auth, NOW), None);
        // Manual jobs are never retried automatically.
        assert_eq!(
            due_now(&self::job(Trigger::Manual), &transient, NOW + 300, None),
            None
        );
    }

    #[test]
    fn review_task_blocks_wait_for_confirmation() {
        let job = job(Trigger::Interval);
        let mut state = JobState {
            blocked: Some(Blocked {
                kind: BlockKind::Other,
                detail: String::new(),
                since: NOW,
                confirmed: false,
            }),
            ..JobState::default()
        };
        assert_eq!(due_now(&job, &state, NOW + 86_400, None), None);
        if let Some(block) = state.blocked.as_mut() {
            block.confirmed = true;
        }
        assert_eq!(due_now(&job, &state, NOW, None), Some(RunCause::Confirmed));
    }

    #[test]
    fn review_task_outstanding_triggers_run_after_a_restart() {
        let job = job(Trigger::OnConnect);
        let state = JobState {
            pending_trigger: Some(PendingTrigger {
                kind: PendingKind::Connect,
                since: NOW - 60,
                volume: Some("USB-1".into()),
            }),
            ..JobState::default()
        };
        assert_eq!(due_now(&job, &state, NOW, None), Some(RunCause::Connect));
        let mut disabled = job.clone();
        disabled.enabled = false;
        assert_eq!(due_now(&disabled, &state, NOW, None), None);
    }

    #[test]
    fn review_task_job_ids_give_the_creation_anchor() {
        let job = SyncJob::new("x".into(), "/a".into(), "/b".into());
        let created = created_at(&job.id).expect("ids encode the creation time");
        assert!((created - crate::syncjobs::JOB_STATE_VERSION as i64).abs() > 0);
        assert_eq!(created_at("not-hex"), None);
        assert_eq!(created_at("1"), None);
    }
}
