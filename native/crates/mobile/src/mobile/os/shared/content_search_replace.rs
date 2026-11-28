//! `search.content.replace.*` (api.md §4.14): "In Dateien ersetzen" of the
//! app below a local or remote folder (plan docs/plaene/2026-11-28-ersetzen-
//! masken-shell, part C). `vfs::content_replace` searches where the files lie
//! and writes each file through the pooled backend of the location (the file
//! checked unchanged first, an encrypted backup of the original kept in the
//! replace journal); a dry run only counts. As the search, a run lives on a
//! task thread and its events wait in the session until `poll` fetches them.
//! `undo` restores a run from the journal, `history` lists the journal.
use super::super::args::{opt_str, str_arg, u64_or};
use super::super::crumbs;
use super::super::error::ApiError;
use super::super::fs_list::reject_trash;
use super::super::location::{join, Loc};
use super::super::runtime::{lock, Runtime, TaskCtx};
use super::super::text_json::byte_at_utf16;
use super::super::text_view::pick_bool;
use super::parse_query;
use crate::agent_proto::{HitRef, ReplaceEvent, ReplaceRequest, ReplaceSelection, ReplaceSummary};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Finished runs kept for `poll`; running ones always stay.
const MAX_SESSIONS: usize = 4;
/// Unfetched events after which the run waits for `poll`.
pub(crate) const MAX_BACKLOG: usize = 4096;
/// Events one `poll` returns at most.
const MAX_POLL_EVENTS: usize = 2000;
const CHANNEL_SLOTS: usize = 256;
const WAIT_STEP: Duration = Duration::from_millis(200);

#[derive(Default)]
pub(crate) struct ReplaceSessions {
    sessions: Mutex<Vec<(String, Arc<ReplaceSession>)>>,
}

impl ReplaceSessions {
    fn insert(&self, id: String, session: Arc<ReplaceSession>) {
        let mut sessions = lock(&self.sessions);
        sessions.push((id, session));
        while sessions.len() > MAX_SESSIONS {
            let Some(oldest) = sessions.iter().position(|(_, s)| lock(&s.data).done) else {
                break;
            };
            sessions.remove(oldest);
        }
    }

    pub(crate) fn get(&self, id: &str) -> Result<Arc<ReplaceSession>, ApiError> {
        lock(&self.sessions)
            .iter()
            .find(|(session_id, _)| session_id == id)
            .map(|(_, session)| session.clone())
            .ok_or_else(|| ApiError::not_found("Dieses Ersetzen ist nicht mehr vorhanden"))
    }
}

pub(crate) struct ReplaceSession {
    base: Loc,
    data: Mutex<ReplaceData>,
    drained: Condvar,
}

#[derive(Default)]
struct ReplaceData {
    events: VecDeque<Value>,
    first: u64,
    files: u64,
    replacements: u64,
    summary: Option<ReplaceSummary>,
    error: Option<String>,
    canceled: bool,
    done: bool,
}

impl ReplaceSession {
    fn new(base: Loc) -> Arc<Self> {
        Arc::new(Self {
            base,
            data: Mutex::new(ReplaceData::default()),
            drained: Condvar::new(),
        })
    }

    /// Queues one event; waits while `MAX_BACKLOG` events are unfetched.
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

    fn finish(&self, outcome: &Result<ReplaceSummary, ApiError>) {
        let mut data = lock(&self.data);
        match outcome {
            Ok(summary) => {
                data.files = summary.files_searched;
                data.replacements = summary.replacements;
                data.canceled = summary.canceled;
                data.summary = Some(summary.clone());
            }
            Err(error) if error.kind == "canceled" => data.canceled = true,
            Err(error) => data.error = Some(error.message.clone()),
        }
        data.done = true;
        self.drained.notify_all();
    }

    /// Events from `since` on (earlier ones are released) and the totals.
    fn poll(&self, since: u64) -> Value {
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
            "replacements": data.replacements,
            "done": data.done && all_fetched,
            "canceled": data.canceled,
            "error": data.error,
            "summary": data.summary.as_ref().map(summary_json),
        })
    }
}

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "search.content.replace.start" => start(rt, args),
        "search.content.replace.poll" => str_arg(args, "taskId")
            .and_then(|id| rt.inner.content.replaces.get(id))
            .map(|session| session.poll(u64_or(args, "since", 0))),
        "search.content.replace.cancel" => cancel(rt, args),
        "search.content.replace.undo" => super::content_search_undo::undo(rt, args),
        "search.content.replace.history" => super::content_search_undo::history(),
        _ => return None,
    })
}

/// A relative path below the searched folder, as the search reported it.
fn checked_rel(rel: &str) -> Result<String, ApiError> {
    let escapes = rel
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..");
    if rel.starts_with('/') || rel.contains('\\') || escapes {
        return Err(ApiError::invalid(format!(
            "Ungültiger relativer Pfad: {rel}"
        )));
    }
    Ok(rel.to_string())
}

/// One `hits` entry: its UTF-16 ranges become byte ranges of its `text`; a
/// cut line (`truncated`) names every match of that line (empty ranges).
pub(crate) fn parse_hit(value: &Value) -> Result<HitRef, ApiError> {
    let rel = checked_rel(str_arg(value, "rel")?)?;
    let line = value
        .get("line")
        .and_then(Value::as_u64)
        .filter(|line| *line > 0)
        .ok_or_else(|| ApiError::invalid("Treffer ohne gültige Zeilennummer"))?;
    let text = opt_str(value, "text").unwrap_or_default();
    let truncated = pick_bool(value, "truncated", "truncated", false);
    let mut ranges = Vec::new();
    if !truncated {
        for pair in value
            .get("ranges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let unit = |index: usize| {
                pair.get(index).and_then(Value::as_u64).ok_or_else(|| {
                    ApiError::invalid("Bereich eines Treffers ist kein [start, end]")
                })
            };
            let (start, end) = (unit(0)? as usize, unit(1)? as usize);
            let (start, end) = (
                byte_at_utf16(text, start),
                byte_at_utf16(text, end.max(start)),
            );
            ranges.push((start as u32, end as u32));
        }
    }
    let mtime_ms = value
        .get("mtimeMs")
        .or_else(|| value.get("mtime_ms"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Ok(HitRef {
        rel,
        line,
        ranges,
        size: u64_or(value, "size", 0),
        mtime_ms,
    })
}

/// The `ReplaceRequest` of a `start` call; every mistake is `invalid` at once.
pub(crate) fn parse_request(args: &Value) -> Result<ReplaceRequest, ApiError> {
    let query = parse_query(args)?;
    let replacement = args
        .get("replacement")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("„replacement“ fehlt (leer löscht die Treffer)"))?
        .to_string();
    let selection = match opt_str(args, "selection").unwrap_or("all") {
        "all" => ReplaceSelection::All,
        "hits" => {
            let hits = args
                .get("hits")
                .and_then(Value::as_array)
                .map(|list| list.iter().map(parse_hit).collect::<Result<Vec<_>, _>>())
                .transpose()?
                .unwrap_or_default();
            if hits.is_empty() {
                return Err(ApiError::invalid("Keine Treffer zum Ersetzen ausgewählt"));
            }
            ReplaceSelection::Hits(hits)
        }
        "files" => {
            let files = args
                .get("files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|rel| {
                    rel.as_str()
                        .ok_or_else(|| ApiError::invalid("„files“ enthält keinen Pfad"))
                        .and_then(checked_rel)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if files.is_empty() {
                return Err(ApiError::invalid("Keine Dateien zum Ersetzen ausgewählt"));
            }
            ReplaceSelection::Files(files)
        }
        other => return Err(ApiError::invalid(format!("Unbekannte Auswahl: {other}"))),
    };
    // A replacement text the mode cannot read fails here, before any task.
    crate::agent_proto::ReplaceProgram::new(&query, &replacement).map_err(ApiError::invalid)?;
    Ok(ReplaceRequest {
        query,
        replacement,
        selection,
        dry_run: pick_bool(args, "dryRun", "dry_run", false),
    })
}

fn start(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let base = Loc::parse(str_arg(args, "location")?)?;
    reject_trash(&base)?;
    let request = parse_request(args)?;
    let what = if request.dry_run {
        "Vorschau Ersetzen"
    } else {
        "Ersetzen in Dateien"
    };
    let title = format!(
        "{what}: {}",
        crumbs::task_title(&base, &rt.volumes(), &super::super::domains::vault_label)
    );
    let private = base.is_vault();
    let session = ReplaceSession::new(base);
    let runtime = rt.clone();
    let worker = session.clone();
    let id = rt.spawn_task("contentReplace", title, move |ctx| {
        if private {
            ctx.keep_names_private();
        }
        let outcome = run(&runtime, ctx, &worker, &request);
        worker.finish(&outcome);
        match outcome {
            // The session keeps the totals (and the undo id) of a cancelled
            // run; the task itself ends as cancelled.
            Ok(summary) if summary.canceled => Err(ApiError::canceled()),
            other => other.map(|summary| summary_json(&summary)),
        }
    });
    rt.inner.content.replaces.insert(id.clone(), session);
    Ok(json!({ "taskId": id }))
}

fn cancel(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = str_arg(args, "taskId")?;
    let session = rt.inner.content.replaces.get(id)?;
    rt.hub().with(|state| state.tasks.cancel(id));
    session.drained.notify_all();
    Ok(json!({}))
}

fn run(
    rt: &Runtime,
    ctx: &TaskCtx,
    session: &ReplaceSession,
    request: &ReplaceRequest,
) -> Result<ReplaceSummary, ApiError> {
    let cancel = ctx.cancel_flag();
    let base = &session.base;
    let (backend, path) = rt.resolve_live(base)?;
    let location = base.location();
    let (tx, rx) = crossbeam_channel::bounded(CHANNEL_SLOTS);
    let replaced = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let emit = move |event: ReplaceEvent| tx.send(event).is_ok();
            crate::vfs::content_replace(&*backend, &path, request, &location, &emit, &cancel)
        });
        collect(ctx, session, rx, &cancel);
        worker
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("Ersetzen abgebrochen (interner Fehler)")))
    });
    match replaced {
        Ok(summary) => {
            ctx.progress(0, 0, summary.files_searched, summary.replacements);
            ctx.message(&format!(
                "{} Ersetzungen in {} von {} Dateien",
                summary.replacements, summary.files_changed, summary.files_searched
            ));
            Ok(summary)
        }
        Err(_) if ctx.cancelled() => Err(ApiError::canceled()),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(ApiError::canceled()),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            Err(ApiError::invalid(error.to_string()))
        }
        Err(error) => Err(ApiError::from(error).context("Ersetzen in Dateien")),
    }
}

/// Moves the run's events into the session until it ends; on a cancel it
/// returns at once, which drops the receiver and ends the run after its
/// current file.
fn collect(
    ctx: &TaskCtx,
    session: &ReplaceSession,
    rx: Receiver<ReplaceEvent>,
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
        if let ReplaceEvent::Progress {
            files_searched,
            replacements,
            ..
        } = event
        {
            {
                let mut data = lock(&session.data);
                data.files = files_searched;
                data.replacements = replacements;
            }
            ctx.progress(0, 0, files_searched, replacements);
            continue;
        }
        if let Some(value) = event_json(&session.base, event) {
            if !session.push(value, cancel) {
                return;
            }
        }
    }
}

fn location_of(base: &Loc, rel: &str) -> String {
    if rel.is_empty() {
        base.location()
    } else {
        base.at(&join(&base.path, rel))
    }
}

/// One event of `poll`; progress and the before/after lines of a dry run
/// are not events (the totals carry the progress).
pub(crate) fn event_json(base: &Loc, event: ReplaceEvent) -> Option<Value> {
    Some(match event {
        ReplaceEvent::File {
            rel, replacements, ..
        } => json!({
            "type": "file",
            "location": location_of(base, &rel),
            "rel": rel,
            "replacements": replacements,
        }),
        ReplaceEvent::Skipped { rel, reason } => json!({
            "type": "skipped",
            "location": location_of(base, &rel),
            "rel": rel,
            "reason": reason.message(),
            "reasonKey": reason.key(),
        }),
        ReplaceEvent::Progress { .. } | ReplaceEvent::Preview { .. } => return None,
    })
}

pub(crate) fn summary_json(summary: &ReplaceSummary) -> Value {
    json!({
        "filesSearched": summary.files_searched,
        "filesChanged": summary.files_changed,
        "replacements": summary.replacements,
        "skipped": summary.skipped,
        "undoId": summary.undo_id,
        "canceled": summary.canceled,
    })
}
