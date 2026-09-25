//! `sync.run` (the desktop GUI's "Jetzt" path: `resolve_endpoint` +
//! `bisync::run`, then `mark_run`/`record_result`; no pre/post commands) and
//! `sync.mirror` (the desktop "Spiegeln nach…": one-way `sync::start_sync`
//! without deletions, nothing persisted).
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{canceled, invalid, reject_app_internal, str_arg, text_error};
use super::sync_conflicts::{self, PairContext};
use super::sync_jobs::{find_job, notify_jobs};
use crate::mobile::{ApiError, Runtime, TaskCtx};
use crate::syncjobs::SyncJob;

/// Errors listed per task; the rest are summarized in the task message.
const MAX_LISTED_ERRORS: usize = 200;

struct RunSlot {
    token: u64,
    task: Option<String>,
}

static RUNNING: Mutex<BTreeMap<String, RunSlot>> = Mutex::new(BTreeMap::new());
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn running() -> MutexGuard<'static, BTreeMap<String, RunSlot>> {
    RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Task id of the facade run of this job, if one is in progress.
pub(super) fn running_task(job_id: &str) -> Option<String> {
    running().get(job_id).and_then(|slot| slot.task.clone())
}

/// Reserves the job for one task; `None` when one is already in progress.
fn claim(job_id: &str) -> Option<u64> {
    let mut running = running();
    if running.contains_key(job_id) {
        return None;
    }
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    running.insert(job_id.to_string(), RunSlot { token, task: None });
    Some(token)
}

/// Frees the job again when the task ends, also by unwinding.
struct SlotGuard {
    job_id: String,
    token: u64,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let mut running = running();
        if running
            .get(&self.job_id)
            .is_some_and(|slot| slot.token == self.token)
        {
            running.remove(&self.job_id);
        }
    }
}

/// Starts a task that owns the job (run, conflict check, resolution, merge),
/// so two of them never work on the same pair at once.
pub(super) fn spawn_for_job<F>(
    rt: &Runtime,
    job_id: &str,
    title: String,
    work: F,
) -> Result<String, ApiError>
where
    F: FnOnce(&TaskCtx) -> Result<Value, ApiError> + Send + 'static,
{
    let token = claim(job_id).ok_or_else(|| {
        ApiError::new(
            "busy",
            "Für diesen Sync-Job läuft bereits ein Vorgang – bitte warten.",
        )
    })?;
    let guard = SlotGuard {
        job_id: job_id.to_string(),
        token,
    };
    let task = rt.spawn_task("sync", title, move |ctx| {
        let result = work(ctx);
        drop(guard);
        if let Ok(rt) = Runtime::get() {
            notify_jobs(rt);
        }
        result
    });
    if let Some(slot) = running().get_mut(job_id).filter(|slot| slot.token == token) {
        slot.task = Some(task.clone());
    }
    notify_jobs(rt);
    Ok(task)
}

type Bounds = (u64, u64, i64, i64);

fn checked_settings(
    job: &SyncJob,
    dry_run: bool,
) -> Result<(crate::bisync::BisyncOptions, Bounds, globset::GlobSet), ApiError> {
    let settings = (|| {
        job.validate()?;
        Ok::<_, String>((
            job.checked_opts(dry_run)?,
            job.checked_filter_bounds(super::args::now_secs())?,
            job.checked_glob_set()?,
        ))
    })();
    settings.map_err(|error| invalid(format!("Ungültiges Sync-Setup: {error}")))
}

/// Opens both job sides like the desktop job runner (fresh, uncached).
pub(super) fn open_pair(job: &SyncJob) -> Result<PairContext, ApiError> {
    let (a, root_a) = crate::connect::resolve_endpoint(&job.source)
        .map_err(|error| text_error("network", "Seite A", error))?;
    let (b, root_b) = crate::connect::resolve_endpoint(&job.target)
        .map_err(|error| text_error("network", "Seite B", error))?;
    let a = crate::vfs::sync_backend(a);
    let b = crate::vfs::sync_backend(b);
    let pair = crate::bisync::pair_id_for(&*a, &root_a, &*b, &root_b);
    Ok(PairContext {
        a,
        root_a,
        b,
        root_b,
        pair,
    })
}

/// Runs a bisync of `job` over `pair` (dry or real) on the calling thread.
pub(super) fn run_bisync(
    ctx: &TaskCtx,
    job: &SyncJob,
    pair: &PairContext,
    dry_run: bool,
) -> Result<crate::bisync::Outcome, ApiError> {
    let (opts, bounds, ignore) = checked_settings(job, dry_run)?;
    let filter = crate::bisync::WalkFilter {
        include_hidden: job.include_hidden,
        ignore: &ignore,
        min_size: bounds.0,
        max_size: bounds.1,
        after_mtime_ms: bounds.2,
        before_mtime_ms: bounds.3,
    };
    let cancel = ctx.cancel_flag();
    Ok(crate::bisync::run(
        &*pair.a,
        &pair.root_a,
        &*pair.b,
        &pair.root_b,
        opts,
        &cancel,
        &filter,
    ))
}

pub(super) fn run(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job = find_job(str_arg(args, "id")?)?;
    reject_app_internal(&job.source)?;
    reject_app_internal(&job.target)?;
    checked_settings(&job, false)?;
    // Resolutions not yet saved would be overwritten by the next run.
    sync_conflicts::settle_before_run(&job.id)?;
    let job_id = job.id.clone();
    let title = format!("Sync: {}", job.name);
    let task = spawn_for_job(rt, &job_id, title, move |ctx| run_job(ctx, &job))?;
    Ok(json!({ "taskId": task }))
}

fn run_job(ctx: &TaskCtx, job: &SyncJob) -> Result<Value, ApiError> {
    ctx.message("Verbinde…");
    let pair = open_pair(job)?;
    if ctx.cancelled() {
        return Err(canceled("Abgebrochen"));
    }
    ctx.message("Synchronisiere…");
    let out = run_bisync(ctx, job, &pair, false)?;
    let was_canceled = ctx.cancelled() || out.errors.iter().any(|(kind, _)| kind == "abgebrochen");
    let note = if was_canceled {
        "abgebrochen"
    } else if !out.errors.is_empty() {
        "Fehler"
    } else if !out.conflicts.is_empty() {
        "Konflikte"
    } else {
        "ok"
    };
    let mut persistence = Vec::new();
    if let Err(error) = crate::syncjobs::mark_run(&job.id) {
        persistence.push(format!("Letzten Lauf speichern: {error}"));
    }
    let record = crate::syncjobs::JobResult {
        when: super::args::now_secs(),
        a_to_b: out.stats.a_to_b,
        b_to_a: out.stats.b_to_a,
        deleted: out.stats.deleted,
        conflicts: out.conflicts.len() as u64,
        errors: out.errors.len() as u64,
        note: out.omissions.result_note(note),
    };
    if let Err(error) = crate::syncjobs::record_result(&job.id, &record) {
        persistence.push(format!("Laufergebnis speichern: {error}"));
    }
    report_errors(ctx, &out.errors);
    for message in &persistence {
        ctx.error("", message);
    }
    let omitted = out.omissions.summary();
    let summary = run_summary(&out, omitted.as_deref(), was_canceled);
    let conflicts = out.conflicts.len();
    sync_conflicts::store_run(&job.id, pair, out.conflicts);
    ctx.message(&summary);
    if was_canceled {
        return Err(canceled(summary));
    }
    Ok(json!({
        "summary": summary,
        "aToB": record.a_to_b,
        "bToA": record.b_to_a,
        "deleted": record.deleted,
        "conflicts": conflicts,
        "errors": record.errors as usize + persistence.len(),
        "omitted": omitted,
    }))
}

fn run_summary(out: &crate::bisync::Outcome, omitted: Option<&str>, canceled: bool) -> String {
    let stats = &out.stats;
    let mut summary = format!(
        "{} →, {} ←, {} gelöscht, {} Konflikte ({} MB)",
        stats.a_to_b,
        stats.b_to_a,
        stats.deleted,
        out.conflicts.len(),
        stats.bytes / 1_048_576
    );
    if let Some(omitted) = omitted {
        summary = format!("{summary}; {omitted}");
    }
    if !out.errors.is_empty() {
        summary = format!("{summary}; {} Fehler", out.errors.len());
    }
    if canceled {
        summary = format!("Abgebrochen; {summary}");
    }
    summary
}

pub(super) fn report_errors(ctx: &TaskCtx, errors: &[(String, String)]) {
    for (path, message) in errors.iter().take(MAX_LISTED_ERRORS) {
        ctx.error(path, message);
    }
    if errors.len() > MAX_LISTED_ERRORS {
        ctx.error(
            "",
            &format!("{} weitere Fehler", errors.len() - MAX_LISTED_ERRORS),
        );
    }
}

pub(super) fn mirror(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let source = str_arg(args, "source")?.to_string();
    let target = str_arg(args, "target")?.to_string();
    reject_app_internal(&source)?;
    reject_app_internal(&target)?;
    crate::connect::validate_sync_endpoints(&source, &target).map_err(invalid)?;
    let title = format!("Spiegeln nach {target}");
    let task = rt.spawn_task("mirror", title, move |ctx| {
        mirror_task(ctx, &source, &target)
    });
    Ok(json!({ "taskId": task }))
}

fn mirror_task(ctx: &TaskCtx, source: &str, target: &str) -> Result<Value, ApiError> {
    ctx.message("Verbinde…");
    let rt = Runtime::get()?;
    let (src, src_root) = rt.resolve(source)?;
    let (dst, dst_root) = rt.resolve(target)?;
    let (tx, rx) = crossbeam_channel::unbounded();
    let handle = crate::sync::start_sync(
        crate::vfs::sync_backend(src),
        src_root,
        crate::vfs::sync_backend(dst),
        dst_root,
        crate::sync::SyncOptions {
            delete_extra: false,
            dry_run: false,
        },
        tx,
    );
    let result = loop {
        if ctx.cancelled() {
            handle.cancel.store(true, Ordering::Relaxed);
        }
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(crate::sync::SyncMsg::Progress(progress)) => {
                let stats = &progress.stats;
                ctx.progress(
                    stats.bytes,
                    0,
                    stats.copied.saturating_add(stats.skipped),
                    0,
                );
                ctx.message(&progress.current);
            }
            Ok(crate::sync::SyncMsg::Done(result)) => break result,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                return Err(ApiError::new(
                    "internal",
                    "Spiegelungs-Thread wurde ohne Ergebnis beendet.",
                ));
            }
        }
    };
    let was_canceled = handle.cancel.load(Ordering::Relaxed);
    let stats = &result.stats;
    let omitted = result.omissions.summary();
    let suffix = omitted
        .as_deref()
        .map(|text| format!("; {text}"))
        .unwrap_or_default();
    report_errors(ctx, &result.errors);
    let summary = if stats.errors > 0 {
        format!(
            "Spiegelung unvollständig: {} kopiert, {} Fehler{suffix}",
            stats.copied, stats.errors
        )
    } else if was_canceled {
        format!(
            "Spiegelung abgebrochen: {} bereits kopiert{suffix}",
            stats.copied
        )
    } else {
        format!(
            "Spiegelung fertig: {} kopiert, {} übersprungen ({} MB){suffix}",
            stats.copied,
            stats.skipped,
            stats.bytes / 1_048_576
        )
    };
    ctx.message(&summary);
    if was_canceled {
        return Err(canceled(summary));
    }
    Ok(json!({
        "summary": summary,
        "copied": stats.copied,
        "skipped": stats.skipped,
        "errors": stats.errors,
        "omitted": omitted,
    }))
}
