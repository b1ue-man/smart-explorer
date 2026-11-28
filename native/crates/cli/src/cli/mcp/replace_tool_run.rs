//! One `replace` run of the MCP tool: the replace core on its own thread,
//! its events on the job's thread into the `changes` and `previews` streams
//! (masked with the active lists), the totals into the job result.
use super::{event_rel, listed, selection, skipped_value, totals, Choice, ReplaceControl};
use crate::agent_proto::{ReplaceEvent, ReplaceRequest};
use crate::cli::mcp::file_tools::etag;
use crate::cli::mcp::grep_tool::mask::HitMasker;
use crate::cli::mcp::jobs::{Job, JobEnd, JobStatus};
use crate::cli::mcp::targets::{Resolved, Targets};
use crate::vfs::ReplaceJournal;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Events the core may queue ahead of the job's thread.
const BACKLOG: usize = 1_024;

/// The etag of the changed file `rel` (the root itself for a one-file run).
fn changed_etag(target: &Resolved, rel: &str) -> Option<String> {
    let joined = crate::cli::ops::join(&target.path, rel);
    target
        .backend
        .stat(&joined)
        .or_else(|_| target.backend.stat(&target.path))
        .ok()
        .filter(|meta| !meta.is_dir)
        .map(|meta| etag(&meta))
}

/// The JSON line of one event for its stream, masked with `masker`.
fn record(
    target: &Resolved,
    masker: &mut Option<HitMasker>,
    event: &ReplaceEvent,
    dry_run: bool,
) -> Option<(&'static str, Vec<u8>)> {
    let path = |rel: &str, masker: &Option<HitMasker>| match masker {
        Some(masker) => masker.set().mask_path(rel),
        None => rel.to_string(),
    };
    let (stream, value) = match event {
        ReplaceEvent::File {
            rel,
            replacements,
            size_before,
            size_after,
        } => {
            let mut value = json!({
                "rel": path(rel, masker),
                "replacements": replacements,
                "size_before": size_before,
                "size_after": size_after,
            });
            if !dry_run {
                if let (Some(tag), Value::Object(fields)) = (changed_etag(target, rel), &mut value)
                {
                    fields.insert("etag".into(), json!(tag));
                }
            }
            ("changes", value)
        }
        ReplaceEvent::Preview {
            rel,
            line,
            before,
            after,
            truncated,
        } => {
            let (before, after) = match masker.as_mut() {
                Some(masker) => (
                    masker.block((rel, 0), *line, before, *truncated),
                    masker.set().mask(after).text,
                ),
                None => (before.clone(), after.clone()),
            };
            let mut value = json!({
                "rel": path(rel, masker),
                "line": line,
                "before": before,
                "after": after,
            });
            if let (true, Value::Object(fields)) = (*truncated, &mut value) {
                fields.insert("truncated".into(), json!(true));
            }
            ("previews", value)
        }
        ReplaceEvent::Skipped { .. } | ReplaceEvent::Progress { .. } => return None,
    };
    let mut line = value.to_string().into_bytes();
    line.push(b'\n');
    Some((stream, line))
}

/// The run of `request` over `target`; the originals go into `journal`, or
/// the user's journal when it is `None`.
pub(super) fn replace(
    job: &Arc<Job>,
    (target, mut masker, journal): (&Resolved, Option<HitMasker>, Option<&ReplaceJournal>),
    mut request: ReplaceRequest,
    choice: Choice,
    control: &ReplaceControl,
    targets: &Targets,
) -> JobEnd {
    request.selection = selection(target, choice);
    let mut result = json!({
        "root": target.path,
        "target": target.spec,
        "pattern": request.query.pattern,
        "mode": request.query.mode.key(),
        "dry_run": request.dry_run,
    });
    let mut skipped = Vec::new();
    let mut write_error = None;
    let (tx, rx) = crossbeam_channel::bounded::<ReplaceEvent>(BACKLOG);
    let backend = target.backend.clone();
    let (root, location) = (target.path.clone(), target.spec.clone());
    let cancel = control.cancel.clone();
    let outcome = std::thread::scope(|scope| {
        let request = &request;
        let worker = scope.spawn(move || {
            let emit = |event| tx.send(event).is_ok();
            match journal {
                Some(journal) => crate::vfs::content_replace_with(
                    journal, &*backend, &root, request, &location, &emit, &cancel,
                ),
                None => crate::vfs::content_replace(
                    &*backend, &root, request, &location, &emit, &cancel,
                ),
            }
        });
        for event in rx.iter() {
            if let ReplaceEvent::Progress {
                files_searched,
                files_changed,
                replacements,
            } = &event
            {
                control.files.store(*files_searched, Ordering::SeqCst);
                control.changed.store(*files_changed, Ordering::SeqCst);
                control.replacements.store(*replacements, Ordering::SeqCst);
                job.changed();
                continue;
            }
            if let ReplaceEvent::Skipped { rel, reason } = &event {
                if listed(&skipped) {
                    skipped.push(skipped_value(rel, reason));
                }
            }
            if write_error.is_none() {
                if let Some((stream, line)) = record(target, &mut masker, &event, request.dry_run) {
                    if let Err(error) = job.append(stream, &line) {
                        write_error = Some(error);
                        control.cancel.store(true, Ordering::SeqCst);
                    }
                }
            }
            if let (Some(masker), Some(rel)) = (masker.as_mut(), event_rel(&event)) {
                masker.forget(rel);
            }
        }
        worker
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("the replacement panicked")))
    });
    if let Value::Object(fields) = &mut result {
        fields.insert("skipped_items".into(), json!(skipped));
    }
    match (outcome, write_error) {
        (_, Some(error)) => JobEnd {
            status: JobStatus::Failed,
            result,
            error: Some(error),
        },
        (Ok(summary), None) => {
            if let Value::Object(fields) = &mut result {
                for (key, value) in totals(&summary) {
                    fields.insert(key.into(), value);
                }
            }
            JobEnd {
                status: if summary.canceled || job.cancel_requested() {
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
