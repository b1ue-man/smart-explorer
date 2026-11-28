//! `search.content.replace.undo` and `search.content.replace.history`
//! (api.md §4.14): the replace journal of the core keeps an encrypted copy
//! of every file a run overwrote; `undo` puts the copies of one run back
//! through the pooled backend of the location the run recorded, `history`
//! lists the runs, newest first.
use super::super::args::str_arg;
use super::super::error::ApiError;
use super::super::location::Loc;
use super::super::runtime::Runtime;
use crate::vfs::ReplaceJournal;
use serde_json::{json, Value};
use std::io;

/// The pooled backend of a location the journal recorded.
fn resolve(rt: &Runtime, location: &str) -> io::Result<crate::vfs::BackendHandle> {
    let loc = Loc::parse(location).map_err(|error| io::Error::other(error.message))?;
    rt.resolve_live(&loc)
        .map(|(backend, _)| backend)
        .map_err(|error| io::Error::other(error.message))
}

/// Restores the run `undoId`; returns once every file is handled: the
/// restored files and those left (changed since the run), with the reason.
pub(crate) fn undo(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = str_arg(args, "undoId")?;
    let journal = ReplaceJournal::open_default().map_err(ApiError::from)?;
    let resolve = |location: &str| resolve(rt, location);
    let summary = journal
        .undo(id, &resolve)
        .map_err(|error| ApiError::from(error).context("Rückgängig"))?;
    let skipped: Vec<Value> = summary
        .skipped
        .iter()
        .map(|(rel, reason)| json!({ "rel": rel, "reason": reason }))
        .collect();
    Ok(json!({ "restored": summary.restored, "skipped": skipped }))
}

/// The runs of the journal, newest first.
pub(crate) fn history() -> Result<Value, ApiError> {
    let journal = ReplaceJournal::open_default().map_err(ApiError::from)?;
    let entries: Vec<Value> = journal
        .history()
        .map_err(ApiError::from)?
        .into_iter()
        .map(|entry| {
            json!({
                "undoId": entry.id,
                "createdMs": entry.created_ms,
                "location": entry.location,
                "root": entry.root,
                "pattern": entry.pattern,
                "replacement": entry.replacement,
                "mode": entry.mode.key(),
                "filesChanged": entry.files_changed,
                "replacements": entry.replacements,
                "state": entry.state.key(),
            })
        })
        .collect();
    Ok(json!({ "entries": entries }))
}
