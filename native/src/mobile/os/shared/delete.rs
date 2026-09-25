//! `fs.delete` and `fs.properties` (api.md §4.3). Deleting without
//! `permanent` uses the app trash (local volumes) or the backend's own trash;
//! places without one answer `unsupported` so the app can ask for permanent
//! deletion. Permanent deletion goes through the bounded desktop planner.
use super::args::{bool_or, nonempty_list};
use super::crumbs::volume_of;
use super::error::ApiError;
use super::fs_list::{reject_trash, require_writable};
use super::location::{is_same_or_below, Loc};
use super::runtime::{Runtime, TaskCtx};
use crate::vfs::{Backend, BackendHandle, DeleteDisposition, DeleteTarget, RecursiveDeleteStatus};
use serde_json::{json, Value};
use std::path::Path;

/// Entries a properties walk counts before it stops.
const MAX_PROPERTY_ENTRIES: u64 = 2_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    AppTrash,
    BackendTrash,
    Permanent,
}

pub(crate) fn delete(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let permanent = bool_or(args, "permanent", false);
    let mut locations = Vec::new();
    for location in nonempty_list(args, "locations")? {
        let loc = Loc::parse(&location)?;
        require_writable(&loc)?;
        if loc.is_root() {
            return Err(ApiError::invalid("Die Wurzel kann nicht gelöscht werden"));
        }
        locations.push(loc);
    }
    let locations = collapse_nested(locations);
    let volumes = rt.volumes();
    let mut jobs = Vec::new();
    for loc in locations {
        let (backend, _) = rt.resolve_loc(&loc)?;
        let route = if permanent {
            Route::Permanent
        } else if loc.is_local() {
            if volume_of(&loc.path, &volumes).is_none() {
                return Err(ApiError::unsupported(format!(
                    "Für „{}“ gibt es keinen Papierkorb – endgültig löschen?",
                    loc.name()
                )));
            }
            Route::AppTrash
        } else if backend.delete_disposition() == DeleteDisposition::Recycle {
            Route::BackendTrash
        } else {
            return Err(ApiError::unsupported(
                "Dieser Ort hat keinen Papierkorb – endgültig löschen?",
            ));
        };
        jobs.push((loc, backend, route));
    }
    let title = if permanent {
        format!("Endgültig löschen: {} Element(e)", jobs.len())
    } else {
        format!("In den Papierkorb: {} Element(e)", jobs.len())
    };
    let id = rt.spawn_task("delete", title, move |ctx| run_delete(ctx, jobs));
    Ok(json!({ "taskId": id }))
}

/// Drops locations below another selected location of the same connection.
fn collapse_nested(mut locations: Vec<Loc>) -> Vec<Loc> {
    locations.sort_by(|left, right| left.location().cmp(&right.location()));
    locations.dedup();
    let all = locations.clone();
    locations.retain(|loc| {
        !all.iter().any(|other| {
            other != loc
                && other.kind == loc.kind
                && other.prefix == loc.prefix
                && is_same_or_below(&loc.path, &other.path)
        })
    });
    locations
}

fn run_delete(ctx: &TaskCtx, jobs: Vec<(Loc, BackendHandle, Route)>) -> Result<Value, ApiError> {
    let total = jobs.len() as u64;
    let mut deleted = 0u64;
    let mut failed = 0u64;
    let cancel = ctx.cancel_flag();
    for (index, (loc, backend, route)) in jobs.iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        ctx.progress(0, 0, index as u64, total);
        let result = match route {
            Route::AppTrash => crate::apptrash::move_to_trash(Path::new(&loc.path))
                .map(|_| ())
                .map_err(ApiError::from),
            Route::BackendTrash => recycle(&**backend, &loc.path),
            Route::Permanent => remove_permanently(ctx, &**backend, &loc.path, &cancel),
        };
        match result {
            Ok(()) => deleted += 1,
            Err(error) if error.kind == "canceled" => break,
            Err(error) => {
                failed += 1;
                ctx.error(&loc.location(), &error.message);
            }
        }
    }
    ctx.progress(0, 0, deleted + failed, total);
    let summary = json!({ "deleted": deleted, "failed": failed });
    if failed > 0 && !ctx.cancelled() {
        ctx.set_failure_result(summary);
        return Err(ApiError::internal(format!(
            "{failed} von {total} Element(en) nicht gelöscht"
        )));
    }
    Ok(summary)
}

fn recycle(backend: &dyn Backend, path: &str) -> Result<(), ApiError> {
    let meta = backend.stat(path)?;
    if meta.is_dir {
        backend.remove_dir(path)?;
    } else {
        backend.remove_file_id(path, meta.id.as_deref())?;
    }
    Ok(())
}

fn remove_permanently(
    ctx: &TaskCtx,
    backend: &dyn Backend,
    path: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), ApiError> {
    let meta = backend.stat(path)?;
    let target = DeleteTarget {
        path: path.to_string(),
        id: meta.id,
        is_dir: meta.is_dir,
        is_symlink: meta.is_symlink,
    };
    match crate::vfs::remove_entry_controlled(backend, &target, cancel, |progress| {
        ctx.message(&progress.current);
    }) {
        Ok(report) if report.status == RecursiveDeleteStatus::Complete => Ok(()),
        Ok(_) => Err(ApiError::canceled()),
        Err(failure) => Err(ApiError::from(failure.error)),
    }
}

pub(crate) fn properties(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let mut locations = Vec::new();
    for location in nonempty_list(args, "locations")? {
        let loc = Loc::parse(&location)?;
        reject_trash(&loc)?;
        locations.push(loc);
    }
    let title = match locations.as_slice() {
        [single] => format!("Eigenschaften: {}", single.name()),
        many => format!("Eigenschaften: {} Elemente", many.len()),
    };
    let runtime = rt.clone();
    let id = rt.spawn_task("properties", title, move |ctx| {
        run_properties(&runtime, ctx, locations)
    });
    Ok(json!({ "taskId": id }))
}

#[derive(Default)]
struct Totals {
    files: u64,
    dirs: u64,
    bytes: u64,
    issues: u64,
    truncated: bool,
}

fn run_properties(rt: &Runtime, ctx: &TaskCtx, locations: Vec<Loc>) -> Result<Value, ApiError> {
    let mut totals = Totals::default();
    let mut single = None;
    let count = locations.len();
    for loc in &locations {
        let meta = rt.with_read(loc, |backend, path| backend.stat(path))?;
        if count == 1 {
            single = Some((meta.mtime_ms, meta.btime_ms, loc.location()));
        }
        if meta.is_dir && !meta.is_symlink {
            let (backend, path) = rt.resolve_loc(loc)?;
            walk(ctx, &*backend, &path, &mut totals)?;
        } else {
            totals.files += 1;
            totals.bytes = totals.bytes.saturating_add(meta.size);
        }
        ctx.progress(totals.bytes, 0, totals.files + totals.dirs, 0);
    }
    let mut messages = Vec::new();
    if totals.truncated {
        messages.push("Grenze erreicht – Werte unvollständig".to_string());
    }
    if totals.issues > 0 {
        messages.push(format!("{} Ordner nicht lesbar", totals.issues));
    }
    if !messages.is_empty() {
        ctx.message(&messages.join(" · "));
    }
    let mut result = json!({
        "items": count,
        "files": totals.files,
        "dirs": totals.dirs,
        "bytes": totals.bytes,
    });
    if let Some((mtime, btime, location)) = single {
        result["mtimeMs"] = json!((mtime != 0).then_some(mtime));
        result["btimeMs"] = json!((btime != 0).then_some(btime));
        result["location"] = json!(location);
    }
    Ok(result)
}

/// Counts everything below `root` without following links.
fn walk(
    ctx: &TaskCtx,
    backend: &dyn Backend,
    root: &str,
    totals: &mut Totals,
) -> Result<(), ApiError> {
    let mut stack = vec![root.to_string()];
    while let Some(dir) = stack.pop() {
        if ctx.cancelled() {
            return Err(ApiError::canceled());
        }
        let children = match backend.list_dir(&dir) {
            Ok(children) => children,
            Err(error) => {
                totals.issues += 1;
                ctx.error(&dir, &error.to_string());
                continue;
            }
        };
        for child in children {
            if crate::apptrash::excluded_name(&child.name) {
                continue;
            }
            if totals.files + totals.dirs >= MAX_PROPERTY_ENTRIES {
                totals.truncated = true;
                return Ok(());
            }
            if child.is_dir {
                totals.dirs += 1;
                if !child.is_symlink {
                    stack.push(super::location::join(&dir, &child.name));
                }
            } else {
                totals.files += 1;
                totals.bytes = totals.bytes.saturating_add(child.size);
            }
        }
        ctx.progress(totals.bytes, 0, totals.files + totals.dirs, 0);
    }
    Ok(())
}
