//! `analyze.*` (storage analysis) and `reclaim.*` (duplicates): the desktop
//! `analytics` scanners on a scan thread with the desktop stack size, live
//! progress with phase and current folder, and the finished result kept per
//! task for drill-down. A remote location is analysed by the device that
//! holds its data, exactly as on the desktop (`analytics::scan_remote`: the
//! host's own worker first, older paths only as fallback). Other apps'
//! private folders (`Android/data`, `Android/obb`) are protected omissions,
//! not read errors; Android's own figures (volume, other apps' data, the
//! installed apps) fill in what no app may walk as approximate rows.
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{canceled, i64_arg, invalid, reject_app_internal, str_arg, string_list};
use super::locations::{is_local, join_segments, location_for};
use crate::analytics::{
    protected_count, protected_text, Approximations, NodeKind, PlatformTotals, ScanOutcome,
    ScanStatus, VolumeRoot,
};
use crate::mobile::location::Loc;
use crate::mobile::{ApiError, Runtime, TaskCtx};

#[path = "analyze_progress.rs"]
mod phases;
#[path = "analyze_platform.rs"]
mod platform;
#[path = "analyze_results.rs"]
mod results;

use results::{bind_task, with_stored, Pending, Stored};

const MAX_NODE_CHILDREN: usize = 500;
const PROGRESS_TICK: Duration = Duration::from_millis(250);

/// Runs `work` on a scan thread; the task thread reports progress, forwards
/// a cancel to `cancel` and waits for the result.
fn run_watched<T: Send>(
    ctx: &TaskCtx,
    cancel: &AtomicBool,
    work: impl FnOnce() -> T + Send,
    mut report: impl FnMut(),
) -> Result<T, ApiError> {
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .name("storage-scan".into())
            .stack_size(crate::analytics::SCAN_THREAD_STACK_BYTES)
            .spawn_scoped(scope, work)
            .map_err(|error| super::args::io_error("Scan starten", error))?;
        while !handle.is_finished() {
            if ctx.cancelled() {
                cancel.store(true, Ordering::Relaxed);
            }
            report();
            std::thread::sleep(PROGRESS_TICK);
        }
        report();
        handle
            .join()
            .map_err(|_| ApiError::new("internal", "Der Scan ist unerwartet abgebrochen."))
    })
}

fn checked_location(args: &Value) -> Result<String, ApiError> {
    let location = str_arg(args, "location")?.to_string();
    reject_app_internal(&location)?;
    if location.trim().is_empty() {
        return Err(invalid("Bitte einen Ort wählen."));
    }
    Ok(location)
}

/// The scan root of a remote location: its live connection (a lost pooled
/// connection is reopened before the scan starts) as the uncached backend,
/// since a full walk must not fill the browsing cache.
fn resolve_remote(location: &str) -> Result<(crate::vfs::BackendHandle, String), ApiError> {
    let (backend, root) = Runtime::get()?.resolve_live(&Loc::parse(location)?)?;
    Ok((crate::vfs::sync_backend(backend), root))
}

/// What an analysis walks: a local path, or a remote root that the device
/// holding it analyses (`scan_remote`: its own worker first, then an agent's
/// walk, the listing walk from here only as the last way).
enum Target {
    Local(String),
    Remote(crate::vfs::BackendHandle, String),
}

impl Target {
    fn root(&self) -> &str {
        match self {
            Target::Local(root) | Target::Remote(_, root) => root,
        }
    }
}

fn scan_target(target: &Target, progress: &crate::analytics::Progress) -> ScanOutcome {
    match target {
        Target::Remote(backend, root) => crate::analytics::scan_remote(&**backend, root, progress),
        Target::Local(root) => crate::analytics::scan(Path::new(root), progress),
    }
}

/// The answer of a start: the task and whether it works against another
/// device (the app keeps the CPU awake while such a task runs).
fn started(task: String, remote: bool) -> Value {
    json!({ "taskId": task, "remote": remote })
}

/// `analyze.start {location, platform?:{volumeUsedBytes?, otherAppsBytes?,
/// apps?}}` → `{taskId, remote}`: Android's figures for the volume of a
/// local root (`analyze_platform.rs`); a remote root has none.
pub(super) fn start_analysis(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let location = checked_location(args)?;
    let remote = !is_local(&location);
    let (place, totals) = if remote {
        (VolumeRoot::default(), PlatformTotals::default())
    } else {
        let place = platform::volume_root(rt, &location);
        (place, platform::platform_totals(args))
    };
    let pending = Pending::open();
    let token = pending.token();
    let title = format!("Speicheranalyse: {location}");
    let task = rt.spawn_task("analyze", title, move |ctx| {
        analysis_task(ctx, pending, location, (place, totals))
    });
    bind_task(token, &task);
    Ok(started(task, remote))
}

fn analysis_task(
    ctx: &TaskCtx,
    pending: Pending,
    location: String,
    (place, totals): (VolumeRoot, PlatformTotals),
) -> Result<Value, ApiError> {
    let scan_progress = crate::analytics::Progress::default();
    let target = if is_local(&location) {
        Target::Local(location.clone())
    } else {
        ctx.message("Verbinde…");
        let (backend, root) = resolve_remote(&location)?;
        Target::Remote(backend, root)
    };
    let root = target.root().to_string();
    let is_remote = matches!(target, Target::Remote(..));
    ctx.message("Analysiere…");
    let progress = scan_progress.clone();
    let mut shown = String::new();
    let outcome = run_watched(
        ctx,
        &progress.cancel,
        move || scan_target(&target, &scan_progress),
        || {
            let snapshot = progress.snapshot();
            let (done, total) = phases::task_bytes(&snapshot);
            ctx.progress(done, total, snapshot.files, 0);
            let quiet = is_remote.then(|| progress.remote_report_age()).flatten();
            show(ctx, &mut shown, phases::status(is_remote, &snapshot, quiet));
        },
    )?;
    let protected = protected_count(&outcome.protected);
    let issue_count = outcome.issues.len() as u64 + outcome.suppressed_issues;
    let snapshot = progress.snapshot();
    // The measured totals (the transfer may have shown its own bytes).
    ctx.progress(snapshot.bytes, 0, snapshot.files, 0);
    let summary = json!({
        "files": snapshot.files,
        "dirs": snapshot.dirs,
        "bytes": snapshot.bytes,
        "issues": issue_count,
        "protected": protected,
        "notes": outcome.notes.len(),
    });
    match outcome.status {
        ScanStatus::Canceled => return Err(canceled("Analyse abgebrochen")),
        ScanStatus::Failed => {
            let detail = outcome
                .issues
                .first()
                .map(|issue| format!("{}: {}", issue.path, issue.detail))
                .unwrap_or_else(|| "Analyse fehlgeschlagen".to_string());
            return Err(ApiError::new("internal", detail));
        }
        ScanStatus::Partial => ctx.message(&format!("{issue_count} Pfade nicht lesbar")),
        ScanStatus::Complete if protected > 0 => {
            ctx.message(&format!("{protected} geschützte Einträge ausgelassen"))
        }
        ScanStatus::Complete => ctx.message(""),
    }
    let approx = outcome
        .tree
        .as_ref()
        .map_or_else(Approximations::default, |tree| {
            let complete = outcome.status == ScanStatus::Complete;
            Approximations::compute(tree, &place, totals, complete)
        });
    pending.store(Stored::Analysis {
        outcome,
        approx,
        base: location,
        root,
    });
    Ok(summary)
}

/// Sets the task message when the status line changed (at most per tick).
fn show(ctx: &TaskCtx, shown: &mut String, line: String) {
    if *shown != line {
        ctx.message(&line);
        *shown = line;
    }
}

/// `analyze.node {taskId, path}` → `{name, size, measured, isDir, kind,
/// children:[{name, size, isDir, childCount, kind}], location}`; `size`
/// includes the approximate rows below, `kind` is `dir|file|aggregate`, or
/// `protected|rest|apps` for the approximate rows. The apps row's name leads
/// to the app list (`location` null), whose rows (`kind: "app"`) add
/// `package, appBytes, dataBytes, cacheBytes`.
pub(super) fn node(args: &Value) -> Result<Value, ApiError> {
    let task = str_arg(args, "taskId")?;
    let segments = string_list(args, "path")?;
    with_stored(task, |stored| {
        let Stored::Analysis {
            outcome,
            approx,
            base,
            root,
        } = stored
        else {
            return Err(invalid("Dieser Task ist keine Speicheranalyse."));
        };
        let tree = outcome
            .tree
            .as_ref()
            .ok_or_else(|| ApiError::new("not_found", "Kein Analyseergebnis."))?;
        let view = crate::analytics::node_view(tree, &segments, approx, MAX_NODE_CHILDREN)
            .ok_or_else(|| ApiError::new("not_found", "Eintrag nicht gefunden."))?;
        // The app list is Android's figures, no place in "Dateien".
        let listed = view.kind == NodeKind::Apps;
        let mut value = to_json(view)?;
        value["location"] = if listed {
            Value::Null
        } else {
            json!(location_for(base, &join_segments(root, &segments)))
        };
        Ok(value)
    })
}

fn to_json(view: impl serde::Serialize) -> Result<Value, ApiError> {
    serde_json::to_value(view).map_err(|error| ApiError::internal(error.to_string()))
}

/// `analyze.issues {taskId}` → `{count, text, notes:[String], protectedCount,
/// protectedText}`: `text` lists the unreadable paths, `notes` the remarks
/// of the walk or of the analysing device (aggregated detail, older analysis
/// path, areas the device protects) – shown even without read problems.
pub(super) fn issues(args: &Value) -> Result<Value, ApiError> {
    let task = str_arg(args, "taskId")?;
    with_stored(task, |stored| {
        let Stored::Analysis { outcome, .. } = stored else {
            return Err(invalid("Dieser Task ist keine Speicheranalyse."));
        };
        Ok(issues_json(outcome))
    })
}

fn issues_json(outcome: &ScanOutcome) -> Value {
    let mut lines: Vec<String> = outcome
        .issues
        .iter()
        .map(|issue| format!("{}: {}", issue.path, issue.detail))
        .collect();
    if outcome.suppressed_issues > 0 {
        lines.push(format!("… {} weitere", outcome.suppressed_issues));
    }
    json!({
        "count": outcome.issues.len() as u64 + outcome.suppressed_issues,
        "text": lines.join("\n"),
        "notes": outcome.notes,
        "protectedCount": protected_count(&outcome.protected),
        "protectedText": protected_text(&outcome.protected),
    })
}

/// `analyze.release {taskId}` / `reclaim.release {taskId}` → `{released}`:
/// the app shows this result no more (a new search started or the page went
/// back to the setup); its memory is freed at once.
pub(super) fn release(args: &Value) -> Result<Value, ApiError> {
    let task = str_arg(args, "taskId")?;
    Ok(json!({ "released": results::release(task) }))
}

/// `reclaim.start {location, minSize}` → `{taskId, remote}`.
pub(super) fn start_reclaim(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let location = checked_location(args)?;
    let min_size = i64_arg(args, "minSize")?;
    let min_size = u64::try_from(min_size)
        .map_err(|_| invalid("Die Mindestgröße darf nicht negativ sein."))?
        .max(1);
    let remote = !is_local(&location);
    let pending = Pending::open();
    let token = pending.token();
    let title = format!("Duplikate: {location}");
    let task = rt.spawn_task("reclaim", title, move |ctx| {
        reclaim_task(ctx, pending, location, min_size)
    });
    bind_task(token, &task);
    Ok(started(task, remote))
}

fn reclaim_task(
    ctx: &TaskCtx,
    pending: Pending,
    location: String,
    min_size: u64,
) -> Result<Value, ApiError> {
    let progress = crate::analytics::ReclaimProgress::default();
    let remote = if is_local(&location) {
        None
    } else {
        ctx.message("Verbinde…");
        Some(resolve_remote(&location)?)
    };
    ctx.message("Suche Duplikate…");
    let scan_progress = progress.clone();
    let root = location.clone();
    let mut shown = String::new();
    // Every file of at least `minSize` is a candidate and every group is
    // kept. Local: compared in parallel here. Remote: the backend's own
    // hashes where it has them, else same-size candidates compared by their
    // ends and only then by their content – never every file downloaded.
    let report = run_watched(
        ctx,
        &progress.cancel,
        move || match remote {
            Some((backend, root)) => {
                crate::analytics::find_backend_duplicates(backend, &root, &scan_progress, min_size)
            }
            None => crate::analytics::find_duplicates(Path::new(&root), &scan_progress, min_size),
        },
        || {
            ctx.progress(
                progress.bytes.load(Ordering::Relaxed),
                0,
                progress.files.load(Ordering::Relaxed),
                0,
            );
            show(ctx, &mut shown, progress.status_line());
        },
    )?;
    if progress.cancel.load(Ordering::Relaxed) {
        return Err(canceled("Duplikatsuche abgebrochen"));
    }
    if let Some(error) = &report.root_error {
        return Err(ApiError::new("not_found", error.clone()));
    }
    for error in report.summary.errors.iter().take(100) {
        ctx.error("", error);
    }
    let result = json!({
        "groups": report.groups.len(),
        "reclaimable": report.groups.iter().map(|group| group.reclaimable).sum::<u64>(),
        "errors": report.summary.error_count(),
        "candidates": report.summary.candidates,
        "protected": protected_count(&report.summary.protected),
    });
    ctx.message("");
    pending.store(Stored::Duplicates {
        groups: report.groups,
        summary: report.summary,
        base: location,
    });
    Ok(result)
}

/// `reclaim.summary {taskId}` → `{files, bytes, candidates, compared, groups,
/// protectedCount, protectedText, errorCount, errorText, limit}`.
pub(super) fn summary(args: &Value) -> Result<Value, ApiError> {
    let task = str_arg(args, "taskId")?;
    with_stored(task, |stored| match stored {
        Stored::Duplicates { summary, .. } => to_json(summary.view()),
        Stored::Analysis { .. } => Err(invalid("Dieser Task ist keine Duplikatsuche.")),
    })
}

pub(super) fn groups(args: &Value) -> Result<Value, ApiError> {
    let task = str_arg(args, "taskId")?;
    with_stored(task, |stored| {
        let Stored::Duplicates { groups, base, .. } = stored else {
            return Err(invalid("Dieser Task ist keine Duplikatsuche."));
        };
        let list: Vec<Value> = groups
            .iter()
            .map(|group| {
                let items: Vec<Value> = group
                    .items
                    .iter()
                    .map(|item| {
                        json!({
                            "location": location_for(base, &item.path),
                            "mtimeMs": item.mtime_ms,
                        })
                    })
                    .collect();
                json!({ "size": group.size, "items": items })
            })
            .collect();
        Ok(Value::Array(list))
    })
}

/// Registers a finished analysis under `task` (drill-down tests).
#[cfg(test)]
pub(super) fn insert_analysis_for_test(task: &str, outcome: ScanOutcome, base: &str, root: &str) {
    let pending = Pending::open();
    bind_task(pending.token(), task);
    pending.store(Stored::Analysis {
        outcome,
        approx: Approximations::default(),
        base: base.to_string(),
        root: root.to_string(),
    });
}

#[cfg(test)]
#[path = "analyze_tests.rs"]
mod tests;
