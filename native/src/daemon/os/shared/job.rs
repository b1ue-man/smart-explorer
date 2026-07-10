use crate::syncjobs::SyncJob;
use std::sync::atomic::{AtomicBool, Ordering};

use super::platform;
use super::state::{log, now_secs};

/// Run one job to completion (synchronously). Endpoints are resolved the same
/// way the GUI does - local paths directly, remote URLs by re-opening the
/// matching saved connection (credentials live in the OS keyring).
pub(crate) fn run_one(job: &SyncJob, cancel: &AtomicBool) {
    if canceled(job, cancel) {
        return;
    }
    // This must stay ahead of endpoint resolution and run_before: malformed
    // persisted safety fields must never get a chance to connect or mutate.
    let prepared = match PreparedJob::new(job, now_secs()) {
        Ok(prepared) => prepared,
        Err(error) => {
            log(&format!(
                "skip sync job {:?}: invalid configuration: {error}",
                safe_job_label(job)
            ));
            return;
        }
    };
    let (a, root_a) = match crate::connect::resolve_endpoint(&job.source) {
        Ok(x) => x,
        Err(e) => {
            log(&format!("skip '{}': source {}", job.name, e));
            return;
        }
    };
    if canceled(job, cancel) {
        return;
    }
    let (b, root_b) = match crate::connect::resolve_endpoint(&job.target) {
        Ok(x) => x,
        Err(e) => {
            log(&format!("skip '{}': target {}", job.name, e));
            return;
        }
    };
    if canceled(job, cancel) {
        return;
    }
    if !job.run_before.trim().is_empty() {
        if let Err(error) = run_cmd(&job.run_before) {
            log(&format!(
                "before-command failed for '{}': {error}",
                job.name
            ));
            persist_attempt(
                job,
                crate::syncjobs::JobResult {
                    when: now_secs(),
                    errors: 1,
                    note: "Befehl davor fehlgeschlagen".into(),
                    ..Default::default()
                },
                true,
            );
            return;
        }
    }
    if canceled(job, cancel) {
        return;
    }
    let (min_size, max_size, after, before) = prepared.bounds;
    let filter = crate::bisync::WalkFilter {
        include_hidden: job.include_hidden,
        ignore: &prepared.ignore,
        min_size,
        max_size,
        after_mtime_ms: after,
        before_mtime_ms: before,
    };
    let out = crate::bisync::run(&*a, &root_a, &*b, &root_b, prepared.opts, cancel, &filter);
    let was_canceled =
        cancel.load(Ordering::Acquire) || out.errors.iter().any(|(kind, _)| kind == "abgebrochen");
    log(&format!(
        "ran '{}' [{}]: {}→ {}← {}del {}conf {}err",
        job.name,
        job.trigger.as_str(),
        out.stats.a_to_b,
        out.stats.b_to_a,
        out.stats.deleted,
        out.conflicts.len(),
        out.errors.len()
    ));
    let note = if was_canceled {
        "abgebrochen"
    } else if !out.errors.is_empty() {
        "Fehler"
    } else if !out.conflicts.is_empty() {
        "Konflikte"
    } else {
        "ok"
    };
    let mut result = crate::syncjobs::JobResult {
        when: now_secs(),
        a_to_b: out.stats.a_to_b,
        b_to_a: out.stats.b_to_a,
        deleted: out.stats.deleted,
        conflicts: out.conflicts.len() as u64,
        errors: out.errors.len() as u64,
        note: note.into(),
    };
    // A post-command observes every completed sync attempt, including one with
    // conflicts or errors. Cancellation is the sole exception: shutdown must
    // not launch more user code. Its own failure becomes part of the result.
    if !was_canceled && !job.run_after.trim().is_empty() {
        if let Err(error) = run_cmd(&job.run_after) {
            log(&format!("after-command failed for '{}': {error}", job.name));
            result.errors = result.errors.saturating_add(1);
            result.note = if result.note == "ok" {
                "Befehl danach fehlgeschlagen".into()
            } else {
                format!("{}; Befehl danach fehlgeschlagen", result.note)
            };
        }
    }
    persist_attempt(job, result, !was_canceled);
}

struct PreparedJob {
    opts: crate::bisync::BisyncOptions,
    ignore: globset::GlobSet,
    bounds: (u64, u64, i64, i64),
}

impl PreparedJob {
    fn new(job: &SyncJob, now: i64) -> Result<Self, String> {
        job.validate()?;
        Ok(PreparedJob {
            opts: job.checked_opts(false)?,
            ignore: job.checked_glob_set()?,
            bounds: job.checked_filter_bounds(now)?,
        })
    }
}

fn safe_job_label(job: &SyncJob) -> String {
    let label = if job.name.trim().is_empty() {
        &job.id
    } else {
        &job.name
    };
    label.chars().flat_map(char::escape_default).collect()
}

fn canceled(job: &SyncJob, cancel: &AtomicBool) -> bool {
    if cancel.load(Ordering::Acquire) {
        log(&format!("canceled '{}' during daemon stop", job.name));
        true
    } else {
        false
    }
}

/// Run a user-specified shell command and require a successful exit status.
fn run_cmd(cmd: &str) -> Result<(), String> {
    match platform::run_shell_command(cmd) {
        Ok(status) if status.success() => {
            log(&format!("ran command ({status}): {cmd}"));
            Ok(())
        }
        Ok(status) => Err(exit_failure(status.code())),
        Err(error) => Err(format!("could not start: {error}")),
    }
}

fn exit_failure(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exited with status {code}"),
        None => "terminated without an exit status".into(),
    }
}

fn persist_attempt(job: &SyncJob, result: crate::syncjobs::JobResult, mark_run: bool) {
    if mark_run {
        if let Err(error) = crate::syncjobs::mark_run(&job.id) {
            log(&format!(
                "could not persist last run for '{}': {error}",
                job.name
            ));
        }
    }
    if let Err(error) = crate::syncjobs::record_result(&job.id, &result) {
        log(&format!(
            "could not persist result for '{}': {error}",
            job.name
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_job() -> SyncJob {
        SyncJob::new("job".into(), "/source".into(), "/target".into())
    }

    #[test]
    fn preparation_rejects_invalid_config_before_runtime_work() {
        let mut job = valid_job();
        job.max_delete_pct = 101;
        let error = match PreparedJob::new(&job, 1_700_000_000) {
            Ok(_) => panic!("invalid deletion guard must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("max_delete_pct"));

        let mut job = valid_job();
        job.ignore = vec!["[".into()];
        assert!(PreparedJob::new(&job, 1_700_000_000).is_err());
    }

    #[test]
    fn preparation_checks_time_dependent_filter_arithmetic() {
        let mut job = valid_job();
        job.filter_min_age_days = 1;
        let error = match PreparedJob::new(&job, 1) {
            Ok(_) => panic!("unrepresentable minimum-age bound must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("minimum-age"));
    }

    #[test]
    fn hook_exit_failure_is_explicit_for_codes_and_signals() {
        assert_eq!(exit_failure(Some(7)), "exited with status 7");
        assert!(exit_failure(None).contains("without an exit status"));
    }
}
