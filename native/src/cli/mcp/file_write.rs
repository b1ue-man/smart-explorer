//! `write` and `edit`: new content is written to a private local file and
//! published through the `se put` pipeline (stage next to the destination,
//! verify, publish without exposing partial content). `if_match` binds the
//! change to the file state the agent read: the pipeline compares the
//! destination it planned with that etag and re-validates the planned state
//! right before publishing.
use super::args::Args;
use super::file_tools::{etag, read_range};
use super::response;
use super::session::Session;
use crate::cli::link_paths::resolve_destination;
use crate::cli::private_file::PrivateFile;
use crate::cli::target::Target;
use crate::cli::tree_guard::DestinationState;
use crate::cli::tree_observer::TreeObserver;
use crate::cli::tree_ops::TreeRequest;
use base64::Engine as _;
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::Write;

pub(super) fn write(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let target = session.targets.resolve(&args.string("target")?)?;
    let content = args
        .opt_string("content")?
        .ok_or_else(|| "missing required argument `content`".to_string())?;
    let bytes = match args.opt_string("encoding")?.as_deref() {
        None | Some("text") => content.into_bytes(),
        Some("base64") => base64::engine::general_purpose::STANDARD
            .decode(content.trim())
            .map_err(|error| format!("`content` is not valid base64: {error}"))?,
        Some(other) => return Err(format!("unknown encoding `{other}`; use text or base64")),
    };
    publish(
        &target,
        &bytes,
        args.flag("overwrite")?,
        args.opt_string("if_match")?.as_deref(),
    )
}

pub(super) fn edit(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let target = session.targets.resolve(&args.string("target")?)?;
    let edits = parse_edits(args.value("edits"))?;
    let meta = target
        .backend
        .stat(&target.path)
        .map_err(|error| error.to_string())?;
    if meta.is_symlink {
        return Err(format!(
            "{} is a link; `edit` changes files only at their real path (Smart Explorer does not write through links)",
            target.path
        ));
    }
    if meta.is_dir || meta.special {
        return Err(format!("{} is not a regular file", target.path));
    }
    let current = etag(&meta);
    if let Some(expected) = args.opt_string("if_match")? {
        if expected != current {
            return Ok(changed_since_read(&target.path, &current));
        }
    }
    let size = usize::try_from(meta.size).map_err(|_| "file is too large to edit".to_string())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| format!("{} is too large to edit in memory", target.path))?;
    bytes.extend(read_range(&target, &meta, 0, size)?);
    if bytes.len() != size {
        return Err(format!("{} changed while it was read", target.path));
    }
    let mut text = String::from_utf8(bytes).map_err(|_| {
        format!(
            "{} is not UTF-8 text; edit changes text files only",
            target.path
        )
    })?;
    let mut replacements = 0usize;
    for (index, edit) in edits.iter().enumerate() {
        let count = text.matches(edit.old_text.as_str()).count();
        if count == 0 {
            return Err(format!("edit {}: `old_text` was not found", index + 1));
        }
        if count > 1 && !edit.replace_all {
            return Err(format!(
                "edit {}: `old_text` occurs {count} times; include more context or set replace_all",
                index + 1
            ));
        }
        text = text.replace(edit.old_text.as_str(), &edit.new_text);
        replacements += count;
    }
    // Bound to the state just read, so a concurrent change is never lost.
    let result = publish(&target, text.as_bytes(), true, Some(&current))?;
    if result.is_error == Some(true) {
        return Ok(result);
    }
    let mut value = result.structured_content.unwrap_or(Value::Null);
    if let Value::Object(fields) = &mut value {
        fields.insert("replacements".into(), json!(replacements));
    }
    Ok(response::ok(value))
}

struct Edit {
    old_text: String,
    new_text: String,
    replace_all: bool,
}

fn parse_edits(value: Option<&Value>) -> Result<Vec<Edit>, String> {
    let items = value
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| "`edits` must be a non-empty list".to_string())?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let field = |name: &str| item.get(name).and_then(Value::as_str).map(str::to_string);
            let old_text = field("old_text")
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    format!("edit {}: `old_text` must be a non-empty string", index + 1)
                })?;
            let new_text = field("new_text")
                .ok_or_else(|| format!("edit {}: `new_text` must be a string", index + 1))?;
            Ok(Edit {
                old_text,
                new_text,
                replace_all: item
                    .get("replace_all")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn changed_since_read(path: &str, current: &str) -> CallToolResult {
    response::error_with(
        format!("{path} changed since it was read (etag mismatch); read it again"),
        json!({ "etag": current }),
    )
}

/// Publishes `bytes` at `target`. Without `overwrite` or `if_match` an
/// existing file is refused. Links in the existing part of a local path lead
/// where they point; a link as the file itself is refused.
fn publish(
    target: &Target,
    bytes: &[u8],
    overwrite: bool,
    if_match: Option<&str>,
) -> Result<CallToolResult, String> {
    let resolved = resolve_destination(target)?;
    let destination = resolved.as_ref().unwrap_or(target);
    let (source, mut file) = PrivateFile::create("smart-explorer-mcp-write-")?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| format!("cannot create a private staging file: {error}"))?;
    drop(file);
    let refused_etag = RefCell::new(None::<String>);
    let created = RefCell::new(false);
    let check = |state: &DestinationState| -> Result<(), String> {
        match (state, if_match) {
            (DestinationState::Directory(_), _) => Err(format!("{} is a directory", target.path)),
            (DestinationState::Missing, Some(_)) => Err(format!(
                "{} no longer exists; `if_match` refers to a file that was removed",
                target.path
            )),
            (DestinationState::Missing, None) => {
                *created.borrow_mut() = true;
                Ok(())
            }
            (DestinationState::File(meta), Some(expected)) if etag(meta) != expected => {
                *refused_etag.borrow_mut() = Some(etag(meta));
                Err("changed".to_string())
            }
            (DestinationState::File(_), None) if !overwrite => Err(format!(
                "{} exists; pass overwrite: true (or if_match with its etag) to replace it",
                target.path
            )),
            (DestinationState::File(_), _) => Ok(()),
        }
    };
    let observer = TreeObserver::default();
    let request = TreeRequest {
        resume: false,
        recursive: false,
        force: true,
        expected_root: None,
        destination_check: Some(&check),
        observer: &observer,
    };
    let outcome = source.publish(&*destination.backend, &destination.path, &request);
    if let Some(current) = refused_etag.borrow().as_deref() {
        return Ok(changed_since_read(&target.path, current));
    }
    outcome?;
    let meta = destination
        .backend
        .stat(&destination.path)
        .map_err(|error| format!("written, but cannot read back {}: {error}", target.path))?;
    let created = *created.borrow();
    Ok(response::ok(json!({
        "path": target.path,
        "bytes": bytes.len(),
        "created": created,
        "etag": etag(&meta),
    })))
}

#[cfg(test)]
mod tests {
    use super::parse_edits;
    use serde_json::json;

    #[test]
    fn mcp_task_edits_require_old_and_new_text() {
        let edits = parse_edits(Some(&json!([
            {"old_text": "a", "new_text": "b"},
            {"old_text": "c", "new_text": "", "replace_all": true}
        ])))
        .unwrap();
        assert_eq!(edits.len(), 2);
        assert!(edits[1].replace_all);
        assert!(parse_edits(Some(&json!([{"old_text": "", "new_text": "x"}]))).is_err());
        assert!(parse_edits(Some(&json!([]))).is_err());
        assert!(parse_edits(None).is_err());
    }
}
