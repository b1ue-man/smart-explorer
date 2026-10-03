//! Persistent sync jobs - the saved "sync setups" (source, target, direction,
//! conflict mode, retention, schedule, hidden/ignore). Stored as one
//! human-readable `key=value` file per job under `<appdata>/smart_explorer/
//! sync/jobs/<id>.conf`, shared by the Sync UI, the split-view "sync these
//! folders" action, and the background worker - so a setup survives a restart
//! and every surface agrees.
//!
//! The `key=value` format is deliberately forward-compatible: unknown keys are
//! ignored and missing keys fall back to defaults, so new options can be added
//! over time without breaking old files or older builds. The previous single
//! positional `jobs.tsv` is auto-imported once on first load.

#[path = "os/shared/baseline_migration.rs"]
mod baseline_migration;
#[path = "os/shared/editor.rs"]
pub mod editor;
#[path = "os/shared/job_state.rs"]
mod job_state;
#[path = "os/shared/job_state_classify.rs"]
mod job_state_classify;
#[cfg(any(target_os = "linux", target_os = "android"))]
#[path = "os/linux_os/job_state_lock.rs"]
mod job_state_lock;
#[cfg(windows)]
#[path = "os/windows/job_state_lock.rs"]
mod job_state_lock;
#[path = "os/shared/job_state_policy.rs"]
mod job_state_policy;
#[path = "os/shared/job_state_store.rs"]
mod job_state_store;
#[path = "os/shared/migration.rs"]
mod migration;
pub(crate) use baseline_migration::legacy_baseline_pending;
#[path = "os/shared/recorded_options.rs"]
mod recorded_options;
pub(crate) use recorded_options::recorded_options;
#[path = "os/shared/persistence.rs"]
mod persistence;
#[path = "os/shared/persistence_codec.rs"]
mod persistence_codec;
#[cfg(any(target_os = "linux", target_os = "android"))]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[path = "os/shared/results.rs"]
mod results;
#[path = "core/schedule.rs"]
mod schedule;
#[path = "core/types.rs"]
mod types;
#[path = "core/validation.rs"]
mod validation;

pub use job_state::{
    AttemptOutcome, AttemptReport, BlockKind, Blocked, ChangeDetection, ConnectMark, FailureKind,
    JobError, JobSide, JobState, Notified, PendingKind, PendingTrigger, ProblemKind, ProblemNotice,
    RunCause, RunMark, Runner, WatchStatus, FAILURE_SERIES_MIN, JOB_STATE_VERSION,
    RUN_MARK_STALE_SECS,
};
pub use job_state_classify::{block_confirmation, block_kind, classify_run};
pub use job_state_store::{
    confirm_block, load_job_state, load_job_states, record_attempt, remove_job_state,
    take_problem_notices, update_job_state,
};
#[allow(unused_imports)]
pub use persistence::{
    jobs_dir, jobs_path, load, load_report, remove, upsert, BrokenJob, JobLoadReport,
};
pub use results::{load_results, mark_run, record_result, JobResult};
#[allow(unused_imports)]
pub use schedule::within_window;
pub use types::{SyncJob, Trigger, CURRENT_CONFIG_VERSION};

pub(crate) use job_state_lock::StateLock as RuntimeFileLock;

pub use job_state_classify::classify_failure;
