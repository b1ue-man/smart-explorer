//! `replace` as a job: Notepad++'s Replace in Files below `target` (default:
//! the working root) over the replace core of `se replace`
//! (`vfs::content_replace`, plan docs/plaene/2026-11-28-ersetzen-masken-shell,
//! contract 2): the search runs where the files are, each changed file is
//! published whole and only when unchanged since it was read, its original
//! is kept encrypted in the user's replace journal (`undo`). The fields of
//! `grep` choose the matches, `files` or `hits` narrow them to files or hit
//! lines of an earlier `grep`; `dry_run` only previews. Changed files stream
//! into `changes` (with their new etag), previews into `previews`; the job
//! result carries the totals, the skipped files with their reason and the
//! `undo_id`. `history` lists the journal's runs, `undo` restores one.
//!
//! With an active mask list both streams are masked here (`PreMasked`):
//! paths, and the preview lines from the masked file (`grep_mask.rs`). The
//! `ranges` of `hits` then refer to masked lines and are not used: every
//! match of the named line is replaced (`ranges_ignored`).
use super::args::Args;
use super::grep_tool::mask::HitMasker;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::mask_layer_guard::{guard, Reach};
use super::session::Session;
use super::targets::{Resolved, Targets};
use crate::agent_proto::{
    ContentQuery, HitRef, ReplaceEvent, ReplaceProgram, ReplaceRequest, ReplaceSelection,
};
use crate::masks::MaskSet;
use crate::vfs::{ReplaceJournal, ReplaceSummary};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[path = "replace_tool_run.rs"]
mod run;

/// Skipped files and folders the result names one by one.
const SKIPPED_LISTED: usize = 50;

pub(super) const DESCRIPTION: &str = "Replaces text in files below the folder `target` (default: the working root), as Notepad++'s Replace in Files: every field of `grep` (`pattern`, `mode`, `match_case`, `whole_word`, `dot_matches_newline`, `filters`, `folders`, `exclude_folders`, `subfolders`, `hidden`, `gitignore`) plus `replacement` (normal: the text; extended: with Notepad++ escapes; regex: Notepad++'s substitution syntax `$0`/`$&`, `${n}`, `$+{name}`, `\\n`, `\\U…\\E`, `\\L`, `\\u`, `\\l`, conditions `?1yes:no`). `files` (relative paths) or `hits` (`[{rel, line, ranges?}]` from a `grep` with `format: json`) replace only there. `dry_run: true` changes nothing and streams `previews` (lines before and after). It runs where the files are; each changed file is published whole and only if it did not change since it was read (else skipped as `changed`); binary, read-only and too large files are skipped with their reason; encoding, BOM and line ends stay. The original of every changed file is kept encrypted: `undo: <undo_id>` restores the files of a run that are unchanged since, `history: true` lists the runs. Changed files stream into `changes` (`rel`, `replacements`, `size_before`, `size_after`, `etag`); the result carries `files_searched`, `files_changed`, `replacements`, `skipped`, `skipped_items`, `undo_id`. Use it instead of `sed -i` or editor-wide replace.";

pub(super) fn properties() -> Value {
    let flag = |text: &str| json!({"type": "boolean", "description": text});
    json!({
        "target": {"type": "string", "description": "Folder (or file) to replace in; relative to the working root; default: the working root."},
        "pattern": {"type": "string", "description": "What to find; never empty."},
        "replacement": {"type": "string", "description": "What replaces each match (may be empty)."},
        "mode": {"type": "string", "enum": ["normal", "extended", "regex"], "description": "normal (default), extended or regex."},
        "match_case": flag("Match case (default false)."),
        "whole_word": flag("Whole words only (normal and extended)."),
        "dot_matches_newline": flag("With regex: `.` also matches line ends."),
        "filters": {"type": "string", "description": "Notepad++ filters, space-separated; empty = every file."},
        "folders": {"type": "string", "description": "Regex on the relative folder path: replace only there."},
        "exclude_folders": {"type": "string", "description": "Regex: leave out matching folders with everything below."},
        "subfolders": flag("Also below the folder (default true)."),
        "hidden": flag("Also enter hidden folders (default false)."),
        "gitignore": flag("Leave out what .gitignore, .ignore and .git/info/exclude exclude (default false)."),
        "files": {"type": "array", "items": {"type": "string"}, "description": "Only these files (paths relative to `target`)."},
        "hits": {"type": "array", "items": {"type": "object", "properties": {
            "rel": {"type": "string"},
            "line": {"type": "integer", "minimum": 1},
            "ranges": {"type": "array", "items": {"type": "array", "items": {"type": "integer"}}},
            "size": {"type": "integer"},
            "mtime_ms": {"type": "integer"}
        }, "required": ["rel", "line"]}, "description": "Only these hit lines (from `grep` with `format: json`); `size`/`mtime_ms` default to the file as it is now."},
        "dry_run": flag("Only preview: change nothing, stream `previews`."),
        "undo": {"type": "string", "description": "The `undo_id` of a run: restore its files (those unchanged since)."},
        "history": flag("List the runs of the replace journal (newest first)."),
        "wait_seconds": {"type": "integer", "minimum": 0, "description": "Seconds to wait for the end of the job before answering (default: a short wait)."},
        "max_bytes": {"type": "integer", "minimum": 1024, "description": "Largest page of output in the answer."}
    })
}

/// One hit line of `hits` before its file state is known.
#[derive(Debug)]
pub(super) struct HitArg {
    rel: String,
    line: u64,
    ranges: Vec<(u32, u32)>,
    state: Option<(u64, i64)>,
}

/// Which matches a call replaces.
#[derive(Debug)]
pub(super) enum Choice {
    All,
    Files(Vec<String>),
    Hits(Vec<HitArg>),
}

/// What a call does.
#[derive(Debug)]
pub(super) enum Call {
    History,
    Undo(String),
    Replace {
        query: ContentQuery,
        replacement: String,
        choice: Choice,
        dry_run: bool,
    },
}

fn hits(value: &Value, masked: bool) -> Result<(Vec<HitArg>, bool), String> {
    let items = value
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| "`hits` must be a non-empty list".to_string())?;
    let mut ignored = false;
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let field = |name: &str| item.get(name);
        let rel = field("rel")
            .and_then(Value::as_str)
            .filter(|rel| !rel.is_empty())
            .ok_or_else(|| format!("hit {}: `rel` must be a non-empty string", index + 1))?;
        let line = field("line")
            .and_then(Value::as_u64)
            .filter(|line| *line >= 1)
            .ok_or_else(|| {
                format!(
                    "hit {}: `line` must be a line number (1 or more)",
                    index + 1
                )
            })?;
        let mut ranges = Vec::new();
        if let Some(list) = field("ranges").and_then(Value::as_array) {
            for range in list {
                let pair = range.as_array().filter(|pair| pair.len() == 2);
                let bound = |at: usize| {
                    pair.and_then(|pair| pair[at].as_u64())
                        .and_then(|bound| u32::try_from(bound).ok())
                };
                match (bound(0), bound(1)) {
                    (Some(start), Some(end)) if start <= end => ranges.push((start, end)),
                    _ => {
                        return Err(format!(
                            "hit {}: `ranges` are [start, end] pairs",
                            index + 1
                        ))
                    }
                }
            }
        }
        // Ranges from a masked grep point into masked lines.
        if masked && !ranges.is_empty() {
            ranges.clear();
            ignored = true;
        }
        let state = match (
            field("size").and_then(Value::as_u64),
            field("mtime_ms").and_then(Value::as_i64),
        ) {
            (Some(size), Some(mtime)) => Some((size, mtime)),
            _ => None,
        };
        out.push(HitArg {
            rel: rel.to_string(),
            line,
            ranges,
            state,
        });
    }
    Ok((out, ignored))
}

/// The call's action; invalid arguments fail before a job starts.
pub(super) fn call(args: &Args<'_>, masked: bool) -> Result<(Call, bool), String> {
    if args.flag("history")? {
        return Ok((Call::History, false));
    }
    if let Some(id) = args.opt_string("undo")? {
        return Ok((Call::Undo(id), false));
    }
    let query = super::grep_tool::query(args)?;
    let replacement = args
        .opt_string("replacement")?
        .ok_or_else(|| "missing required argument `replacement`".to_string())?;
    // An invalid pattern, filter, folder regex or replacement fails here.
    crate::agent_proto::prepare_content_search(&query)
        .map_err(|error| format!("invalid search: {error}"))?;
    ReplaceProgram::new(&query, &replacement)
        .map_err(|error| format!("invalid replacement: {error}"))?;
    let mut ignored = false;
    let choice = match (args.opt_strings("files")?, args.value("hits")) {
        (Some(_), Some(_)) => return Err("pass either `files` or `hits`, not both".to_string()),
        (Some(files), None) if files.is_empty() => {
            return Err("`files` must be a non-empty list".to_string())
        }
        (Some(files), None) => Choice::Files(files),
        (None, Some(value)) => {
            let (list, dropped) = hits(value, masked)?;
            ignored = dropped;
            Choice::Hits(list)
        }
        (None, None) => Choice::All,
    };
    Ok((
        Call::Replace {
            query,
            replacement,
            choice,
            dry_run: args.flag("dry_run")?,
        },
        ignored,
    ))
}

pub(super) struct ReplaceControl {
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) files: Arc<AtomicU64>,
    pub(super) changed: Arc<AtomicU64>,
    pub(super) replacements: Arc<AtomicU64>,
}

impl JobControl for ReplaceControl {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn progress(&self) -> Value {
        json!({
            "files_searched": self.files.load(Ordering::SeqCst),
            "files_changed": self.changed.load(Ordering::SeqCst),
            "replacements": self.replacements.load(Ordering::SeqCst),
        })
    }
}

pub(super) fn start(session: &Session, args: &Args<'_>) -> Result<Arc<Job>, String> {
    let masks = session.masks.set()?;
    let (call, ranges_ignored) = call(args, !masks.is_empty())?;
    let targets = session.targets.clone();
    let target = match &call {
        Call::Replace { .. } => {
            let target = session
                .targets
                .resolve(&super::file_tools::target_spec(args)?)?;
            guard(&masks, &target, Reach::Tree)?;
            Some(target)
        }
        Call::History | Call::Undo(_) => None,
    };
    let summary = match (&call, &target) {
        (Call::History, _) => "replace history".to_string(),
        (Call::Undo(id), _) => format!("replace undo {id}"),
        (Call::Replace { query, .. }, Some(target)) => {
            format!("replace {:?} in {}", query.pattern, target.spec)
        }
        (Call::Replace { .. }, None) => "replace".to_string(),
    };
    let job = session
        .jobs
        .create(JobKind::Replace, summary, &["changes", "previews"])?;
    let control = Arc::new(ReplaceControl {
        cancel: Arc::new(AtomicBool::new(false)),
        files: Arc::default(),
        changed: Arc::default(),
        replacements: Arc::default(),
    });
    job.set_control(control.clone());
    spawn_worker(job.clone(), move |job| match (call, target) {
        (Call::History, _) => history(),
        (Call::Undo(id), _) => undo(&id, &targets, &masks),
        (Call::Replace { .. }, None) => JobEnd {
            status: JobStatus::Failed,
            result: json!({}),
            error: Some("replace without a target".to_string()),
        },
        (
            Call::Replace {
                query,
                replacement,
                choice,
                dry_run,
            },
            Some(target),
        ) => {
            let request = ReplaceRequest {
                query,
                replacement,
                selection: ReplaceSelection::All,
                dry_run,
            };
            let masker = HitMasker::new(masks, target.backend.clone(), &target.path);
            let mut end = run::replace(
                job,
                (&target, masker, None),
                request,
                choice,
                &control,
                &targets,
            );
            if ranges_ignored {
                if let Value::Object(fields) = &mut end.result {
                    fields.insert("ranges_ignored".into(), json!(true));
                }
            }
            end
        }
    })?;
    Ok(job)
}

/// The runs of the journal, newest first.
fn history() -> JobEnd {
    let runs = ReplaceJournal::open_default().and_then(|journal| journal.history());
    match runs {
        Ok(runs) => JobEnd {
            status: JobStatus::Completed,
            result: json!({
                "runs": runs
                    .iter()
                    .map(|run| json!({
                        "undo_id": run.id,
                        "created_ms": run.created_ms,
                        "target": run.location,
                        "root": run.root,
                        "pattern": run.pattern,
                        "replacement": run.replacement,
                        "mode": run.mode.key(),
                        "files_changed": run.files_changed,
                        "replacements": run.replacements,
                        "state": run.state.key(),
                    }))
                    .collect::<Vec<_>>(),
            }),
            error: None,
        },
        Err(error) => JobEnd {
            status: JobStatus::Failed,
            result: json!({}),
            error: Some(format!("cannot read the replace journal: {error}")),
        },
    }
}

/// Restores the files of run `id`; its place is opened as this session
/// opens targets, and refused like a replacement there.
fn undo(id: &str, targets: &Targets, masks: &MaskSet) -> JobEnd {
    let resolve = |location: &str| -> crate::vfs::VfsResult<crate::vfs::BackendHandle> {
        let target: Resolved = targets.resolve(location).map_err(std::io::Error::other)?;
        guard(masks, &target, Reach::Tree).map_err(std::io::Error::other)?;
        Ok(target.backend.clone())
    };
    let outcome = ReplaceJournal::open_default().and_then(|journal| journal.undo(id, &resolve));
    match outcome {
        Ok(summary) => JobEnd {
            status: JobStatus::Completed,
            result: json!({
                "undo_id": id,
                "restored": summary.restored,
                "skipped_items": summary
                    .skipped
                    .iter()
                    .map(|(rel, reason)| json!({ "rel": rel, "reason": reason }))
                    .collect::<Vec<_>>(),
            }),
            error: None,
        },
        Err(error) => JobEnd {
            status: JobStatus::Failed,
            result: json!({ "undo_id": id }),
            error: Some(error.to_string()),
        },
    }
}

/// `choice` as the core's selection: hit lines get the size and time of
/// their file as `stat` reads it now when the call did not name them.
pub(super) fn selection(target: &Resolved, choice: Choice) -> ReplaceSelection {
    match choice {
        Choice::All => ReplaceSelection::All,
        Choice::Files(files) => ReplaceSelection::Files(files),
        Choice::Hits(list) => {
            let mut states: HashMap<String, (u64, i64)> = HashMap::new();
            let hits = list
                .into_iter()
                .map(|hit| {
                    let (size, mtime_ms) = hit.state.unwrap_or_else(|| {
                        *states.entry(hit.rel.clone()).or_insert_with(|| {
                            target
                                .backend
                                .stat(&crate::cli::ops::join(&target.path, &hit.rel))
                                .map_or((0, 0), |meta| (meta.size, meta.mtime_ms))
                        })
                    });
                    HitRef {
                        rel: hit.rel,
                        line: hit.line,
                        ranges: hit.ranges,
                        size,
                        mtime_ms,
                    }
                })
                .collect();
            ReplaceSelection::Hits(hits)
        }
    }
}

/// The totals of a finished run as the job result's fields.
pub(super) fn totals(summary: &ReplaceSummary) -> Vec<(&'static str, Value)> {
    vec![
        ("files_searched", json!(summary.files_searched)),
        ("files_changed", json!(summary.files_changed)),
        ("replacements", json!(summary.replacements)),
        ("skipped", json!(summary.skipped)),
        ("undo_id", json!(summary.undo_id)),
        ("canceled", json!(summary.canceled)),
    ]
}

/// A skipped file or folder as the result lists it.
pub(super) fn skipped_value(rel: &str, reason: &crate::agent_proto::ReplaceSkip) -> Value {
    json!({ "rel": rel, "reason": reason.key(), "detail": reason.message() })
}

pub(super) fn listed(skipped: &[Value]) -> bool {
    skipped.len() < SKIPPED_LISTED
}

/// The event's file, for the masker to forget it.
pub(super) fn event_rel(event: &ReplaceEvent) -> Option<&str> {
    match event {
        ReplaceEvent::File { rel, .. } | ReplaceEvent::Skipped { rel, .. } => Some(rel),
        ReplaceEvent::Progress { .. } | ReplaceEvent::Preview { .. } => None,
    }
}

#[cfg(test)]
#[path = "replace_tool_tests.rs"]
mod tests;
