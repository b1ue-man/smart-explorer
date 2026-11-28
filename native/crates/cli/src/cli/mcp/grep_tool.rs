//! `grep` as a job: content search below a folder where the files are
//! (`vfs::content_search`), with Notepad++'s options and Claude Code's Grep
//! outputs (hit lines with numbers, file names, counts per file). The
//! records stream into the job's `matches` (text lines `rel:line:text`) or
//! `hits` (JSON lines with `rel`, `line`, `text`, `ranges`) output and page
//! with `hits_offset` like `search`; the job result carries the totals.
use super::args::Args;
use super::grep_records::{run_search, GrepOutput, Records};
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use super::targets::{Resolved, Targets};
use crate::agent_proto::{ContentEvent, ContentQuery, SkipReason, TextMode};
use crate::vfs::{Backend, Scheme};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Skipped files and folders the result names one by one.
const SKIPPED_LISTED: usize = 50;

pub(super) const DESCRIPTION: &str = "Searches file contents below the folder `target` (default: the working root), as Notepad++'s Find in Files: `mode` normal (the text), extended (Notepad++ escapes \\n, \\t, \\xHH) or regex (Rust regex plus lookaround, back references, \\R, \\< \\>); `match_case`, `whole_word`, `dot_matches_newline`; `filters` is Notepad++'s filter field (`*.rs *.toml`, `!*.min.js` leaves files out, `!\\target` a first-level folder, `!+\\node_modules` folders on every level); `folders`/`exclude_folders` are regexes on the folder path relative to `target`; `subfolders` (default true), `hidden` folders. Binary files are skipped, links never entered. It runs where the files are: locally, in the SSH agent, on the Share peer, otherwise by reading through the connection (`searched_at`). `output` content (default: `rel:line:text`, context lines `rel-line-text` with `context_lines`), files (names only) or count (`rel:count`); `format` json gives JSON lines with `rel`, `line`, `text`, byte `ranges`. Results page via `hits_offset`; the totals (`files_searched`, `files_matched`, `hits`, `skipped`, `limited`) come with the end of the job. An invalid pattern or folder regex fails at once. Use it instead of Grep, `grep` or `rg`.";

pub(super) fn properties(target: &str, wait: &str, max_bytes: &str) -> Value {
    json!({
        "target": {"type": "string", "description": target},
        "pattern": {"type": "string", "description": "What to find; never empty."},
        "mode": {"type": "string", "enum": ["normal", "extended", "regex"], "description": "normal (default), extended or regex."},
        "match_case": {"type": "boolean", "description": "Match case (default false)."},
        "whole_word": {"type": "boolean", "description": "Whole words only (normal and extended)."},
        "dot_matches_newline": {"type": "boolean", "description": "With regex: `.` also matches line ends."},
        "filters": {"type": "string", "description": "Notepad++ filters, space-separated; empty = every file."},
        "folders": {"type": "string", "description": "Regex: search only in folders whose relative path (`/`-separated, root = \"\") matches."},
        "exclude_folders": {"type": "string", "description": "Regex: leave out matching folders with everything below."},
        "subfolders": {"type": "boolean", "description": "Also search below the folder (default true)."},
        "hidden": {"type": "boolean", "description": "Also enter hidden folders (default false)."},
        "context_lines": {"type": "integer", "minimum": 0, "description": "Lines before and after each hit (output content)."},
        "output": {"type": "string", "enum": ["content", "files", "count"], "description": "content (default), files or count."},
        "format": {"type": "string", "enum": ["text", "json"], "description": "text (default, compact lines) or json (JSON lines with ranges)."},
        "max_results": {"type": "integer", "minimum": 0, "description": "Stop after this many hit lines; 0 or absent = all."},
        "wait_seconds": {"type": "integer", "minimum": 0, "description": wait},
        "max_bytes": {"type": "integer", "minimum": 1024, "description": max_bytes}
    })
}

pub(super) struct GrepControl {
    cancel: Arc<AtomicBool>,
    files: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    hits: Arc<AtomicU64>,
}

impl JobControl for GrepControl {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn progress(&self) -> Value {
        json!({
            "files_searched": self.files.load(Ordering::SeqCst),
            "bytes": self.bytes.load(Ordering::SeqCst),
            "hits": self.hits.load(Ordering::SeqCst),
        })
    }
}

/// The call's search: query, output and format.
pub(super) struct GrepRequest {
    pub(super) query: ContentQuery,
    pub(super) output: GrepOutput,
    pub(super) json: bool,
}

pub(super) fn request(args: &Args<'_>) -> Result<GrepRequest, String> {
    let mut query = ContentQuery::text(args.string("pattern")?);
    query.mode = match args.opt_string("mode")? {
        None => TextMode::Normal,
        Some(mode) => TextMode::parse(&mode)
            .ok_or_else(|| format!("unknown mode `{mode}`; use normal, extended or regex"))?,
    };
    let output = match args.opt_string("output")? {
        None => GrepOutput::Content,
        Some(output) => GrepOutput::parse(&output)
            .ok_or_else(|| format!("unknown output `{output}`; use content, files or count"))?,
    };
    let json = match args.opt_string("format")?.as_deref() {
        None | Some("text") => false,
        Some("json") => true,
        Some(other) => return Err(format!("unknown format `{other}`; use text or json")),
    };
    query.match_case = args.flag("match_case")?;
    query.whole_word = args.flag("whole_word")?;
    query.dot_matches_newline = args.flag("dot_matches_newline")?;
    query.filters = args.opt_string("filters")?.unwrap_or_default();
    query.folder_include = args.opt_string("folders")?.filter(|text| !text.is_empty());
    query.folder_exclude = args
        .opt_string("exclude_folders")?
        .filter(|text| !text.is_empty());
    query.subfolders = match args.value("subfolders") {
        None => true,
        Some(_) => args.flag("subfolders")?,
    };
    query.hidden = args.flag("hidden")?;
    let context = args.opt_u64("context_lines")?.unwrap_or(0);
    query.context_lines = if output == GrepOutput::Content {
        u32::try_from(context).map_err(|_| "`context_lines` is too large".to_string())?
    } else {
        0
    };
    query.max_hits = args.opt_u64("max_results")?.unwrap_or(0);
    query.files_only = output == GrepOutput::Files;
    // An invalid pattern, filter or folder regex fails the call itself,
    // with the search's own text.
    crate::agent_proto::prepare_content_search(&query)
        .map_err(|error| format!("invalid search: {error}"))?;
    Ok(GrepRequest {
        query,
        output,
        json,
    })
}

pub(super) fn start(session: &Session, args: &Args<'_>) -> Result<Arc<Job>, String> {
    let spec = super::file_tools::target_spec(args)?;
    let request = request(args)?;
    let target = session.targets.resolve(&spec)?;
    let stream = if request.json { "hits" } else { "matches" };
    let job = session.jobs.create(
        JobKind::Grep,
        format!(
            "grep {} for {:?}",
            if spec.is_empty() { "." } else { spec.as_str() },
            request.query.pattern
        ),
        &[stream],
    )?;
    let control = Arc::new(GrepControl {
        cancel: Arc::new(AtomicBool::new(false)),
        files: Arc::default(),
        bytes: Arc::default(),
        hits: Arc::default(),
    });
    job.set_control(control.clone());
    let targets = session.targets.clone();
    spawn_worker(job.clone(), move |job| {
        work(job, &target, &request, stream, &control, &targets)
    })?;
    Ok(job)
}

/// The search of one job, its records appended to `stream`.
fn work(
    job: &Arc<Job>,
    target: &Resolved,
    request: &GrepRequest,
    stream: &'static str,
    control: &GrepControl,
    targets: &Targets,
) -> JobEnd {
    let searched_at = searched_at(&*target.backend);
    let mut records = Records::new(
        request.output,
        request.json,
        false,
        request.query.context_lines > 0,
    );
    let mut skipped = Vec::new();
    let mut write_error = None;
    let mut buffer = Vec::new();
    let outcome = run_search(
        target.backend.clone(),
        target.path.clone(),
        request.query.clone(),
        control.cancel.clone(),
        &mut |event| match event {
            ContentEvent::Hits {
                rel, lines, last, ..
            } => {
                if write_error.is_some() {
                    return;
                }
                buffer.clear();
                records.hits(&rel, &lines, last, &mut buffer);
                if let Err(error) = job.append(stream, &buffer) {
                    write_error = Some(error);
                    control.cancel.store(true, Ordering::SeqCst);
                }
            }
            ContentEvent::Skipped { rel, reason } => {
                if skipped.len() < SKIPPED_LISTED {
                    skipped.push(skipped_value(&rel, &reason));
                }
            }
            ContentEvent::Progress { files, bytes, hits } => {
                control.files.store(files, Ordering::SeqCst);
                control.bytes.store(bytes, Ordering::SeqCst);
                control.hits.store(hits, Ordering::SeqCst);
                job.changed();
            }
        },
    );
    let mut result = json!({
        "root": target.path,
        "target": target.spec,
        "pattern": request.query.pattern,
        "mode": request.query.mode.key(),
        "output": request.output.key(),
        "searched_at": searched_at,
        "skipped_items": skipped,
    });
    match (outcome, write_error) {
        (_, Some(error)) => JobEnd {
            status: JobStatus::Failed,
            result,
            error: Some(error),
        },
        (Ok(summary), None) => {
            if let Value::Object(fields) = &mut result {
                fields.insert("files_searched".into(), json!(summary.files_searched));
                fields.insert("files_matched".into(), json!(summary.files_matched));
                fields.insert("hits".into(), json!(summary.hits));
                fields.insert("bytes".into(), json!(summary.bytes));
                fields.insert("skipped".into(), json!(summary.skipped));
                fields.insert("limited".into(), json!(summary.limited));
            }
            JobEnd {
                status: if job.cancel_requested() {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Completed
                },
                result,
                error: None,
            }
        }
        (Err(error), None) => {
            let cancelled = job.cancel_requested();
            let text = targets.io_error(target, error);
            JobEnd {
                status: if cancelled {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                },
                result,
                error: Some(text),
            }
        }
    }
}

fn skipped_value(rel: &str, reason: &SkipReason) -> Value {
    match reason {
        SkipReason::Binary => json!({ "rel": rel, "reason": "binary" }),
        SkipReason::Unreadable(detail) => {
            json!({ "rel": rel, "reason": "unreadable", "detail": detail })
        }
        SkipReason::UnlistableFolder(detail) => {
            json!({ "rel": rel, "reason": "unlistable_folder", "detail": detail })
        }
    }
}

/// Where the search runs, from the kind of connection: `local` on this
/// device, `host` on the Share peer, `agent` in the SSH agent (a saved SFTP
/// connection with its agent offers server-side search), else `reading`
/// through the connection. An older peer or agent without content search is
/// read through the connection as well; the search itself does not report it.
fn searched_at(backend: &dyn Backend) -> &'static str {
    match backend.scheme() {
        Scheme::Local => "local",
        Scheme::Peer => "host",
        Scheme::Sftp if backend.supports_search() => "agent",
        _ => "reading",
    }
}

#[cfg(test)]
#[path = "grep_tool_tests.rs"]
mod tests;
