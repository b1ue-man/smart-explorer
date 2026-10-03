//! Reading and writing the job state (RV1, contract V4).
//!
//! Every write is a read-modify-write under an exclusive per-job file lock
//! (`job-state/<id>.lock`, bounded wait) followed by an atomic replace of
//! `job-state/<id>.json`, so the worker, the desktop window and the Android
//! facade never lose each other's updates; readers need no lock. A missing
//! state is seeded from the legacy `results.tsv` row and `SyncJob::last_run`;
//! an unreadable one is set aside once (`<id>.json.corrupt`) and starts from
//! that seed with `load_error`.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::job_state::{
    AttemptReport, BlockKind, Blocked, JobState, Notified, ProblemNotice, JOB_STATE_VERSION,
};
use super::job_state_lock::StateLock;
use super::job_state_policy::{apply_attempt, build_notice, confirm, notice_due, problem_key};
use super::persistence::{
    app_data_dir, atomic_write, job_file, jobs_dir, load_job_file, read_regular_utf8, san_id,
};
use super::results::{legacy_result, JobResult};
use super::schedule::now_secs;
use super::types::SyncJob;

const STATE_DIR: &str = "job-state";
const STATE_EXTENSION: &str = "json";
const MAX_STATE_BYTES: u64 = 1024 * 1024;
/// A writer holds the lock only to read, change and replace one small file.
const LOCK_WAIT: Duration = Duration::from_secs(10);
/// A `last_run` further in the future than this comes from a wrong clock and
/// is not trusted as a schedule anchor.
const CLOCK_SLACK_SECS: i64 = 86_400;

fn state_dir() -> PathBuf {
    app_data_dir().join(STATE_DIR)
}

fn checked_id(id: &str) -> io::Result<&str> {
    if id.is_empty() || id != san_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sync job id is not a safe file name",
        ));
    }
    Ok(id)
}

fn state_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.{STATE_EXTENSION}"))
}

fn lock_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.lock"))
}

fn corrupt_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.{STATE_EXTENSION}.corrupt"))
}

pub(super) fn empty_state() -> JobState {
    JobState {
        version: JOB_STATE_VERSION,
        ..JobState::default()
    }
}

/// What a state file holds.
enum Stored {
    Present(JobState),
    Missing,
    /// Present but not a readable state (text for `load_error`).
    Invalid(String),
}

fn read_state(path: &Path) -> io::Result<Stored> {
    let body = match read_regular_utf8(path, MAX_STATE_BYTES, "sync job state") {
        Ok(body) => body,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Stored::Missing),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return Ok(Stored::Invalid(error.to_string()))
        }
        Err(error) => return Err(error),
    };
    Ok(match serde_json::from_str::<JobState>(&body) {
        Ok(state) => Stored::Present(state),
        Err(error) => Stored::Invalid(format!("sync job state is not readable: {error}")),
    })
}

/// The seed of a job without a state: its last run from the configuration
/// (when the clock was plausible) and its last `results.tsv` row.
fn legacy_seed(job: Option<&SyncJob>, id: &str, now: i64) -> JobState {
    let mut state = empty_state();
    if let Some(last_run) = job
        .map(|job| job.last_run)
        .filter(|last_run| *last_run > 0 && *last_run <= now.saturating_add(CLOCK_SLACK_SECS))
    {
        state.last_success = Some(last_run);
        state.last_attempt = Some(last_run);
    }
    if let Some(result) = legacy_result(id) {
        state.last_attempt = Some(state.last_attempt.unwrap_or(0).max(result.when));
        state.last_result = Some(result);
    }
    state
}

fn seed_for_id(id: &str, now: i64) -> JobState {
    let job = load_job_file(&job_file(&jobs_dir(), id)).ok();
    legacy_seed(job.as_ref(), id, now)
}

fn load_in(dir: &Path, id: &str, seed: impl FnOnce() -> JobState) -> io::Result<JobState> {
    Ok(match read_state(&state_path(dir, id))? {
        Stored::Present(state) => state,
        Stored::Missing => seed(),
        Stored::Invalid(message) => JobState {
            load_error: Some(message),
            ..seed()
        },
    })
}

/// The state of one job (missing → seeded, unreadable → seeded with
/// `load_error`). Errors: reading failed for another reason.
pub fn load_job_state(id: &str) -> io::Result<JobState> {
    let id = checked_id(id)?;
    load_in(&state_dir(), id, || seed_for_id(id, now_secs()))
}

/// The states of the given jobs for job lists; never fails as a whole (an
/// unreadable state is seeded with `load_error`). Callers that render often
/// cache the result and reload on a change notice or after about a second.
pub fn load_job_states(jobs: &[SyncJob]) -> BTreeMap<String, JobState> {
    let dir = state_dir();
    let now = now_secs();
    jobs.iter()
        .map(|job| {
            let state = checked_id(&job.id)
                .and_then(|id| load_in(&dir, id, || legacy_seed(Some(job), id, now)))
                .unwrap_or_else(|error| JobState {
                    load_error: Some(error.to_string()),
                    ..legacy_seed(Some(job), &job.id, now)
                });
            (job.id.clone(), state)
        })
        .collect()
}

fn ensure_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let metadata = std::fs::symlink_metadata(dir)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "sync job state path is not a regular directory: {}",
                dir.display()
            ),
        ));
    }
    Ok(())
}

/// Locked read-modify-write; `change` may refuse (`Err`), then nothing is
/// written.
fn try_update_in(
    dir: &Path,
    id: &str,
    seed: impl FnOnce() -> JobState,
    change: impl FnOnce(&mut JobState) -> io::Result<()>,
) -> io::Result<JobState> {
    ensure_dir(dir)?;
    let _lock = StateLock::acquire(&lock_path(dir, id), LOCK_WAIT)?;
    let path = state_path(dir, id);
    let mut state = match read_state(&path)? {
        Stored::Present(state) => state,
        Stored::Missing => seed(),
        Stored::Invalid(message) => {
            // Kept once for inspection; the newest unreadable copy wins.
            std::fs::rename(&path, corrupt_path(dir, id))?;
            JobState {
                blocked: Some(Blocked { kind: BlockKind::Other,
                    detail: format!("Job-Zustand unlesbar ({message}); Einstellungen und letzten Lauf prüfen."),
                    since: now_secs(), confirmed: false }),
                load_error: Some(message),
                ..seed()
            }
        }
    };
    change(&mut state)?;
    state.version = JOB_STATE_VERSION;
    let body = serde_json::to_vec_pretty(&state).map_err(io::Error::other)?;
    atomic_write(&path, &body)?;
    Ok(state)
}

fn try_update(
    id: &str,
    change: impl FnOnce(&mut JobState) -> io::Result<()>,
) -> io::Result<JobState> {
    let id = checked_id(id)?;
    try_update_in(&state_dir(), id, || seed_for_id(id, now_secs()), change)
}

/// Locked read-modify-write of one job's state; returns what was stored.
pub fn update_job_state(id: &str, change: impl FnOnce(&mut JobState)) -> io::Result<JobState> {
    try_update(id, |state| {
        change(state);
        Ok(())
    })
}

/// Records one finished attempt of any runner (locked): attempt and success
/// times, failure series and backoff, error, block, result, run mark and the
/// outstanding trigger it covered.
pub fn record_attempt(id: &str, report: &AttemptReport) -> io::Result<JobState> {
    update_job_state(id, |state| apply_attempt(state, report))
}

/// The user confirmed the shown block ("Trotzdem ausführen"): the next run may
/// pass exactly this stop once. Errors: `InvalidInput` when the job is not
/// blocked by `kind` (any more).
pub fn confirm_block(id: &str, kind: &BlockKind) -> io::Result<JobState> {
    let now = now_secs();
    try_update(id, |state| {
        if confirm(state, kind, now) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Die Sperre hat sich inzwischen geändert; bitte erneut prüfen.",
            ))
        }
    })
}

/// Removes the state of a deleted job; a missing state is no error.
pub fn remove_job_state(id: &str) -> io::Result<()> {
    let id = checked_id(id)?;
    let dir = state_dir();
    remove_if_present(&state_path(&dir, id))?;
    // Best effort: a writer may still hold the lock file open (Windows), and
    // a left-over copy of an unreadable state is only kept for inspection.
    let _ = remove_if_present(&lock_path(&dir, id));
    let _ = remove_if_present(&corrupt_path(&dir, id));
    Ok(())
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Problems of these jobs due for a system notification at `now`, marked as
/// notified (throttled per job and problem). Called by the background worker.
pub fn take_problem_notices(jobs: &[SyncJob], now: i64) -> Vec<ProblemNotice> {
    let dir = state_dir();
    let mut notices = Vec::new();
    for job in jobs.iter().filter(|job| job.enabled) {
        let Ok(id) = checked_id(&job.id) else {
            continue;
        };
        let Ok(state) = load_in(&dir, id, || legacy_seed(Some(job), id, now)) else {
            continue;
        };
        let Some(key) = problem_key(&state) else {
            continue;
        };
        if !notice_due(state.notified.as_ref(), &key, now) {
            continue;
        }
        // Checked again under the lock, so two workers never notify twice.
        let mut notice = None;
        let marked = try_update_in(
            &dir,
            id,
            || legacy_seed(Some(job), id, now),
            |state| {
                let (Some(key), Some(kind)) = (problem_key(state), state.problem()) else {
                    return Err(io::Error::other("problem cleared meanwhile"));
                };
                if !notice_due(state.notified.as_ref(), &key, now) {
                    return Err(io::Error::other("problem notified meanwhile"));
                }
                notice = Some(build_notice(job, state, kind));
                state.notified = Some(Notified { key, at: now });
                Ok(())
            },
        );
        if marked.is_ok() {
            notices.extend(notice);
        }
    }
    notices
}

/// The last result of every job that has a state, plus legacy `results.tsv`
/// rows of jobs without one (for `load_results`).
pub(super) fn stored_results(legacy: BTreeMap<String, JobResult>) -> BTreeMap<String, JobResult> {
    let mut results = legacy;
    let Ok(entries) = std::fs::read_dir(state_dir()) else {
        return results;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some(STATE_EXTENSION) {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if let Ok(Stored::Present(state)) = read_state(&path) {
            match state.last_result {
                Some(result) => {
                    results.insert(id.to_string(), result);
                }
                None => {
                    results.remove(id);
                }
            }
        }
    }
    results
}

#[cfg(test)]
#[path = "job_state_store_tests.rs"]
mod review_task_job_state_store_tests;
