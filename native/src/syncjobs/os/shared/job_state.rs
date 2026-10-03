//! Runtime state of a sync job (RV1, contract V4): one JSON file per job,
//! `<sync data>/job-state/<id>.json`, apart from the configuration in
//! `jobs/<id>.conf`. Writers (background worker, desktop window, Android
//! facade, possibly in different processes) change it only through the
//! locked read-modify-write of `job_state_store`, so no update is lost.
//! Readers load it without the lock (the file is replaced atomically).
//!
//! Times are Unix seconds. Fields and enum values a newer version adds are
//! tolerated (unknown fields are ignored, unknown values read as `Other`);
//! fields an older file lacks take their defaults. Schedules count from
//! `last_success`; `SyncJob::last_run` only seeds a missing state.

use serde::{Deserialize, Serialize};

use super::results::JobResult;

/// Current format of the state file.
pub const JOB_STATE_VERSION: u32 = 1;
/// A run mark not renewed for this long belongs to a run that ended without
/// clearing it (crash, killed process) and counts as not running.
pub const RUN_MARK_STALE_SECS: i64 = 180;
/// Consecutive failures from which a job has a failure series (shown on the
/// job line, notified throttled).
pub const FAILURE_SERIES_MIN: u32 = 3;

/// Everything the scheduler, the job lists and the notifications need about
/// one job's runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobState {
    /// Format of the file (`JOB_STATE_VERSION`).
    pub version: u32,
    /// Start of the last attempt, also when it failed before syncing.
    pub last_attempt: Option<i64>,
    /// Who started the last attempt and why.
    pub last_runner: Option<Runner>,
    pub last_cause: Option<RunCause>,
    /// End of the last successful run; interval and calendar schedules count
    /// from here.
    pub last_success: Option<i64>,
    /// Failed attempts since the last success; cancellations and blocks do
    /// not count.
    pub consecutive_failures: u32,
    /// Error of the latest failed attempt; a success clears it.
    pub last_error: Option<JobError>,
    /// The run stopped for safety and waits for the user (no automatic
    /// retry); a success clears it.
    pub blocked: Option<Blocked>,
    /// A trigger no finished run has covered yet; it survives cancellation,
    /// pause and restart.
    pub pending_trigger: Option<PendingTrigger>,
    /// Earliest automatic retry after a failure; `None` = none planned.
    pub retry_at: Option<i64>,
    /// Counts and note of the latest finished run (the former `results.tsv`
    /// row).
    pub last_result: Option<JobResult>,
    /// How changes are detected now (real-time jobs; written by the worker).
    pub watch: Option<WatchStatus>,
    /// The job runs right now (see `running_now`).
    pub running: Option<RunMark>,
    /// Last complete verification run of the watched side(s).
    pub last_verify: Option<i64>,
    /// Host change cursor (`watch::host_cursor`, Android MediaStore) seen at
    /// `last_verify`.
    pub verify_cursor: Option<String>,
    /// Worker bookkeeping: the last volume arrival handled for an on-connect
    /// job.
    pub last_connect: Option<ConnectMark>,
    /// Worker bookkeeping: the last problem notification (throttle).
    pub notified: Option<Notified>,
    /// Not stored: why the stored state could not be read (the file was set
    /// aside and this state starts empty).
    #[serde(skip)]
    pub load_error: Option<String>,
}

impl JobState {
    /// The run mark while its runner still renews it.
    pub fn running_now(&self, now: i64) -> Option<&RunMark> {
        self.running
            .as_ref()
            .filter(|mark| now.saturating_sub(mark.alive) <= RUN_MARK_STALE_SECS)
    }

    /// The problem the job line shows and the worker notifies (throttled).
    pub fn problem(&self) -> Option<ProblemKind> {
        if self.blocked.is_some() {
            return Some(ProblemKind::Blocked);
        }
        let error = self.last_error.as_ref()?;
        if error.kind.needs_user() {
            Some(ProblemKind::NeedsAction)
        } else if self.consecutive_failures >= FAILURE_SERIES_MIN {
            Some(ProblemKind::FailureSeries)
        } else {
            None
        }
    }
}

/// Why an attempt failed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobError {
    pub kind: FailureKind,
    /// German text for the job line and notifications.
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The configuration is invalid or its file unreadable; no automatic
    /// retry until it changes.
    Config,
    /// A side was not reachable (host offline, name not resolved, peer or
    /// drive absent); retried with backoff.
    Unreachable,
    /// Login refused or credentials missing; no automatic retry until the
    /// user acts.
    Auth,
    /// Local access is missing (Android all-files access, permission on the
    /// root); no automatic retry until the user acts.
    Access,
    /// The before or after command failed; retried with backoff.
    Hook,
    /// The run ended with file errors, which the next run repeats; retried
    /// with backoff.
    Run,
    /// The target is full, over quota or read-only; retried with a long
    /// backoff.
    TargetFull,
    /// Unexpected failure (panic, internal error); retried with backoff.
    Internal,
    /// Written by a newer version.
    #[default]
    #[serde(other)]
    Other,
}

impl FailureKind {
    /// Automatic retries cannot help until the user acts.
    pub fn needs_user(self) -> bool {
        matches!(
            self,
            FailureKind::Config | FailureKind::Auth | FailureKind::Access
        )
    }
}

/// A safety stop of the engine that waits for the user.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Blocked {
    pub kind: BlockKind,
    /// German explanation from the engine, e.g. "120 von 200 Dateien in B
    /// würden gelöscht".
    pub detail: String,
    /// When the stop first happened.
    pub since: i64,
    /// The user confirmed exactly this stop ("Trotzdem ausführen"): the next
    /// run may pass it once.
    pub confirmed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockKind {
    /// The run would delete at least the minimum number and the percentage of
    /// files of one side the job allows.
    MassDelete {
        side: JobSide,
        deletions: u64,
        total: u64,
    },
    /// The run would delete more files than the job's absolute limit.
    DeleteLimit { deletions: u64, limit: u64 },
    /// A side that had `previous` entries at the last run is empty (drive not
    /// mounted?).
    SideEmpty {
        side: JobSide,
        #[serde(default)]
        previous: u64,
    },
    /// The replica marker of a side that had one is missing (other or
    /// unmounted drive?).
    ReplicaMissing { side: JobSide },
    /// Written by a newer version, or a stop without details.
    #[default]
    #[serde(other)]
    Other,
}

/// Side of a job: `A` = `SyncJob::source`, `B` = `SyncJob::target`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobSide {
    #[default]
    A,
    B,
}

/// A trigger that is still outstanding.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PendingTrigger {
    pub kind: PendingKind,
    /// First occurrence no finished run has covered yet.
    pub since: i64,
    /// On-connect: identity of the arrived volume the job waits for.
    pub volume: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingKind {
    /// A change was seen (events, poll or host signal).
    Change,
    /// A matching volume arrived.
    Connect,
    /// The startup pass of this logon or boot.
    Startup,
    /// A verification run is due.
    Verify,
    /// The user confirmed a block; run once with that stop allowed.
    Confirmed,
    /// Written by a newer version.
    #[default]
    #[serde(other)]
    Other,
}

/// How a real-time job detects changes right now.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchStatus {
    pub detection: ChangeDetection,
    /// Since when this applies.
    pub since: i64,
    /// Why events are missing or incomplete (German), e.g.
    /// "Überwachungs-Limit erreicht".
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ChangeDetection {
    /// Watches are being set up.
    Starting,
    /// Operating-system events (plus verification runs).
    Events,
    /// Events plus a poll every `poll_secs` (network file systems, partial
    /// coverage).
    EventsAndPoll { poll_secs: u64 },
    /// Poll only, every `poll_secs` (remote side, watch limit, unsupported).
    Poll { poll_secs: u64 },
    /// Written by a newer version.
    #[default]
    #[serde(other)]
    Other,
}

/// Marks a run in progress so other processes can show it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunMark {
    pub runner: Runner,
    pub cause: RunCause,
    pub started: i64,
    /// Renewed about every 30 s while the run lives.
    pub alive: i64,
    /// The runner has seen no progress since then (hang detection).
    pub stalled_since: Option<i64>,
}

/// Who runs a job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runner {
    /// The background worker (desktop process or embedded Android thread).
    Daemon,
    /// The desktop window ("Jetzt").
    Desktop,
    /// The Android facade (`sync.run`).
    Android,
    /// The terminal (`se`).
    Cli,
    /// Unknown: reported through the legacy `record_result`, or written by a
    /// newer version.
    #[default]
    #[serde(other)]
    Other,
}

/// Why a run started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCause {
    Manual,
    Interval,
    Calendar,
    /// Real-time change (events or host signal).
    Change,
    /// Real-time poll found a change.
    Poll,
    /// Verification run (start of the worker, hourly, daily target check).
    Verify,
    Startup,
    Connect,
    /// Retry after a failure.
    Retry,
    /// Catch-up run of the Android host.
    CatchUp,
    /// The user confirmed a block.
    Confirmed,
    /// Written by a newer version.
    #[default]
    #[serde(other)]
    Other,
}

/// Worker bookkeeping of an on-connect job.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectMark {
    /// Identity of the volume (Windows volume serial or GUID, Linux UUID).
    pub volume: String,
    pub seen: i64,
    /// Boot or logon marker of the arrival.
    pub session: Option<String>,
}

/// Worker bookkeeping of the last problem notification.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notified {
    /// Identity of the notified problem (kind and cause).
    pub key: String,
    pub at: i64,
}

/// One finished attempt, reported by whoever ran it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptReport {
    pub runner: Runner,
    pub cause: RunCause,
    pub started: i64,
    pub finished: i64,
    pub outcome: AttemptOutcome,
    /// Counts and note of the run; `None` when it never started syncing.
    pub result: Option<JobResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// The run completed; conflicts and protected omissions are allowed.
    Success,
    /// It could not start or ended with errors.
    Failed(JobError),
    /// Stopped by the user, a pause, a worker stop or the host; nothing is
    /// counted and outstanding triggers stay.
    Cancelled,
    /// Safety stop of the engine; waits for the user.
    Blocked(Blocked),
}

/// A problem worth a system notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProblemNotice {
    pub job_id: String,
    pub job_name: String,
    pub kind: ProblemKind,
    /// Short German title, e.g. "Sync „Fotos“ blockiert".
    pub title: String,
    /// German text: what happened and what to do.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProblemKind {
    /// A safety stop waits for the user.
    Blocked,
    /// Automatic retries stopped: login, configuration or access must be
    /// fixed.
    NeedsAction,
    /// Repeated failures (`FAILURE_SERIES_MIN` or more).
    FailureSeries,
}
