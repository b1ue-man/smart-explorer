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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Finished scan results kept in memory. A new scan drops the oldest
/// finished session while there are more than this many; a running scan is
/// never dropped, so with every session running all of them stay.
const MAX_SESSIONS: usize = 6;

#[derive(Default)]
pub(crate) struct ScanRegistry {
    sessions: Mutex<Vec<(String, Arc<ScanSession>)>>,
}

impl ScanRegistry {
    fn insert(&self, id: String, session: Arc<ScanSession>) {
        let mut sessions = lock(&self.sessions);
        sessions.push((id, session));
        while sessions.len() > MAX_SESSIONS {
            let Some(oldest) = sessions.iter().position(|(_, s)| s.finished()) else {
                break;
            };
            sessions.remove(oldest);
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
    /// Set when the scan task ended, however it ended.
    finished: AtomicBool,
}

/// Marks the session finished when the scan task returns or unwinds.
struct FinishOnDrop<'a>(&'a ScanSession);

impl Drop for FinishOnDrop<'_> {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
    }
}

#[derive(Default)]
struct ScanData {
    /// Shared with the views that read them: a batch that arrives while a view
    /// still holds the entries copies them once (`Arc::make_mut`), so no view
    /// builds its tree under the lock.
    entries: Arc<Vec<FileEntry>>,
    revision: u64,
    matches: u64,
    scanned: u64,
    truncated: bool,
    issues: Vec<(String, String)>,
    /// The issues already listed, so each one appears once.
    issue_keys: HashSet<(String, String)>,
    tree: Option<(u64, SortSpec, Arc<TreeRows>)>,
    last_view: Option<(u64, String)>,
}

/// What one `scan.view` answer is made from, copied out of the lock.
struct Snapshot {
    revision: u64,
    matches: u64,
    scanned: u64,
    truncated: bool,
    issues: usize,
    /// The caller already holds this revision for this window.
    unchanged: bool,
    entries: Arc<Vec<FileEntry>>,
    /// The cached tree, when it belongs to this revision and sort.
    tree: Option<Arc<TreeRows>>,
}

impl ScanSession {
    fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// The state one answer is made from, taken in constant time under the lock:
    /// the entries are shared, the cached tree is shared when it matches.
    fn snapshot(&self, sort: SortSpec, view_key: &str, since: Option<i64>) -> Snapshot {
        let mut data = lock(&self.data);
        let revision = data.revision;
        let unchanged = since == Some(revision as i64)
            && data
                .last_view
                .as_ref()
                .is_some_and(|(seen, key)| *seen == revision && key == view_key);
        data.last_view = Some((revision, view_key.to_string()));
        let tree = match &data.tree {
            Some((cached, cached_sort, tree)) if *cached == revision && *cached_sort == sort => {
                Some(Arc::clone(tree))
            }
            _ => None,
        };
        Snapshot {
            revision,
            matches: data.matches,
            scanned: data.scanned,
            truncated: data.truncated,
            issues: data.issues.len(),
            unchanged,
            entries: Arc::clone(&data.entries),
            tree,
        }
    }

    /// Keeps `tree` as the cached tree unless a tree of a newer revision is kept.
    fn keep_tree(&self, revision: u64, sort: SortSpec, tree: Arc<TreeRows>) {
        let mut data = lock(&self.data);
        if data
            .tree
            .as_ref()
            .is_none_or(|(kept, _, _)| *kept <= revision)
        {
            data.tree = Some((revision, sort, tree));
        }
    }
}

/// Appends one batch of scan results; every batch is a new revision.
fn push_entries(data: &mut ScanData, batch: Vec<FileEntry>) {
    Arc::make_mut(&mut data.entries).extend(batch);
    data.revision += 1;
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
    let title = format!(
        "Suche in {}",
        crumbs::task_title(&base, &rt.volumes(), &super::domains::vault_label)
    );
    let private = base.is_vault();
    let session = Arc::new(ScanSession {
        base,
        filter,
        data: Mutex::new(ScanData::default()),
        finished: AtomicBool::new(false),
    });
    let runtime = rt.clone();
    let worker_session = session.clone();
    let id = rt.spawn_task("scan", title, move |ctx| {
        let _finish = FinishOnDrop(&worker_session);
        if private {
            ctx.keep_names_private();
        }
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
        if ctx.cancelled() {
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
                push_entries(&mut data, batch);
            }
            ScanMessage::Progress(progress) => data.scanned = progress.scanned,
            // Android reports the existing bounded issue list. The retained
            // local retry roots belong to the desktop partial-scan action.
            ScanMessage::DeniedRoots(_) => {}
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

/// Lists every distinct issue (R15c: no count; the former 500 dropped
/// findings without a technical cause).
fn push_issue(data: &mut ScanData, path: String, detail: String) {
    let issue = (path, detail);
    if data.issue_keys.insert(issue.clone()) {
        data.issues.push(issue);
    }
}

/// The rows a `scan.view` call asked for. The answer is one JNI string in
/// the app's own process, without a Binder or message size, so the caller's
/// amount holds; without `limit` every row from `offset` on.
fn view_limit(args: &Value) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map_or(usize::MAX, |limit| {
            usize::try_from(limit).unwrap_or(usize::MAX)
        })
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
    let limit = view_limit(args);
    let since = opt_i64(args, "sinceRevision");
    Ok(view_answer(
        &session, sort, &collapsed, offset, limit, since,
    ))
}

/// The `scan.view` answer for one window. The lock is held only for the
/// snapshot; the tree (when no cached one matches) and the rows are built from
/// the snapshot, so the scan consumer is not kept waiting.
fn view_answer(
    session: &ScanSession,
    sort: SortSpec,
    collapsed: &HashSet<String>,
    offset: usize,
    limit: usize,
    since: Option<i64>,
) -> Value {
    let view_key = format!("{sort:?}|{offset}|{limit}|{:?}", sorted(collapsed));
    let snapshot = session.snapshot(sort, &view_key, since);
    let tree = match &snapshot.tree {
        Some(tree) => Arc::clone(tree),
        None => {
            let tree = Arc::new(tree_rows(
                &snapshot.entries,
                &session.base.path,
                &session.filter,
                sort,
            ));
            session.keep_tree(snapshot.revision, sort, Arc::clone(&tree));
            tree
        }
    };
    let entries: &[FileEntry] = &snapshot.entries;
    let is_collapsed = |entry: &FileEntry| collapsed.contains(&session.base.at(&entry.path));
    let visible = visible_rows(&tree, entries, is_collapsed);
    let rows: Vec<Value> = if snapshot.unchanged {
        Vec::new()
    } else {
        window(&visible, offset, limit)
            .iter()
            .map(|&position| {
                let (index, depth) = tree.rows[position];
                let entry = &entries[index];
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
    let mut result = json!({
        "revision": snapshot.revision,
        "matches": snapshot.matches,
        "scanned": snapshot.scanned,
        "truncated": snapshot.truncated,
        "issues": snapshot.issues,
    });
    result["unchanged"] = json!(snapshot.unchanged);
    result["entries"] = Value::Array(rows);
    result["visibleTotal"] = json!(visible.len());
    result
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

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "scan_view_tests.rs"]
mod view_tests;
