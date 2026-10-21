//! The comparison mode of vault sync jobs and „Tresor prüfen“ on Android
//! (VLT-M11, U31-7 to U31-9, U31-17), on `crate::vault` in this process.
//!
//! - `vault.compare {job, groups?}` (task) → head with `list` and `revision`.
//! - `vault.check {site}` (task) → `{site, label, list, revision, total}`.
//! - `vault.compareItems {list, revision, group?, offset?, limit?}` → the
//!   entries of one group page by page (`nextOffset`). The answer is a JNI
//!   string of this process: the caller's `limit` holds, without it every
//!   entry from `offset` on. Task results carry only the head, because a
//!   task result travels again with every `task` event.
//! - `vault.compareText {job, item}`, `vault.resolve {job, item, action,
//!   confirmed?}` (task), `vault.resolveAll {job, action, confirmed?}` (task),
//!   `vault.mergeApply {job, item, content}` (task).
//!
//! S2: entries hold cleartext paths for the screens with `FLAG_SECURE`; a
//! failed task keeps its text only in its result and reports a text without
//! names to the error log and the notification.
use serde_json::{json, Value};

use super::args::{invalid, opt_bool, opt_str, str_arg};
use super::vaults::label_of;
use crate::mobile::{ApiError, Runtime, TaskCtx};
use crate::vault::{CompareGroup, CompareItem, CompareView, ResolveReport, ResolveRequest};

/// What the error log and the notification say of a failed vault task.
const FAILED_TEXT: &str = "Die Aktion im Tresor ist fehlgeschlagen.";

pub(super) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "vault.compare" => compare(rt, args),
        "vault.check" => check(rt, args),
        "vault.compareItems" => items(args),
        "vault.compareText" => compare_text(args),
        "vault.resolve" => resolve(rt, args, false),
        "vault.resolveAll" => resolve(rt, args, true),
        "vault.mergeApply" => merge_apply(rt, args),
        _ => return None,
    })
}

/// The code of an I/O error of the comparison: a locked vault and a
/// comparison that is gone keep theirs.
fn io_error(error: &std::io::Error) -> ApiError {
    if crate::vault::is_compare_stale(error) {
        return ApiError::new(crate::vault::COMPARE_STALE_CODE, error.to_string());
    }
    if crate::vault::locked_site(error).is_some() {
        return ApiError::new(crate::vault::VAULT_LOCKED_CODE, error.to_string());
    }
    ApiError::from(std::io::Error::new(error.kind(), error.to_string()))
}

/// Fails a task: the full text and the code stay in the task result (for
/// the protected screen); the log and the notification get `FAILED_TEXT`.
fn fail(ctx: &TaskCtx, error: ApiError) -> ApiError {
    ctx.set_failure_result(json!({ "code": error.kind, "message": error.message }));
    ApiError::new(error.kind, public_text(&error))
}

/// The text of a failed vault task outside the protected screens: a
/// cancellation and a locked vault name no file; every other text may.
pub(super) fn public_text(error: &ApiError) -> String {
    match error.kind {
        "canceled" => error.message.clone(),
        kind if kind == crate::vault::VAULT_LOCKED_CODE => error.message.clone(),
        _ => FAILED_TEXT.to_string(),
    }
}

fn side(state: &crate::vault::SideState) -> Value {
    json!({
        "exists": state.exists,
        "size": state.size,
        "mtimeMs": state.mtime_ms,
        "location": state.location,
    })
}

pub(super) fn item_json(item: &CompareItem) -> Value {
    let actions: Vec<&str> = item.actions.iter().map(|action| action.key()).collect();
    json!({
        "item": item.id,
        "group": item.group.key(),
        "kind": item.kind.key(),
        "path": item.path,
        "a": side(&item.a),
        "b": side(&item.b),
        "actions": actions,
        "finding": item.finding,
        "note": item.note,
    })
}

fn head_json(view: &CompareView) -> Value {
    json!({
        "list": view.job,
        "job": view.job,
        "siteA": view.site_a,
        "labelA": view.label_a,
        "siteB": view.site_b,
        "labelB": view.label_b,
        "revision": view.revision,
        "counts": view.counts,
        "total": view.items.len(),
        "unavailable": view.unavailable,
    })
}

fn groups(args: &Value) -> Result<Vec<CompareGroup>, ApiError> {
    let Some(keys) = args.get("groups").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    keys.iter()
        .map(|key| {
            key.as_str()
                .and_then(CompareGroup::from_key)
                .ok_or_else(|| invalid(format!("Unbekannte Gruppe: {key}")))
        })
        .collect()
}

fn job_title(job: &str, verb: &str) -> String {
    match crate::vault::find_vault_job(job) {
        Ok(found) => format!("{verb}: {}", found.name),
        Err(_) => verb.to_string(),
    }
}

/// `vault.compare {job, groups?}` (task, T18).
fn compare(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job = str_arg(args, "job")?.to_string();
    let groups = groups(args)?;
    let title = job_title(&job, "Tresor-Sync vergleichen");
    let id = rt.spawn_task("vault", title, move |ctx| {
        let cancel = ctx.cancel_flag();
        crate::vault::compare_job(&job, &groups, &cancel)
            .map(|view| head_json(&view))
            .map_err(|error| fail(ctx, io_error(&error)))
    });
    Ok(json!({ "taskId": id }))
}

/// `vault.check {site}` (task, T21).
fn check(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let site = str_arg(args, "site")?.to_string();
    let title = format!("Tresor {} prüfen", label_of(&site));
    let id = rt.spawn_task("vault", title, move |ctx| {
        let cancel = ctx.cancel_flag();
        crate::vault::check_site(&site, &cancel)
            .map(|checked| {
                json!({
                    "site": checked.site,
                    "label": checked.label,
                    "list": format!("check:{}", checked.site),
                    "revision": checked.revision,
                    "total": checked.items.len(),
                })
            })
            .map_err(|error| fail(ctx, io_error(&error)))
    });
    Ok(json!({ "taskId": id }))
}

/// The entries of `listing` in `group` (all groups without one) from
/// `offset` on, at most `limit`; `nextOffset` names the rest.
pub(super) fn page(
    listing: &[CompareItem],
    group: Option<CompareGroup>,
    offset: usize,
    limit: Option<usize>,
) -> Value {
    let matching: Vec<&CompareItem> = listing
        .iter()
        .filter(|item| group.is_none_or(|group| item.group == group))
        .collect();
    let start = offset.min(matching.len());
    let end = limit.map_or(matching.len(), |limit| {
        start.saturating_add(limit).min(matching.len())
    });
    let items: Vec<Value> = matching[start..end]
        .iter()
        .map(|item| item_json(item))
        .collect();
    let next = (end < matching.len()).then_some(end);
    json!({ "items": items, "nextOffset": next, "total": matching.len() })
}

fn items(args: &Value) -> Result<Value, ApiError> {
    let list = str_arg(args, "list")?;
    let revision = args
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("„revision“ fehlt."))?;
    let group = match opt_str(args, "group") {
        None => None,
        Some(key) => Some(
            CompareGroup::from_key(key)
                .ok_or_else(|| invalid(format!("Unbekannte Gruppe: {key}")))?,
        ),
    };
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let limit = args.get("limit").and_then(Value::as_u64);
    let listing =
        crate::vault::compare_snapshot(list, revision).map_err(|error| io_error(&error))?;
    let as_index = |value: u64| usize::try_from(value).unwrap_or(usize::MAX);
    Ok(page(&listing, group, as_index(offset), limit.map(as_index)))
}

/// `vault.compareText {job, item}` → `{a, b, rows, crlf, finalNewline}`:
/// both versions of a text conflict and their aligned lines (`linemerge`,
/// fail-closed with its deadline) for the difference and the merge (T20).
fn compare_text(args: &Value) -> Result<Value, ApiError> {
    let (a, b) = crate::vault::compare_text_job(str_arg(args, "job")?, str_arg(args, "item")?)
        .map_err(|error| io_error(&error))?;
    let rows = crate::linemerge::rows(&a, &b).map_err(|error| invalid(error.to_string()))?;
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|row| json!({ "left": row.left, "right": row.right, "equal": row.equal }))
        .collect();
    Ok(json!({
        "a": a.as_str(),
        "b": b.as_str(),
        "rows": rows,
        "crlf": a.contains("\r\n"),
        "finalNewline": a.ends_with('\n'),
    }))
}

fn report_json(report: ResolveReport) -> Value {
    let failed: Vec<Value> = report
        .failed
        .into_iter()
        .map(|(item, error)| json!({ "item": item, "error": error }))
        .collect();
    json!({ "item": report.item, "resolved": report.resolved, "failed": failed })
}

/// `vault.resolve` and `vault.resolveAll` (tasks, T19).
fn resolve(rt: &Runtime, args: &Value, all: bool) -> Result<Value, ApiError> {
    let job = str_arg(args, "job")?.to_string();
    let item = if all {
        None
    } else {
        Some(str_arg(args, "item")?.to_string())
    };
    let action = str_arg(args, "action")?.to_string();
    let confirmed = opt_bool(args, "confirmed", false);
    let title = job_title(&job, "Tresor-Vergleich auflösen");
    let id = rt.spawn_task("vault", title, move |ctx| {
        let cancel = ctx.cancel_flag();
        let request = ResolveRequest {
            job: &job,
            item: item.as_deref(),
            action: &action,
            all,
            confirmed,
        };
        crate::vault::resolve_item(request, &cancel)
            .map(report_json)
            .map_err(|error| fail(ctx, io_error(&error)))
    });
    Ok(json!({ "taskId": id }))
}

/// `vault.mergeApply {job, item, content}` (task, T20).
fn merge_apply(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job = str_arg(args, "job")?.to_string();
    let item = str_arg(args, "item")?.to_string();
    let content = zeroize::Zeroizing::new(str_arg(args, "content")?.to_string());
    let title = job_title(&job, "Tresor-Konflikt zusammenführen");
    let id = rt.spawn_task("vault", title, move |ctx| {
        let cancel = ctx.cancel_flag();
        crate::vault::merge_apply_item(&job, &item, &content, &cancel)
            .map(|()| json!({ "item": item, "resolved": 1, "failed": [] }))
            .map_err(|error| fail(ctx, io_error(&error)))
    });
    Ok(json!({ "taskId": id }))
}

#[cfg(test)]
#[path = "vault_compare_tests.rs"]
mod tests;
