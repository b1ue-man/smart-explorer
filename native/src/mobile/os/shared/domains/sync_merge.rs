//! Line merge of a text conflict (`sync.mergeRows/mergeApply/mergeKeepBoth`),
//! as the desktop merge dialog: both sides get the same result, and the
//! baseline records it like a manual A/B resolution.
use serde_json::{json, Value};

use super::args::{invalid, str_arg};
use super::sync_conflicts::{
    apply_resolution, lookup, merge_len, set_merge, take_merge, MergeDraft, PairContext,
};
use super::sync_run::spawn_for_job;
use crate::bisync::Conflict;
use crate::linemerge::TextShape;
use crate::mobile::{ApiError, Runtime};
use crate::vfs::remote_util::{conflict_rel_name, ep_join, read_text, sig_from, write_bytes};
use crate::vfs::Backend;

fn state(backend: &dyn Backend, path: &str) -> Result<(u64, i64), ApiError> {
    sig_from(backend, path)
        .map(|sig| (sig.size, sig.mtime_ms))
        .map_err(|error| ApiError::new("not_found", error))
}

fn load_draft(pair: &PairContext, conflict: &Conflict, cid: &str) -> Result<MergeDraft, ApiError> {
    let path_a = ep_join(&pair.root_a, &conflict.rel);
    let path_b = ep_join(&pair.root_b, &conflict.rel);
    let state_a = state(&*pair.a, &path_a)?;
    let state_b = state(&*pair.b, &path_b)?;
    let text_a = read_text(&*pair.a, &path_a)
        .map_err(|error| ApiError::new("unsupported", format!("Seite A: {error}")))?;
    let text_b = read_text(&*pair.b, &path_b)
        .map_err(|error| ApiError::new("unsupported", format!("Seite B: {error}")))?;
    let rows = crate::linemerge::rows(&text_a, &text_b)
        .map_err(|error| ApiError::new("unsupported", error.to_string()))?;
    Ok(MergeDraft {
        cid: cid.to_string(),
        rows,
        state_a,
        state_b,
        text_a,
        text_b,
    })
}

/// Writing must not replace a version the merge was not computed from.
fn ensure_unchanged(pair: &PairContext, rel: &str, draft: &MergeDraft) -> Result<(), ApiError> {
    let now_a = state(&*pair.a, &ep_join(&pair.root_a, rel))?;
    let now_b = state(&*pair.b, &ep_join(&pair.root_b, rel))?;
    if draft.state_a != now_a || draft.state_b != now_b {
        return Err(ApiError::new(
            "conflict",
            "Die Datei wurde seit dem Laden geändert – bitte die Zusammenführung neu laden.",
        ));
    }
    Ok(())
}

fn write_both(pair: &PairContext, rel: &str, text: &str) -> Result<(), ApiError> {
    write_bytes(&*pair.a, &ep_join(&pair.root_a, rel), text.as_bytes())
        .map_err(|error| ApiError::new("internal", format!("Seite A schreiben: {error}")))?;
    write_bytes(&*pair.b, &ep_join(&pair.root_b, rel), text.as_bytes())
        .map_err(|error| ApiError::new("internal", format!("Seite B schreiben: {error}")))
}

fn finish(job_id: &str, cid: &str, pair: &PairContext, rel: &str) -> Result<Value, ApiError> {
    let sig_a = sig_from(&*pair.a, &ep_join(&pair.root_a, rel))
        .map_err(|error| ApiError::new("internal", error))?;
    let sig_b = sig_from(&*pair.b, &ep_join(&pair.root_b, rel))
        .map_err(|error| ApiError::new("internal", error))?;
    let remaining = apply_resolution(job_id, cid, rel, (Some(sig_a), Some(sig_b)))?;
    Ok(json!({ "remaining": remaining }))
}

pub(super) fn rows(args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?;
    let cid = str_arg(args, "cid")?;
    let (pair, conflict) = lookup(job_id, cid)?;
    let draft = load_draft(&pair, &conflict, cid)?;
    let rows: Vec<Value> = draft
        .rows
        .iter()
        .map(|row| {
            json!({
                "a": row.left,
                "b": row.right,
                "equal": row.equal,
                "takeA": row.take_left,
                "takeB": row.take_right,
            })
        })
        .collect();
    set_merge(job_id, draft);
    Ok(json!({ "rows": rows }))
}

fn choices(args: &Value) -> Result<Vec<(bool, bool)>, ApiError> {
    let rows = args
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Parameter „rows“ fehlt."))?;
    Ok(rows
        .iter()
        .map(|row| {
            let flag = |key: &str| row.get(key).and_then(Value::as_bool).unwrap_or(false);
            (flag("takeA"), flag("takeB"))
        })
        .collect())
}

pub(super) fn apply(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?.to_string();
    let cid = str_arg(args, "cid")?.to_string();
    let choices = choices(args)?;
    let (pair, conflict) = lookup(&job_id, &cid)?;
    if merge_len(&job_id, &cid) != Some(choices.len()) {
        return Err(invalid("Zusammenführung bitte neu laden."));
    }
    let title = format!("Zusammenführen: {}", conflict.rel);
    let owner = job_id.clone();
    let task = spawn_for_job(rt, &owner, title, move |_ctx| {
        let mut draft =
            take_merge(&job_id, &cid).ok_or_else(|| invalid("Zusammenführung bitte neu laden."))?;
        if draft.rows.len() != choices.len() {
            return Err(invalid("Zusammenführung bitte neu laden."));
        }
        for (row, (take_a, take_b)) in draft.rows.iter_mut().zip(choices) {
            if !row.equal {
                row.take_left = take_a;
                row.take_right = take_b;
            }
        }
        ensure_unchanged(&pair, &conflict.rel, &draft)?;
        // Line endings and a final newline follow the inputs (the rows drop them).
        let shape = TextShape::merged(TextShape::of(&draft.text_a), TextShape::of(&draft.text_b));
        let merged = shape.apply(crate::linemerge::assemble_rows(&draft.rows));
        write_both(&pair, &conflict.rel, &merged)?;
        finish(&job_id, &cid, &pair, &conflict.rel)
    })?;
    Ok(json!({ "taskId": task }))
}

pub(super) fn keep_both(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?.to_string();
    let cid = str_arg(args, "cid")?.to_string();
    let (pair, conflict) = lookup(&job_id, &cid)?;
    let title = format!("Beide behalten: {}", conflict.rel);
    let owner = job_id.clone();
    let task = spawn_for_job(rt, &owner, title, move |_ctx| {
        let draft = match take_merge(&job_id, &cid) {
            Some(draft) => draft,
            None => load_draft(&pair, &conflict, &cid)?,
        };
        ensure_unchanged(&pair, &conflict.rel, &draft)?;
        // Both versions stay as they were read, byte for byte.
        let copy_rel = conflict_rel_name(&conflict.rel);
        write_both(&pair, &conflict.rel, &draft.text_a)?;
        write_both(&pair, &copy_rel, &draft.text_b)?;
        finish(&job_id, &cid, &pair, &conflict.rel)
    })?;
    Ok(json!({ "taskId": task }))
}
