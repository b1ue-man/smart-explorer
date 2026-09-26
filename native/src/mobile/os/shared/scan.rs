//! `scan.*` (api.md §4.4): recursive filtered scans through the desktop
//! scanners (`scanner` locally, `rscan` remotely) with scan-time pruning
//! (`FilterRetention`), and windowed tree views of the result.
use super::args::{bool_or, filter_arg, opt_i64, pass_all, sort_arg, str_arg, u64_or, SortSpec};
use super::crumbs;
use super::entry::{entry_json, TreeInfo};
use super::error::ApiError;
use super::fs_list::reject_trash;
use super::location::Loc;
use super::runtime::{lock, Runtime, TaskCtx};
use super::scanview::{tree_rows, visible_rows, window, TreeRows};
use crate::filter::{CompiledFilter, FilterRetention};
use crate::scanner::{RetentionHandle, ScanMessage, ScanOpts};
use crate::types::{FileEntry, FilterDef};
use crossbeam_channel::RecvTimeoutError;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_VIEW_LIMIT: u64 = 500;
/// Scan results kept in memory; the oldest session goes first.
const MAX_SESSIONS: usize = 6;
const MAX_ISSUES: usize = 500;

#[derive(Default)]
pub(crate) struct ScanRegistry {
    sessions: Mutex<Vec<(String, Arc<ScanSession>)>>,
}

impl ScanRegistry {
    fn insert(&self, id: String, session: Arc<ScanSession>) {
        let mut sessions = lock(&self.sessions);
        sessions.push((id, session));
        while sessions.len() > MAX_SESSIONS {
            let (_, dropped) = sessions.remove(0);
            dropped.stop();
        }
    }

    fn get(&self, id: &str) -> Result<Arc<ScanSession>, ApiError> {
        lock(&self.sessions)
            .iter()
            .find(|(session_id, _)| session_id == id)
            .map(|(_, session)| session.clone())
            .ok_or_else(|| ApiError::not_found("Dieses Suchergebnis ist nicht mehr vorhanden"))
    }
}

struct ScanSession {
    base: Loc,
    filter: FilterDef,
    data: Mutex<ScanData>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Default)]
struct ScanData {
    entries: Vec<FileEntry>,
    revision: u64,
    matches: u64,
    scanned: u64,
    truncated: bool,
    issues: Vec<(String, String)>,
    tree: Option<(u64, SortSpec, TreeRows)>,
    last_view: Option<(u64, String)>,
}

impl ScanSession {
    fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "scan.validate" => validate(args),
        "scan.start" => start(rt, args),
        "scan.view" => view(rt, args),
        "scan.issues" => issues(rt, args),
        _ => return None,
    })
}

fn validate(args: &Value) -> Result<Value, ApiError> {
    let filter = filter_arg(args, "filter", true)?.unwrap_or_else(|| pass_all(true));
    let compiled = CompiledFilter::compile(&filter);
    Ok(json!({ "error": compiled.error() }))
}

fn start(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let base = Loc::parse(str_arg(args, "location")?)?;
    reject_trash(&base)?;
    let show_hidden = bool_or(args, "showHidden", false);
    let filter = filter_arg(args, "filter", show_hidden)?.unwrap_or_else(|| pass_all(show_hidden));
    if let Some(error) = CompiledFilter::compile(&filter).error() {
        return Err(ApiError::invalid(error.to_string()));
    }
    let title = format!("Suche in {}", crumbs::title(&base, &rt.volumes()));
    let session = Arc::new(ScanSession {
        base,
        filter,
        data: Mutex::new(ScanData::default()),
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let runtime = rt.clone();
    let worker_session = session.clone();
    let id = rt.spawn_task("scan", title, move |ctx| {
        run_scan(&runtime, ctx, &worker_session)
    });
    rt.inner.scans.insert(id.clone(), session);
    Ok(json!({ "taskId": id }))
}

fn run_scan(rt: &Runtime, ctx: &TaskCtx, session: &ScanSession) -> Result<Value, ApiError> {
    let root = session.base.path.clone();
    let (tx, rx) = crossbeam_channel::unbounded();
    let (max_depth, retention) = if crate::filter::filter_prunes(&session.filter) {
        let retention = FilterRetention::new(session.filter.clone(), root.clone());
        let max_depth = retention.max_depth();
        (max_depth, Some(Arc::new(retention) as RetentionHandle))
    } else {
        (None, None)
    };
    let handle = if session.base.is_local() {
        crate::scanner::start_scan(
            std::path::PathBuf::from(&root),
            ScanOpts {
                follow_symlinks: false,
                max_depth,
                retention,
            },
            tx,
        )
    } else {
        let (backend, path) = rt.resolve_live(&session.base)?;
        crate::rscan::start_scan_backend(backend, path, max_depth, retention, tx)
    };
    let compiled = CompiledFilter::compile(&session.filter);
    loop {
        if ctx.cancelled() || session.cancel.load(Ordering::Acquire) {
            handle.cancel.store(true, Ordering::Relaxed);
        }
        let message = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(message) => message,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let mut data = lock(&session.data);
        match message {
            ScanMessage::Entries(batch) => {
                data.matches += batch
                    .iter()
                    .filter(|entry| entry.depth > 0 && compiled.matches(entry, &root))
                    .count() as u64;
                data.entries.extend(batch);
                data.revision += 1;
            }
            ScanMessage::Progress(progress) => data.scanned = progress.scanned,
            ScanMessage::Error(detail) => push_issue(&mut data, String::new(), detail),
            ScanMessage::FailedPaths(paths) => {
                for (path, detail) in paths {
                    push_issue(&mut data, path, detail);
                }
            }
            ScanMessage::Done(progress) => {
                data.scanned = progress.scanned;
                data.truncated = handle.truncated.load(Ordering::Relaxed);
                data.revision += 1;
                ctx.progress(progress.bytes, 0, data.scanned, data.matches);
                break;
            }
        }
        ctx.progress(0, 0, data.scanned, data.matches);
    }
    let data = lock(&session.data);
    let mut notes = Vec::new();
    if data.truncated {
        notes.push("Scan-Grenze erreicht – Ergebnisse unvollständig".to_string());
    }
    if !data.issues.is_empty() {
        notes.push(format!("{} Ordner nicht lesbar", data.issues.len()));
    }
    if !notes.is_empty() {
        ctx.message(&notes.join(" · "));
    }
    Ok(json!({
        "matches": data.matches,
        "scanned": data.scanned,
        "truncated": data.truncated,
        "issues": data.issues.len(),
    }))
}

fn push_issue(data: &mut ScanData, path: String, detail: String) {
    if data.issues.len() < MAX_ISSUES && !data.issues.contains(&(path.clone(), detail.clone())) {
        data.issues.push((path, detail));
    }
}

fn view(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let session = rt.inner.scans.get(str_arg(args, "taskId")?)?;
    let sort = sort_arg(args, "sort")?;
    let collapsed: HashSet<String> = args
        .get("collapsed")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let offset = u64_or(args, "offset", 0) as usize;
    let limit = u64_or(args, "limit", MAX_VIEW_LIMIT).min(MAX_VIEW_LIMIT) as usize;
    let since = opt_i64(args, "sinceRevision");
    let view_key = format!("{sort:?}|{offset}|{limit}|{:?}", sorted(&collapsed));
    let mut data = lock(&session.data);
    let revision = data.revision;
    let counters = json!({
        "revision": revision,
        "matches": data.matches,
        "scanned": data.scanned,
        "truncated": data.truncated,
        "issues": data.issues.len(),
    });
    let unchanged = since == Some(revision as i64)
        && data.last_view.as_ref() == Some(&(revision, view_key.clone()));
    let fresh = matches!(&data.tree, Some((cached, cached_sort, _)) if *cached == revision && *cached_sort == sort);
    if !fresh {
        let tree = tree_rows(&data.entries, &session.base.path, &session.filter, sort);
        data.tree = Some((revision, sort, tree));
    }
    data.last_view = Some((revision, view_key));
    let data = &*data;
    let Some((_, _, tree)) = data.tree.as_ref() else {
        return Err(ApiError::internal("Baumansicht fehlt"));
    };
    let is_collapsed = |entry: &FileEntry| collapsed.contains(&session.base.at(&entry.path));
    let visible = visible_rows(tree, &data.entries, &is_collapsed);
    let rows: Vec<Value> = if unchanged {
        Vec::new()
    } else {
        window(&visible, offset, limit)
            .iter()
            .map(|&position| {
                let (index, depth) = tree.rows[position];
                let entry = &data.entries[index];
                entry_json(
                    &session.base,
                    entry,
                    TreeInfo {
                        depth,
                        has_children: tree.has_children[position],
                        expanded: entry.is_dir && !is_collapsed(entry),
                    },
                )
            })
            .collect()
    };
    let mut result = counters;
    result["unchanged"] = json!(unchanged);
    result["entries"] = Value::Array(rows);
    result["visibleTotal"] = json!(visible.len());
    Ok(result)
}

fn sorted(set: &HashSet<String>) -> Vec<&String> {
    let mut items: Vec<_> = set.iter().collect();
    items.sort();
    items
}

fn issues(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let session = rt.inner.scans.get(str_arg(args, "taskId")?)?;
    let data = lock(&session.data);
    let text = data
        .issues
        .iter()
        .map(|(path, detail)| {
            if path.is_empty() {
                detail.clone()
            } else {
                format!("{path}: {detail}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(json!({ "text": text }))
}
