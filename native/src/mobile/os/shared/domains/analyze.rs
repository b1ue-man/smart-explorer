//! `analyze.*` (storage analysis) and `reclaim.*` (duplicates): the desktop
//! `analytics` scanners on a scan thread with the desktop stack size, live
//! progress with phase and current folder, and the finished result kept per
//! task for drill-down. Other apps' private folders (`Android/data`,
//! `Android/obb`) are protected omissions, not read errors; Android's own
//! totals fill in what no app may walk as approximate rows of the view.
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{canceled, i64_arg, invalid, reject_app_internal, str_arg, string_list};
use super::locations::{is_local, join_segments, location_for};
use crate::analytics::{
    protected_count, protected_text, Approximations, DuplicateGroup, DuplicateReport,
    DuplicateSummary, PlatformTotals, ScanOutcome, ScanStatus, VolumeRoot,
};
use crate::mobile::{ApiError, Runtime, TaskCtx};

/// Results kept for drill-down (oldest finished one dropped first).
const MAX_RESULTS: usize = 4;
const MAX_NODE_CHILDREN: usize = 500;
const PROGRESS_TICK: Duration = Duration::from_millis(250);

enum Stored {
    Analysis {
        outcome: ScanOutcome,
        approx: Approximations,
        base: String,
        root: String,
    },
    Duplicates {
        groups: Vec<DuplicateGroup>,
        summary: DuplicateSummary,
        base: String,
    },
}

struct Slot {
    token: u64,
    task: Option<String>,
    result: Option<Stored>,
}

static RESULTS: Mutex<VecDeque<Slot>> = Mutex::new(VecDeque::new());
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn with_results<T>(work: impl FnOnce(&mut VecDeque<Slot>) -> T) -> T {
    work(&mut RESULTS.lock().unwrap_or_else(PoisonError::into_inner))
}

fn open_slot() -> u64 {
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    with_results(|slots| {
        while slots.len() >= MAX_RESULTS {
            // A running scan keeps its slot while a finished result can go.
            let oldest = slots
                .iter()
                .position(|slot| slot.result.is_some())
                .unwrap_or(0);
            slots.remove(oldest);
        }
        slots.push_back(Slot {
            token,
            task: None,
            result: None,
        });
    });
    token
}

fn bind_task(token: u64, task: &str) {
    with_results(|slots| {
        if let Some(slot) = slots.iter_mut().find(|slot| slot.token == token) {
            slot.task = Some(task.to_string());
        }
    });
}

/// A failed or canceled scan has no result: its slot is freed at once.
fn release_on_error(token: u64, outcome: Result<Value, ApiError>) -> Result<Value, ApiError> {
    if outcome.is_err() {
        with_results(|slots| slots.retain(|slot| slot.token != token));
    }
    outcome
}

fn store(token: u64, result: Stored) {
    with_results(|slots| {
        if let Some(slot) = slots.iter_mut().find(|slot| slot.token == token) {
            slot.result = Some(result);
        }
    });
}

fn with_stored<T>(
    task: &str,
    work: impl FnOnce(&Stored) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    with_results(|slots| {
        let slot = slots
            .iter()
            .find(|slot| slot.task.as_deref() == Some(task))
            .ok_or_else(|| ApiError::new("not_found", "Ergebnis nicht mehr vorhanden."))?;
        match &slot.result {
            Some(result) => work(result),
            None => Err(ApiError::new("busy", "Der Scan läuft noch.")),
        }
    })
}

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

/// The scan root: a local path as is, a remote location as its uncached
/// backend (a full walk must not fill the browsing cache).
fn resolve_remote(location: &str) -> Result<(crate::vfs::BackendHandle, String), ApiError> {
    let (backend, root) = Runtime::get()?.resolve(location)?;
    Ok((crate::vfs::sync_backend(backend), root))
}

/// `analyze.start {location, platform?:{volumeUsedBytes?, otherAppsBytes?}}`:
/// Android's totals for the volume of a local root (missing, `null` or
/// negative values are unknown).
pub(super) fn start_analysis(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let location = checked_location(args)?;
    let place = if is_local(&location) {
        volume_root(rt, &location)
    } else {
        VolumeRoot::default()
    };
    let total = |key: &str| args.get("platform")?.get(key)?.as_u64();
    let totals = PlatformTotals {
        volume_used_bytes: total("volumeUsedBytes"),
        other_apps_bytes: total("otherAppsBytes"),
    };
    let token = open_slot();
    let title = format!("Speicheranalyse: {location}");
    let task = rt.spawn_task("analyze", title, move |ctx| {
        release_on_error(token, analysis_task(ctx, token, location, (place, totals)))
    });
    bind_task(token, &task);
    Ok(json!({ "taskId": task }))
}

/// Where a local root lies on its volume; Android reports other apps' data
/// only for the primary one.
fn volume_root(rt: &Runtime, location: &str) -> VolumeRoot {
    let Some(place) = crate::apptrash::volume_place(Path::new(location)) else {
        return VolumeRoot::default();
    };
    let primary = rt
        .volumes()
        .iter()
        .any(|volume| volume.primary && Path::new(&volume.path) == place.volume.as_path());
    VolumeRoot::from_segments(&place.below, primary)
}

fn analysis_task(
    ctx: &TaskCtx,
    token: u64,
    location: String,
    (place, totals): (VolumeRoot, PlatformTotals),
) -> Result<Value, ApiError> {
    let progress = crate::analytics::Progress::default();
    let remote = if is_local(&location) {
        None
    } else {
        ctx.message("Verbinde…");
        Some(resolve_remote(&location)?)
    };
    let root = remote
        .as_ref()
        .map(|(_, root)| root.clone())
        .unwrap_or_else(|| location.clone());
    ctx.message("Analysiere…");
    let scan_progress = progress.clone();
    let local_root = location.clone();
    let mut shown = String::new();
    let outcome = run_watched(
        ctx,
        &progress.cancel,
        move || match &remote {
            Some((backend, root)) => {
                crate::analytics::scan_backend(&**backend, root, &scan_progress)
            }
            None => crate::analytics::scan(Path::new(&local_root), &scan_progress),
        },
        || {
            let snapshot = progress.snapshot();
            ctx.progress(snapshot.bytes, 0, snapshot.files, 0);
            show(
                ctx,
                &mut shown,
                crate::analytics::walk_status(snapshot.dirs, &snapshot.current),
            );
        },
    )?;
    let protected = protected_count(&outcome.protected);
    let summary = json!({
        "files": progress.files.load(Ordering::Relaxed),
        "dirs": progress.dirs.load(Ordering::Relaxed),
        "bytes": progress.bytes.load(Ordering::Relaxed),
        "issues": outcome.issues.len() as u64 + outcome.suppressed_issues,
        "protected": protected,
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
        ScanStatus::Partial => ctx.message(&format!(
            "{} Pfade nicht lesbar",
            outcome.issues.len() as u64 + outcome.suppressed_issues
        )),
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
    store(
        token,
        Stored::Analysis {
            outcome,
            approx,
            base: location,
            root,
        },
    );
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
/// `protected|rest` for the approximate rows.
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
        let mut value = to_json(view)?;
        value["location"] = json!(location_for(base, &join_segments(root, &segments)));
        Ok(value)
    })
}

fn to_json(view: impl serde::Serialize) -> Result<Value, ApiError> {
    serde_json::to_value(view).map_err(|error| ApiError::internal(error.to_string()))
}

pub(super) fn issues(args: &Value) -> Result<Value, ApiError> {
    let task = str_arg(args, "taskId")?;
    with_stored(task, |stored| {
        let Stored::Analysis { outcome, .. } = stored else {
            return Err(invalid("Dieser Task ist keine Speicheranalyse."));
        };
        let mut lines: Vec<String> = outcome
            .issues
            .iter()
            .map(|issue| format!("{}: {}", issue.path, issue.detail))
            .collect();
        if outcome.suppressed_issues > 0 {
            lines.push(format!("… {} weitere", outcome.suppressed_issues));
        }
        lines.extend(outcome.notes.iter().cloned());
        Ok(json!({
            "count": outcome.issues.len() as u64 + outcome.suppressed_issues,
            "text": lines.join("\n"),
            "protectedCount": protected_count(&outcome.protected),
            "protectedText": protected_text(&outcome.protected),
        }))
    })
}

pub(super) fn start_reclaim(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let location = checked_location(args)?;
    let min_size = i64_arg(args, "minSize")?;
    let min_size = u64::try_from(min_size)
        .map_err(|_| invalid("Die Mindestgröße darf nicht negativ sein."))?
        .max(1);
    let token = open_slot();
    let title = format!("Duplikate: {location}");
    let task = rt.spawn_task("reclaim", title, move |ctx| {
        release_on_error(token, reclaim_task(ctx, token, location, min_size))
    });
    bind_task(token, &task);
    Ok(json!({ "taskId": task }))
}

fn reclaim_task(
    ctx: &TaskCtx,
    token: u64,
    location: String,
    min_size: u64,
) -> Result<Value, ApiError> {
    let progress = crate::analytics::ReclaimProgress::default();
    let opts = crate::analytics::ReclaimOptions {
        duplicate_min_bytes: min_size,
        ..Default::default()
    };
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
    // Local: every file of at least `minSize` is a candidate, compared in
    // parallel. Remote: provider/agent MD5 with the reclaim walk's caps.
    let report = run_watched(
        ctx,
        &progress.cancel,
        move || match remote {
            Some((backend, root)) => DuplicateReport::from_reclaim(
                crate::analytics::scan_reclaim_backend(backend, &root, &scan_progress, &opts),
            ),
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
    store(
        token,
        Stored::Duplicates {
            groups: report.groups,
            summary: report.summary,
            base: location,
        },
    );
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
    let token = open_slot();
    bind_task(token, task);
    store(
        token,
        Stored::Analysis {
            outcome,
            approx: Approximations::default(),
            base: base.to_string(),
            root: root.to_string(),
        },
    );
}
