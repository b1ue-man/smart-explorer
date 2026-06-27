use crate::syncjobs::SyncJob;
use std::sync::atomic::AtomicBool;

use super::platform;
use super::state::{log, now_secs};

/// Run one job to completion (synchronously). Endpoints are resolved the same
/// way the GUI does - local paths directly, remote URLs by re-opening the
/// matching saved connection (credentials live in the OS keyring).
pub(crate) fn run_one(job: &SyncJob) {
    let (a, root_a) = match crate::connect::resolve_endpoint(&job.source) {
        Ok(x) => x,
        Err(e) => {
            log(&format!("skip '{}': source {}", job.name, e));
            return;
        }
    };
    let (b, root_b) = match crate::connect::resolve_endpoint(&job.target) {
        Ok(x) => x,
        Err(e) => {
            log(&format!("skip '{}': target {}", job.name, e));
            return;
        }
    };
    if !job.run_before.trim().is_empty() {
        run_cmd(&job.run_before);
    }
    let opts = job.opts(false);
    let cancel = AtomicBool::new(false);
    let gs = job.glob_set();
    let (min_size, max_size, after, before) = job.filter_bounds(now_secs());
    let filter = crate::bisync::WalkFilter {
        include_hidden: job.include_hidden,
        ignore: &gs,
        min_size,
        max_size,
        after_mtime_ms: after,
        before_mtime_ms: before,
    };
    let out = crate::bisync::run(&*a, &root_a, &*b, &root_b, opts, &cancel, &filter);
    crate::syncjobs::mark_run(&job.id);
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
    let note = if out.errors.iter().any(|(k, _)| k == "abgebrochen") {
        "abgebrochen"
    } else if !out.errors.is_empty() {
        "Fehler"
    } else if !out.conflicts.is_empty() {
        "Konflikte"
    } else {
        "ok"
    };
    crate::syncjobs::record_result(
        &job.id,
        &crate::syncjobs::JobResult {
            when: now_secs(),
            a_to_b: out.stats.a_to_b,
            b_to_a: out.stats.b_to_a,
            deleted: out.stats.deleted,
            conflicts: out.conflicts.len() as u64,
            errors: out.errors.len() as u64,
            note: note.into(),
        },
    );
    if !job.run_after.trim().is_empty() {
        run_cmd(&job.run_after);
    }
}

/// Run a user-specified shell command (run-before/after a job), waiting for it.
/// Best-effort: failures are logged, not fatal.
fn run_cmd(cmd: &str) {
    match platform::run_shell_command(cmd) {
        Ok(s) => log(&format!("ran command ({}): {}", s, cmd)),
        Err(e) => log(&format!("command failed ({}): {}", e, cmd)),
    }
}
