//! `search.content.*` (api.md §4.14): "Suchen in Dateien" of the app below a
//! local or remote folder, with the fields of Notepad++'s "Find in Files" and
//! the two folder-path regex fields (plan docs/plaene/2026-11-27-dateien-
//! suche-ansicht, B and E). The search runs where the files lie
//! (`vfs::content_search`: locally on every core, the SSH agent, a Share
//! host, else reading through the pooled backend) on a task thread; its
//! events wait in the session until `search.content.poll` fetches them. A
//! search whose events are not fetched waits once `MAX_BACKLOG` are queued,
//! so a search with very many hits never fills the memory.
//! `search.content.replace.*` („In Dateien ersetzen“) lives in
//! `content_search_replace.rs` and `content_search_undo.rs`.
use super::args::{opt_str, str_arg, u64_or};
use super::crumbs;
use super::error::ApiError;
use super::fs_list::reject_trash;
use super::location::{join, Loc};
use super::runtime::{lock, Runtime, TaskCtx};
use super::text_json::ranges_json;
use super::text_view::{pick_bool, text_query};
use crate::agent_proto::{ContentEvent, ContentQuery, ContentSummary, LineHit, SkipReason};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Finished searches kept for `poll`; running ones always stay.
const MAX_SESSIONS: usize = 4;
/// Unfetched events after which the search waits for `poll`.
pub(crate) const MAX_BACKLOG: usize = 4096;
/// Events one `poll` returns at most.
const MAX_POLL_EVENTS: usize = 2000;
/// Events in flight between the search and its collector.
const CHANNEL_SLOTS: usize = 256;
const WAIT_STEP: Duration = Duration::from_millis(200);

#[path = "content_search_replace.rs"]
mod content_search_replace;
#[cfg(test)]
#[path = "content_search_replace_tests.rs"]
mod content_search_replace_tests;
#[path = "content_search_undo.rs"]
mod content_search_undo;

#[derive(Default)]
pub(crate) struct ContentSearches {
    sessions: Mutex<Vec<(String, Arc<SearchSession>)>>,
    /// Running and recent replacements (`search.content.replace.*`).
    replaces: content_search_replace::ReplaceSessions,
}

impl ContentSearches {
    pub(crate) fn insert(&self, id: String, session: Arc<SearchSession>) {
        let mut sessions = lock(&self.sessions);
        sessions.push((id, session));
        while sessions.len() > MAX_SESSIONS {
            let Some(oldest) = sessions.iter().position(|(_, s)| s.finished()) else {
                break;
            };
            sessions.remove(oldest);
        }
    }

    pub(crate) fn get(&self, id: &str) -> Result<Arc<SearchSession>, ApiError> {
        lock(&self.sessions)
            .iter()
            .find(|(session_id, _)| session_id == id)
            .map(|(_, session)| session.clone())
            .ok_or_else(|| ApiError::not_found("Diese Suche ist nicht mehr vorhanden"))
    }
}

pub(crate) struct SearchSession {
    base: Loc,
    data: Mutex<SearchData>,
    /// Woken when `poll` fetched events or the search was canceled.
    drained: Condvar,
}

#[derive(Default)]
struct SearchData {
    events: VecDeque<Value>,
    /// Sequence number of `events[0]`.
    first: u64,
    files: u64,
    bytes: u64,
    hits: u64,
    summary: Option<ContentSummary>,
    error: Option<String>,
    canceled: bool,
    done: bool,
}

impl SearchSession {
    pub(crate) fn new(base: Loc) -> Arc<Self> {
        Arc::new(Self {
            base,
            data: Mutex::new(SearchData::default()),
            drained: Condvar::new(),
        })
    }

    fn finished(&self) -> bool {
        lock(&self.data).done
    }

    /// Queues one event; waits while `MAX_BACKLOG` events are unfetched.
    /// False once `cancel` is set (the event is dropped).
    fn push(&self, event: Value, cancel: &AtomicBool) -> bool {
        let mut data = lock(&self.data);
        while data.events.len() >= MAX_BACKLOG {
            if cancel.load(Ordering::Acquire) {
                return false;
            }
            data = match self.drained.wait_timeout(data, WAIT_STEP) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        data.events.push_back(event);
        true
    }

    fn finish(&self, outcome: &Result<ContentSummary, ApiError>) {
        let mut data = lock(&self.data);
        match outcome {
            Ok(summary) => {
                data.files = summary.files_searched;
                data.bytes = summary.bytes;
                data.hits = summary.hits;
                data.summary = Some(summary.clone());
            }
            Err(error) if error.kind == "canceled" => data.canceled = true,
            Err(error) => data.error = Some(error.message.clone()),
        }
        data.done = true;
        self.drained.notify_all();
    }

    /// Events from `since` on (earlier ones are released) and the totals.
    pub(crate) fn poll(&self, since: u64) -> Value {
        let mut data = lock(&self.data);
        while data.first < since && !data.events.is_empty() {
            data.events.pop_front();
            data.first += 1;
        }
        self.drained.notify_all();
        let start = since.saturating_sub(data.first) as usize;
        let events: Vec<Value> = data
            .events
            .iter()
            .skip(start)
            .take(MAX_POLL_EVENTS)
            .cloned()
            .collect();
        let next = data.first + (start + events.len()) as u64;
        let all_fetched = next >= data.first + data.events.len() as u64;
        json!({
            "events": events,
            "next": next,
            "files": data.files,
            "bytes": data.bytes,
            "hits": data.hits,
            "done": data.done && all_fetched,
            "canceled": data.canceled,
            "error": data.error,
            "summary": data.summary.as_ref().map(summary_json),
        })
    }
}

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    if method.starts_with("search.content.replace.") {
        return content_search_replace::handle(rt, method, args);
    }
    Some(match method {
        "search.content.start" => start(rt, args),
        "search.content.poll" => str_arg(args, "taskId")
            .and_then(|id| rt.inner.content.get(id))
            .map(|session| session.poll(u64_or(args, "since", 0))),
        "search.content.cancel" => cancel(rt, args),
        _ => return None,
    })
}

/// The `ContentQuery` of a `search.content.start` call; an invalid pattern
/// or folder expression is an `invalid` error before anything runs.
pub(crate) fn parse_query(args: &Value) -> Result<ContentQuery, ApiError> {
    let mut query = text_query(args)?;
    query.filters = opt_str(args, "filters").unwrap_or_default().to_string();
    query.folder_include = folder_regex(args, "folderInclude", "folder_include");
    query.folder_exclude = folder_regex(args, "folderExclude", "folder_exclude");
    query.subfolders = pick_bool(args, "subfolders", "subfolders", true);
    query.hidden = pick_bool(args, "hidden", "hidden", false);
    query.files_only = pick_bool(args, "filesOnly", "files_only", false);
    query.context_lines =
        pick_u64(args, "contextLines", "context_lines").min(u32::MAX as u64) as u32;
    query.max_hits = pick_u64(args, "maxHits", "max_hits");
    query.respect_ignore_files =
        pick_bool(args, "respectIgnoreFiles", "respect_ignore_files", false);
    crate::agent_proto::prepare_content_search(&query)
        .map_err(|error| ApiError::invalid(error.to_string()))?;
    Ok(query)
}

fn folder_regex(args: &Value, camel: &str, snake: &str) -> Option<String> {
    opt_str(args, camel)
        .or_else(|| opt_str(args, snake))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn pick_u64(args: &Value, camel: &str, snake: &str) -> u64 {
    args.get(camel)
        .or_else(|| args.get(snake))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn start(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let base = Loc::parse(str_arg(args, "location")?)?;
    reject_trash(&base)?;
    let query = parse_query(args)?;
    let title = format!(
        "Suche in Dateien: {}",
        crumbs::task_title(&base, &rt.volumes(), &super::domains::vault_label)
    );
    let private = base.is_vault();
    let session = SearchSession::new(base);
    let runtime = rt.clone();
    let worker = session.clone();
    let id = rt.spawn_task("contentSearch", title, move |ctx| {
        if private {
            ctx.keep_names_private();
        }
        let outcome = run(&runtime, ctx, &worker, &query);
        worker.finish(&outcome);
        outcome.map(|summary| summary_json(&summary))
    });
    rt.inner.content.insert(id.clone(), session);
    Ok(json!({ "taskId": id }))
}

fn cancel(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = str_arg(args, "taskId")?;
    let session = rt.inner.content.get(id)?;
    rt.hub().with(|state| state.tasks.cancel(id));
    session.drained.notify_all();
    Ok(json!({}))
}

fn run(
    rt: &Runtime,
    ctx: &TaskCtx,
    session: &SearchSession,
    query: &ContentQuery,
) -> Result<ContentSummary, ApiError> {
    let cancel = ctx.cancel_flag();
    let base = &session.base;
    let remote = if base.is_local() {
        None
    } else {
        Some(rt.resolve_live(base)?)
    };
    let (tx, rx) = crossbeam_channel::bounded(CHANNEL_SLOTS);
    let searched = std::thread::scope(|scope| {
        let searcher = scope.spawn(|| match &remote {
            None => crate::vfs::local_content_search(Path::new(&base.path), query, tx, &cancel)
                .map(|summary| summary.unwrap_or_default()),
            Some((backend, path)) => {
                crate::vfs::content_search(&**backend, path, query, tx, &cancel)
            }
        });
        collect(ctx, session, rx, &cancel);
        searcher
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("Suche abgebrochen (interner Fehler)")))
    });
    match searched {
        Ok(_) if ctx.cancelled() => Err(ApiError::canceled()),
        Ok(summary) => {
            ctx.progress(summary.bytes, 0, summary.files_searched, summary.hits);
            ctx.message(&format!(
                "{} Treffer in {} von {} Dateien",
                summary.hits, summary.files_matched, summary.files_searched
            ));
            Ok(summary)
        }
        Err(_) if ctx.cancelled() => Err(ApiError::canceled()),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(ApiError::canceled()),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            Err(ApiError::invalid(error.to_string()))
        }
        Err(error) => Err(ApiError::from(error).context("Suche in Dateien")),
    }
}

/// Moves the search's events into the session until the search ends; on a
/// cancel it returns at once, which drops the receiver and ends the search.
fn collect(
    ctx: &TaskCtx,
    session: &SearchSession,
    rx: Receiver<ContentEvent>,
    cancel: &AtomicBool,
) {
    loop {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        let event = match rx.recv_timeout(WAIT_STEP) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if let ContentEvent::Progress { files, bytes, hits } = event {
            {
                let mut data = lock(&session.data);
                data.files = files;
                data.bytes = bytes;
                data.hits = hits;
            }
            ctx.progress(bytes, 0, files, hits);
            continue;
        }
        if let Some(value) = event_json(&session.base, event) {
            if !session.push(value, cancel) {
                return;
            }
        }
    }
}

/// The location of `rel` below the searched folder.
fn location_of(base: &Loc, rel: &str) -> String {
    if rel.is_empty() {
        base.location()
    } else {
        base.at(&join(&base.path, rel))
    }
}

/// One event of `poll`; progress is not an event (the totals carry it).
pub(crate) fn event_json(base: &Loc, event: ContentEvent) -> Option<Value> {
    Some(match event {
        ContentEvent::Hits {
            rel,
            size,
            mtime_ms,
            lines,
            last,
        } => json!({
            "type": "hits",
            "location": location_of(base, &rel),
            "rel": rel,
            "size": size,
            "mtimeMs": mtime_ms,
            "last": last,
            "lines": lines.iter().map(line_json).collect::<Vec<_>>(),
        }),
        ContentEvent::Skipped { rel, reason } => {
            let (reason, detail) = match reason {
                SkipReason::Binary => ("binary", None),
                SkipReason::Unreadable(detail) => ("unreadable", Some(detail)),
                SkipReason::UnlistableFolder(detail) => ("unlistable", Some(detail)),
            };
            json!({
                "type": "skipped",
                "location": location_of(base, &rel),
                "rel": rel,
                "reason": reason,
                "detail": detail,
            })
        }
        ContentEvent::Progress { .. } => return None,
    })
}

fn line_json(line: &LineHit) -> Value {
    json!({
        "line": line.line,
        "text": line.text,
        "ranges": ranges_json(&line.text, &line.ranges),
        "truncated": line.truncated,
        "context": line.context,
    })
}

fn summary_json(summary: &ContentSummary) -> Value {
    json!({
        "filesSearched": summary.files_searched,
        "filesMatched": summary.files_matched,
        "hits": summary.hits,
        "bytes": summary.bytes,
        "skipped": summary.skipped,
        "limited": summary.limited,
    })
}

#[cfg(test)]
#[path = "content_search_tests.rs"]
mod tests;
