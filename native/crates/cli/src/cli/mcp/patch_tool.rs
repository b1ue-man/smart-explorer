//! `apply_patch`: Codex's patch format (`*** Begin Patch` … `*** End Patch`
//! with `Add File`, `Delete File`, `Update File`, `Move to`, `@@` sections and
//! `*** End of File`; `patch_tool_parse.rs`) applied with Codex's context
//! matching (`patch_tool_apply.rs`) on any target se reaches (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C "Coding-Lücken").
//!
//! Paths are relative to `target` (default: the working root, so a patch
//! acts where `root` points, locally or on a remote); a path with its own
//! target spelling (`/abs`, `C:/…`, `name:path`) stands for itself. Unlike
//! Codex, which writes hunk after hunk, the whole patch is first computed
//! against the files as they are (several hunks on one file apply in order),
//! so a section that does not match changes nothing at all. Then each file
//! is published whole or not at all, as `write` publishes it, bound to the
//! state the patch was computed from; a file that changed meanwhile is
//! refused. The answer names every file with its action and new etag.
use super::args::Args;
use super::mask_layer_guard::{guard, Reach};
use super::response;
use super::session::Session;
use super::targets::{Resolved, Targets};
use crate::masks::MaskSet;
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use std::io::Read;

#[path = "patch_tool_apply.rs"]
mod apply;
#[path = "patch_tool_parse.rs"]
pub(super) mod patch_tool_parse;
#[path = "patch_tool_publish.rs"]
mod patch_tool_publish;
#[cfg(test)]
#[path = "patch_tool_tests.rs"]
mod tests;

use patch_tool_parse::{parse_patch, Hunk};
use patch_tool_publish::Expect;

pub(super) const DESCRIPTION: &str = "Applies a patch in Codex's apply_patch format to one or more files: `*** Begin Patch`, then per file `*** Add File: path` (every line `+text`), `*** Delete File: path`, or `*** Update File: path` (optionally `*** Move to: new/path`) with sections that start with `@@` or `@@ text of a line above the change` and lines ` context`, `-removed`, `+added`, optionally `*** End of File`, and finally `*** End Patch`. Context is matched as Codex matches it (exactly, then ignoring trailing and surrounding white space and typographic punctuation). Paths are relative to `target` (default: the working root, locally or on a remote); a full target spelling stands for itself. The whole patch is computed first, so a section that does not match changes nothing; then each file is published whole or not at all and only if it did not change since it was read. The answer lists every file with `action` (added, updated, deleted), `etag` and the moves. Use it instead of Edit/MultiEdit for changes across files, or Codex's own apply_patch.";

pub(super) fn properties() -> Value {
    json!({
        "patch": {"type": "string", "description": "The patch text, from `*** Begin Patch` to `*** End Patch`."},
        "target": {"type": "string", "description": "Folder the paths of the patch are relative to (default: the working root)."}
    })
}

/// A path of the patch with its own target spelling: absolute, a drive, or
/// a saved connection, Share peer or URL (`name:…`).
fn own_spelling(path: &str) -> bool {
    path.starts_with('/') || path.starts_with('\\') || path.contains(':')
}

/// The target spelling of `path` below `base`.
fn spelling(base: &str, path: &str) -> String {
    let path = path.trim();
    let path = path.strip_prefix("./").unwrap_or(path);
    if base.is_empty() || own_spelling(path) {
        path.to_string()
    } else {
        format!("{}/{path}", base.trim_end_matches('/'))
    }
}

/// One file the patch touches: its state when it was read and after the
/// hunks so far.
struct File {
    /// The path as the patch names it.
    named: String,
    target: Resolved,
    /// The etag read (`None`: the file did not exist).
    original: Option<String>,
    /// Content after the hunks so far (`None`: absent).
    current: Option<Vec<u8>>,
}

struct Plan<'s> {
    targets: &'s Targets,
    masks: &'s MaskSet,
    base: String,
    files: Vec<File>,
    moves: Vec<(String, String)>,
}

impl Plan<'_> {
    /// The index of the file `named`, read on first use.
    fn file(&mut self, named: &str) -> Result<usize, String> {
        let spec = spelling(&self.base, named);
        let target = self.targets.resolve(&spec)?;
        guard(self.masks, &target, Reach::Entry)?;
        let key = (target.backend.state_identity(), target.path.clone());
        if let Some(index) = self.files.iter().position(|file| {
            (
                file.target.backend.state_identity(),
                file.target.path.clone(),
            ) == key
        }) {
            return Ok(index);
        }
        let (original, current) = match target.backend.stat(&target.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
            Err(error) => return Err(self.targets.io_error(&target, error)),
            Ok(meta) if meta.is_symlink => {
                return Err(format!(
                    "{} is a link; apply_patch changes files only at their real path",
                    target.path
                ))
            }
            Ok(meta) if meta.is_dir || meta.special => {
                return Err(format!("{} is not a regular file", target.path))
            }
            Ok(meta) => {
                let mut bytes = Vec::new();
                let size = usize::try_from(meta.size)
                    .map_err(|_| format!("{} is too large to patch", target.path))?;
                bytes
                    .try_reserve_exact(size)
                    .map_err(|_| format!("{} is too large to patch in memory", target.path))?;
                target
                    .backend
                    .open_read_id(&target.path, meta.id.as_deref())
                    .and_then(|mut reader| reader.read_to_end(&mut bytes))
                    .map_err(|error| self.targets.io_error(&target, error))?;
                (Some(super::file_tools::etag(&meta)), Some(bytes))
            }
        };
        self.files.push(File {
            named: named.to_string(),
            target,
            original,
            current,
        });
        Ok(self.files.len() - 1)
    }

    fn hunk(&mut self, hunk: &Hunk) -> Result<(), String> {
        match hunk {
            Hunk::Add { path, contents } => {
                let index = self.file(path)?;
                self.files[index].current = Some(contents.clone().into_bytes());
            }
            Hunk::Delete { path } => {
                let index = self.file(path)?;
                if self.files[index].current.take().is_none() {
                    return Err(format!("Failed to delete file {path}: it does not exist"));
                }
            }
            Hunk::Update {
                path,
                move_path,
                chunks,
            } => {
                let index = self.file(path)?;
                let Some(bytes) = self.files[index].current.as_ref() else {
                    return Err(format!(
                        "Failed to read file to update {path}: it does not exist"
                    ));
                };
                let text = std::str::from_utf8(bytes).map_err(|_| {
                    format!("Failed to read file to update {path}: it is not UTF-8 text")
                })?;
                let updated = apply::updated_contents(text, path, chunks)?.into_bytes();
                match move_path {
                    Some(to) => {
                        let destination = self.file(to)?;
                        self.files[index].current = None;
                        self.files[destination].current = Some(updated);
                        self.moves.push((path.clone(), to.clone()));
                    }
                    None => self.files[index].current = Some(updated),
                }
            }
        }
        Ok(())
    }
}

/// What publishing did to one file.
fn action(file: &File) -> Option<&'static str> {
    match (&file.original, &file.current) {
        (None, Some(_)) => Some("added"),
        (Some(_), Some(_)) => Some("updated"),
        (Some(_), None) => Some("deleted"),
        (None, None) => None,
    }
}

fn entry(file: &File, action: &str, etag: Option<String>) -> Value {
    let mut value = json!({
        "path": file.target.path,
        "named": file.named,
        "action": action,
    });
    if let (Some(etag), Value::Object(fields)) = (etag, &mut value) {
        fields.insert("etag".into(), json!(etag));
    }
    value
}

pub(super) fn apply_patch(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let patch = args
        .opt_string("patch")?
        .ok_or_else(|| "missing required argument `patch`".to_string())?;
    let base = super::file_tools::target_spec(args)?;
    apply_with(&session.targets, &session.masks.set()?, &patch, &base)
}

/// `patch` applied relative to `base` with the session's `targets`; `masks`
/// keeps se's own data out of reach while a list is active.
pub(super) fn apply_with(
    targets: &Targets,
    masks: &MaskSet,
    patch: &str,
    base: &str,
) -> Result<CallToolResult, String> {
    let hunks = parse_patch(patch)?;
    if hunks.is_empty() {
        return Err("No files were modified.".to_string());
    }
    let mut plan = Plan {
        targets,
        masks,
        base: base.to_string(),
        files: Vec::new(),
        moves: Vec::new(),
    };
    for hunk in &hunks {
        plan.hunk(hunk)?;
    }
    let mut done = Vec::new();
    let mut summary = String::from("Success. Updated the following files:");
    for (position, file) in plan.files.iter().enumerate() {
        let Some(action) = action(file) else {
            continue;
        };
        let expect = match &file.original {
            Some(etag) => Expect::Etag(etag.clone()),
            None => Expect::Missing,
        };
        let outcome = match (&file.current, &expect) {
            (Some(bytes), _) => patch_tool_publish::publish(&file.target, bytes, &expect).map(Some),
            (None, Expect::Etag(etag)) => {
                patch_tool_publish::remove(&file.target, etag).map(|()| None)
            }
            (None, Expect::Missing) => Ok(None),
        };
        match outcome {
            Ok(etag) => {
                let letter = match action {
                    "added" => 'A',
                    "deleted" => 'D',
                    _ => 'M',
                };
                summary.push_str(&format!("\n{letter} {}", file.named));
                done.push(entry(file, action, etag));
            }
            Err(error) => {
                targets.after_failure(&file.target);
                let pending: Vec<Value> = plan.files[position + 1..]
                    .iter()
                    .filter_map(|file| action(file).map(|action| entry(file, action, None)))
                    .collect();
                return Ok(response::error_with(
                    format!(
                        "{}: {error}; {} file(s) were applied before, the rest was not",
                        file.named,
                        done.len()
                    ),
                    json!({
                        "applied": done,
                        "failed": entry(file, action, None),
                        "not_applied": pending,
                    }),
                ));
            }
        }
    }
    let moves: Vec<Value> = plan
        .moves
        .iter()
        .map(|(from, to)| json!({ "from": from, "to": to }))
        .collect();
    Ok(response::ok(json!({
        "files": done,
        "moves": moves,
        "summary": summary,
    })))
}
