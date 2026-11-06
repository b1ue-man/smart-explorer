//! When the background worker starts a job (RV1, FS9): a pure policy over the
//! job's configuration and its job state. Schedules count from the last
//! success (never from a failed or cancelled attempt); failed automatic runs
//! are retried with the state's backoff, and failures only the user can fix
//! (login, configuration, access) wait for user action; a safety
//! stop waits for its confirmation; outstanding triggers (startup, connect,
//! confirmation, verification) survive restarts. Real-time changes are timed
//! by the real-time watcher (debounce and maximum wait), not here.

use chrono::{TimeZone, Timelike};

use crate::syncjobs::{JobState, PendingKind, RunCause, Runner, SyncJob, Trigger};

pub(super) const NEEDS_USER_PROBE_SECS: i64 = 86_400; // legacy test constant; no auth probes
const MINUTES_PER_DAY: i32 = 24 * 60;
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

/// New evidence allows one retry of a failure only the user could fix.
pub(super) fn recheck_pending(state: &JobState) -> bool {
    state
        .recheck
        .as_ref()
        .is_some_and(|recheck| recheck.pending)
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
            // No automatic login attempts (account lockout, fail2ban); only
            // new evidence (changed credentials, edit, sibling success).
            return recheck_pending(state).then_some(RunCause::Retry);
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

/// When the job is next due (at the earliest `now`), for the host's wake
/// alarm; `None` when only events start it. A job with an active-hours
/// window is never due outside of it (`due_now`): a time that falls outside
/// the window moves to the next start of the window, so the alarm neither
/// fires "now" again and again nor wakes the device for a start that cannot
/// happen.
pub(super) fn next_due(job: &SyncJob, state: &JobState, now: i64) -> Option<i64> {
    next_due_unwindowed(job, state, now).map(|at| next_active_at(job, at))
}

/// First instant at or after `at` inside the job's active-hours window
/// (`at` itself without a window or inside it).
fn next_active_at(job: &SyncJob, at: i64) -> i64 {
    if job.active_now(at) {
        return at;
    }
    // Outside the window, so its start differs from the current local
    // minute of day (a window with `from == to` is always active).
    let current = chrono::Local
        .timestamp_opt(at, 0)
        .single()
        .map_or(0, |time| (time.hour() * 60 + time.minute()) as i32);
    let ahead = i64::from((job.active_from_min - current).rem_euclid(MINUTES_PER_DAY));
    let minute_start = at - at.rem_euclid(60);
    let mut start = minute_start.saturating_add(ahead * 60);
    // A daylight-saving change between `at` and the start shifts the local
    // minute: step back to the first active minute, or forward to the next.
    let mut steps = 0;
    while steps < MINUTES_PER_DAY && start - 60 > at && job.active_now(start - 60) {
        start -= 60;
        steps += 1;
    }
    while steps < 2 * MINUTES_PER_DAY && !job.active_now(start) {
        start = start.saturating_add(60);
        steps += 1;
    }
    start
}

fn next_due_unwindowed(job: &SyncJob, state: &JobState, now: i64) -> Option<i64> {
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
            return recheck_pending(state).then_some(now);
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

    fn local(day: u32, hour: u32, minute: u32) -> i64 {
        chrono::Local
            .with_ymd_and_hms(2026, 6, day, hour, minute, 0)
            .earliest()
            .expect("a June instant exists in every time zone")
            .timestamp()
    }

    fn confirmed_block() -> JobState {
        JobState {
            blocked: Some(Blocked {
                kind: BlockKind::Other,
                detail: String::new(),
                since: NOW,
                confirmed: true,
            }),
            ..JobState::default()
        }
    }

    #[test]
    fn battery_task_next_due_outside_the_active_window_is_the_next_window_start() {
        let mut windowed = job(Trigger::Interval);
        windowed.active_from_min = 9 * 60;
        windowed.active_to_min = 17 * 60;
        let state = confirmed_block();
        // Due now, but the window is closed: the alarm waits for 09:00.
        let evening = local(10, 20, 30);
        assert_eq!(due_now(&windowed, &state, evening, None), None);
        assert_eq!(next_due(&windowed, &state, evening), Some(local(11, 9, 0)));
        let night = local(10, 2, 0);
        assert_eq!(next_due(&windowed, &state, night), Some(local(10, 9, 0)));
        // Inside the window the job is due now, as before.
        let noon = local(10, 12, 0);
        assert_eq!(next_due(&windowed, &state, noon), Some(noon));
        // A window that wraps past midnight starts in the evening.
        windowed.active_from_min = 22 * 60;
        windowed.active_to_min = 6 * 60;
        assert_eq!(next_due(&windowed, &state, noon), Some(local(10, 22, 0)));
        assert_eq!(
            next_due(&windowed, &state, local(10, 23, 0)),
            Some(local(10, 23, 0))
        );
        // No window (from == to): never moved.
        windowed.active_to_min = windowed.active_from_min;
        assert_eq!(next_due(&windowed, &state, noon), Some(noon));
    }

    #[test]
    fn battery_task_a_timer_time_outside_the_window_moves_to_the_window_start() {
        let mut windowed = job(Trigger::Interval);
        windowed.interval_min = 60;
        windowed.active_from_min = 9 * 60;
        windowed.active_to_min = 17 * 60;
        let now = local(10, 12, 0);
        let state = JobState {
            last_success: Some(now - 60),
            ..JobState::default()
        };
        // The next interval time (13:00) is inside the window: unchanged.
        assert_eq!(next_due(&windowed, &state, now), Some(now - 60 + 3_600));
        // The next interval time (20:30) is outside: it moves to 09:00.
        let late = JobState {
            last_success: Some(local(10, 19, 30)),
            ..JobState::default()
        };
        assert_eq!(
            next_due(&windowed, &late, local(10, 19, 40)),
            Some(local(11, 9, 0))
        );
        // Event-only triggers still have no clock time.
        windowed.trigger = Trigger::OnConnect;
        assert_eq!(next_due(&windowed, &JobState::default(), now), None);
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
