//! `trash.*` (api.md §4.5) and the `trash://` listing: the app trash on the
//! storage volumes (`crate::apptrash`).
use super::args::{nonempty_list, opt_i64};
use super::entry::{extension, kind_of};
use super::error::ApiError;
use super::location::TRASH_LOCATION;
use super::runtime::{Runtime, TaskCtx};
use crate::apptrash::TrashEntry;
use serde_json::{json, Value};

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "trash.list" => list(),
        "trash.restore" => restore(args),
        "trash.delete" => nonempty_list(args, "ids").map(|ids| delete_task(rt, ids, false)),
        "trash.empty" => Ok(delete_task(rt, Vec::new(), true)),
        "trash.purge" => purge(args),
        _ => return None,
    })
}

fn entries() -> Result<Vec<TrashEntry>, ApiError> {
    crate::apptrash::list().map_err(|error| ApiError::from(error).context("Papierkorb lesen"))
}

fn list() -> Result<Value, ApiError> {
    Ok(Value::Array(
        entries()?
            .iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "name": entry.name,
                    "originalLocation": entry.original.to_string_lossy(),
                    "deletedMs": entry.deleted_ms,
                    "size": entry.size,
                    "isDir": entry.is_dir,
                })
            })
            .collect(),
    ))
}

/// `fs.list` of `trash://`: the entries as read-only rows.
pub(crate) fn listing() -> Result<Value, ApiError> {
    let items = entries()?;
    let total: u64 = items.iter().map(|entry| entry.size).sum();
    let rows: Vec<Value> = items
        .iter()
        .map(|entry| {
            let ext = extension(&entry.name, entry.is_dir);
            json!({
                "name": entry.name,
                "location": format!("{TRASH_LOCATION}{}", entry.id),
                "isDir": entry.is_dir,
                "isLink": false,
                "size": entry.size,
                "mtimeMs": entry.deleted_ms,
                "hidden": false,
                "problem": Value::Null,
                "kind": kind_of(&ext, entry.is_dir),
                "ext": ext,
                "depth": 0,
                "hasChildren": false,
                "expanded": false,
            })
        })
        .collect();
    Ok(json!({
        "location": TRASH_LOCATION,
        "title": "Papierkorb",
        "crumbs": [{ "label": "Papierkorb", "location": TRASH_LOCATION }],
        "parent": Value::Null,
        "backend": "trash",
        "readOnly": true,
        "canTrash": false,
        "entries": rows,
        "totalBytes": total,
    }))
}

fn restore(args: &Value) -> Result<Value, ApiError> {
    let ids = nonempty_list(args, "ids")?;
    let originals: Vec<(String, std::path::PathBuf)> = entries()?
        .into_iter()
        .map(|entry| (entry.id, entry.original))
        .collect();
    let (mut restored, mut renamed, mut failed) = (0u64, 0u64, Vec::new());
    for id in &ids {
        match crate::apptrash::restore(id) {
            Ok(path) => {
                restored += 1;
                let original = originals
                    .iter()
                    .find(|(known, _)| known == id)
                    .map(|(_, original)| original);
                if original.is_some_and(|original| *original != path) {
                    renamed += 1;
                }
            }
            Err(error) => failed.push(ApiError::from(error).message),
        }
    }
    if restored == 0 {
        if let Some(first) = failed.first() {
            return Err(ApiError::internal(format!("Wiederherstellen: {first}")));
        }
    }
    Ok(json!({
        "restored": restored,
        "renamed": renamed,
        "failed": failed.len(),
        "errors": failed,
    }))
}

fn delete_task(rt: &Runtime, ids: Vec<String>, everything: bool) -> Value {
    let title = if everything {
        "Papierkorb leeren".to_string()
    } else {
        format!("Endgültig löschen: {} Element(e)", ids.len())
    };
    let id = rt.spawn_task("trash", title, move |ctx| run_delete(ctx, ids, everything));
    json!({ "taskId": id })
}

fn run_delete(ctx: &TaskCtx, ids: Vec<String>, everything: bool) -> Result<Value, ApiError> {
    let ids = if everything {
        entries()?.into_iter().map(|entry| entry.id).collect()
    } else {
        ids
    };
    let total = ids.len() as u64;
    let (mut deleted, mut failed) = (0u64, 0u64);
    for id in &ids {
        if ctx.cancelled() {
            return Err(ApiError::canceled());
        }
        match crate::apptrash::delete(id) {
            Ok(()) => deleted += 1,
            Err(error) => {
                failed += 1;
                ctx.error(id, &ApiError::from(error).message);
            }
        }
        ctx.progress(0, 0, deleted + failed, total);
    }
    let summary = json!({ "deleted": deleted, "failed": failed });
    if failed > 0 {
        ctx.set_failure_result(summary);
        return Err(ApiError::internal(format!(
            "{failed} von {total} Element(en) nicht gelöscht"
        )));
    }
    Ok(summary)
}

fn purge(args: &Value) -> Result<Value, ApiError> {
    let days = opt_i64(args, "olderThanDays")
        .filter(|days| *days >= 0)
        .ok_or_else(|| ApiError::invalid("Argument „olderThanDays“ fehlt"))?;
    let days = u32::try_from(days).unwrap_or(u32::MAX);
    let removed = crate::apptrash::purge_older_than(days)
        .map_err(|error| ApiError::from(error).context("Papierkorb aufräumen"))?;
    Ok(json!({ "removed": removed }))
}
