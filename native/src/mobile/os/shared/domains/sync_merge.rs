//! Line merge of a text conflict (`sync.mergeRows/mergeApply/mergeKeepBoth`),
//! as the desktop merge dialog: both sides get the same result, and the
//! baseline records it like a manual A/B resolution.
use serde_json::{json, Value};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::args::{canceled, invalid, str_arg};
use super::sync_conflicts::{
    apply_resolution, lookup, merge_len, recorded_state, set_merge, take_merge, MergeDraft,
    PairContext,
};
use super::sync_jobs::find_job;
use super::sync_run::{reserve_job, spawn_for_snapshot};
use crate::bisync::Conflict;
use crate::linemerge::TextShape;
use crate::mobile::{ApiError, Runtime};
use crate::vfs::Backend;

#[path = "sync_merge_recovery.rs"]
pub(super) mod recovery;

const MAX_TEXT: usize = 16 * 1024 * 1024;

fn check(cancel: &AtomicBool) -> Result<(), ApiError> {
    if cancel.load(Ordering::Acquire) {
        Err(canceled("Abgebrochen; bitte Dateizugriff prüfen."))
    } else {
        Ok(())
    }
}

fn original(
    backend: &dyn Backend,
    path: &str,
    signature: Option<crate::bisync::Sig>,
    cancel: &AtomicBool,
) -> Result<(String, (u64, i64)), ApiError> {
    check(cancel)?;
    let signature =
        signature.ok_or_else(|| invalid("Die Originaldatei fehlt. Bitte Konflikte neu prüfen."))?;
    if signature.size > MAX_TEXT as u64 {
        return Err(invalid(
            "Text-Zusammenführung ist auf 16 MiB pro Datei begrenzt.",
        ));
    }
    let mut reader = crate::vfs::open_read_regular(backend, path, None)
        .map_err(|e| super::args::io_error("Original lesen", e))?;
    let mut bytes = Vec::new();
    let mut block = [0; 32 * 1024];
    loop {
        check(cancel)?;
        let count = reader
            .read(&mut block)
            .map_err(|e| super::args::io_error("Original lesen", e))?;
        check(cancel)?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > MAX_TEXT {
            return Err(invalid(
                "Text-Zusammenführung ist auf 16 MiB pro Datei begrenzt.",
            ));
        }
        bytes.extend_from_slice(&block[..count]);
    }
    if bytes.contains(&0) {
        return Err(invalid("Keine Textdatei – bitte A/B behalten nutzen."));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| invalid("Keine UTF-8-Textdatei – bitte A/B behalten nutzen."))?;
    Ok((text, (signature.size, signature.mtime_ms)))
}

fn load_draft(
    pair: &PairContext,
    key: &crate::bisync::StateKey,
    conflict: &Conflict,
    cid: &str,
    cancel: &AtomicBool,
) -> Result<MergeDraft, ApiError> {
    if let Some(pending) = recovery::load(pair, key, &conflict.rel, cancel)? {
        return Ok(recovery::draft(cid, pending));
    }
    if conflict.duplicates.is_some() {
        return Err(invalid("Bitte zuerst eine konkrete Dateiversion auswählen; mehrere gleichnamige Dateien können nicht zeilenweise zusammengeführt werden."));
    }
    check(cancel)?;
    let paths = crate::bisync::recorded_original_paths_for_key(
        &*pair.a,
        &pair.root_a,
        &*pair.b,
        &pair.root_b,
        key,
        &conflict.rel,
    )
    .map_err(|e| super::args::io_error("Aufgezeichnete Originalpfade", e))?;
    let (text_a, state_a) = original(&*pair.a, &paths.path_a, conflict.a, cancel)?;
    let (text_b, state_b) = original(&*pair.b, &paths.path_b, conflict.b, cancel)?;
    check(cancel)?;
    let rows = crate::linemerge::rows(&text_a, &text_b)
        .map_err(|error| ApiError::new("unsupported", error.to_string()))?;
    Ok(MergeDraft {
        cid: cid.to_string(),
        rows,
        state_a,
        state_b,
        text_a,
        text_b,
        retry: false,
        pending: None,
    })
}

pub(super) fn rows(args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?;
    let cid = str_arg(args, "cid")?;
    let _read = reserve_job(job_id)?;
    let job = find_job(job_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let storage = crate::daemon::register_storage_run(&job.source, &job.target, &cancel);
    if storage.access_missing() {
        return Err(ApiError::new(
            "permission",
            "Dateizugriff fehlt: Zugriff auf alle Dateien erlauben.",
        ));
    }
    let (pair, conflict) = lookup(job_id, cid)?;
    let key = recorded_state(job_id)?;
    let draft = match recovery::load(&pair, &key, &conflict.rel, &cancel)? {
        Some(pending) => recovery::draft(cid, pending),
        None => match take_merge(job_id, cid) {
            Some(draft) if draft.retry => draft,
            _ => load_draft(&pair, &key, &conflict, cid, &cancel)?,
        },
    };
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
    let pending = draft.pending.as_ref().map(recovery::summary);
    set_merge(job_id, draft);
    Ok(json!({ "rows": rows, "pending": pending }))
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
    let key = recorded_state(&job_id)?;
    if merge_len(&job_id, &cid) != Some(choices.len()) {
        return Err(invalid("Zusammenführung bitte neu laden."));
    }
    let title = format!("Zusammenführen: {}", conflict.rel);
    let job = find_job(&job_id)?;
    let task = spawn_for_snapshot(rt, job, title, move |ctx| {
        let mut draft =
            take_merge(&job_id, &cid).ok_or_else(|| invalid("Zusammenführung bitte neu laden."))?;
        if draft.pending.is_some() {
            set_merge(&job_id, draft);
            return Err(invalid(
                "Bitte den gespeicherten Merge-Auftrag unverändert wiederholen.",
            ));
        }
        if draft.rows.len() != choices.len() {
            return Err(invalid("Zusammenführung bitte neu laden."));
        }
        for (row, (take_a, take_b)) in draft.rows.iter_mut().zip(choices) {
            if !row.equal {
                row.take_left = take_a;
                row.take_right = take_b;
            }
        }
        // Line endings and a final newline follow the inputs (the rows drop them).
        let shape = TextShape::merged(TextShape::of(&draft.text_a), TextShape::of(&draft.text_b));
        let merged = shape.apply(crate::linemerge::assemble_rows(&draft.rows));
        let result = merge(
            ctx,
            &job_id,
            &cid,
            &pair,
            &key,
            &conflict,
            crate::bisync::OriginalContent {
                signature: conflict.a,
                bytes: Some(draft.text_a.as_bytes()),
            },
            crate::bisync::OriginalContent {
                signature: conflict.b,
                bytes: Some(draft.text_b.as_bytes()),
            },
            crate::bisync::MergeChoice::Write(merged.as_bytes()),
        );
        if result.is_err() {
            draft.retry = true;
            set_merge(&job_id, draft);
        }
        result
    })?;
    Ok(json!({ "taskId": task }))
}

pub(super) fn keep_both(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?.to_string();
    let cid = str_arg(args, "cid")?.to_string();
    let (pair, conflict) = lookup(&job_id, &cid)?;
    let key = recorded_state(&job_id)?;
    let title = format!("Beide behalten: {}", conflict.rel);
    let job = find_job(&job_id)?;
    let task = spawn_for_snapshot(rt, job, title, move |ctx| {
        let cancel = ctx.cancel_flag();
        let mut draft = match take_merge(&job_id, &cid) {
            Some(draft) => draft,
            None => load_draft(&pair, &key, &conflict, &cid, &cancel)?,
        };
        if draft.pending.is_some() {
            set_merge(&job_id, draft);
            return Err(invalid(
                "Bitte den gespeicherten Merge-Auftrag unverändert wiederholen.",
            ));
        }
        let result = merge(
            ctx,
            &job_id,
            &cid,
            &pair,
            &key,
            &conflict,
            crate::bisync::OriginalContent {
                signature: conflict.a,
                bytes: Some(draft.text_a.as_bytes()),
            },
            crate::bisync::OriginalContent {
                signature: conflict.b,
                bytes: Some(draft.text_b.as_bytes()),
            },
            crate::bisync::MergeChoice::KeepBoth { keep_a: true },
        );
        if result.is_err() {
            draft.retry = true;
            set_merge(&job_id, draft);
        }
        result
    })?;
    Ok(json!({ "taskId": task }))
}

pub(super) fn retry(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?.to_string();
    let cid = str_arg(args, "cid")?.to_string();
    let (pair, conflict) = lookup(&job_id, &cid)?;
    let key = recorded_state(&job_id)?;
    let job = find_job(&job_id)?;
    let task = spawn_for_snapshot(
        rt,
        job,
        format!("Merge wiederholen: {}", conflict.rel),
        move |ctx| {
            let cancel = ctx.cancel_flag();
            let pending = recovery::load(&pair, &key, &conflict.rel, &cancel)?
            .ok_or_else(|| invalid("Der gespeicherte Auftrag ist nicht mehr offen. Bitte Konflikte neu prüfen."))?;
            let choice = match pending.choice {
                crate::bisync::RecordedMergeChoice::Write => {
                    crate::bisync::MergeChoice::Write(&pending.merged)
                }
                crate::bisync::RecordedMergeChoice::KeepBoth { keep_a } => {
                    crate::bisync::MergeChoice::KeepBoth { keep_a }
                }
            };
            merge(
                ctx,
                &job_id,
                &cid,
                &pair,
                &key,
                &pending.conflict,
                crate::bisync::OriginalContent {
                    signature: pending.conflict.a,
                    bytes: pending.original_a.as_deref(),
                },
                crate::bisync::OriginalContent {
                    signature: pending.conflict.b,
                    bytes: pending.original_b.as_deref(),
                },
                choice,
            )
        },
    )?;
    Ok(json!({ "taskId": task }))
}

fn merge(
    ctx: &crate::mobile::TaskCtx,
    job: &str,
    cid: &str,
    pair: &PairContext,
    key: &crate::bisync::StateKey,
    conflict: &Conflict,
    original_a: crate::bisync::OriginalContent<'_>,
    original_b: crate::bisync::OriginalContent<'_>,
    choice: crate::bisync::MergeChoice<'_>,
) -> Result<Value, ApiError> {
    let cancel = ctx.cancel_flag();
    check(&cancel)?;
    let keys = crate::bisync::pair_key_policy(&*pair.a, &pair.root_a, &*pair.b, &pair.root_b);
    let report = crate::bisync::merge_recorded_for_key(
        &*pair.a, &pair.root_a, &*pair.b, &pair.root_b, key, conflict,
        original_a, original_b,
        choice, &cancel, |phase| ctx.message(match phase {
            crate::bisync::ResolvePhase::Preparing => "Prüft beide Originale…",
            crate::bisync::ResolvePhase::BackingUp => "Sichert die Originale…",
            crate::bisync::ResolvePhase::Copying => "Schreibt die Zusammenführung…",
            crate::bisync::ResolvePhase::Deleting => "Übernimmt die Löschung…",
            crate::bisync::ResolvePhase::ReadingSignatures => "Speichert den bestätigten Zwischenstand…",
        }),
    ).map_err(|failure| {
        ctx.set_failure_result(report_json(&failure.partial, false));
        ApiError::new("conflict", format!("Zusammenführung unvollständig: {}. Bestätigte Teiländerungen bleiben gespeichert. Erneut laden verwendet dieselben Originale für einen sicheren Wiederanlauf.", failure.error))
    })?;
    // Baseline records keep literal spellings; compare their planning keys.
    let planned_key = keys.key(&conflict.rel);
    let mut matching = report
        .baseline
        .iter()
        .filter(|(rel, _)| keys.key(rel).as_ref() == planned_key.as_ref());
    let recorded = report.confirmed_a
        && report.confirmed_b
        && matching.next().map(|(_, entry)| entry) == Some(&(report.a, report.b))
        && matching.next().is_none();
    if !recorded {
        ctx.set_failure_result(report_json(&report, false));
        return Err(ApiError::new(
            "conflict",
            "Das vollständige Ergebnis ist nicht als Sync-Stand bestätigt. Bitte erneut laden.",
        ));
    }
    let remaining = apply_resolution(job, cid, &conflict.rel, (report.a, report.b))?;
    let mut value = report_json(&report, true);
    value["remaining"] = json!(remaining);
    Ok(value)
}

fn report_json(report: &crate::bisync::MergeReport, complete: bool) -> Value {
    json!({ "confirmedA": report.confirmed_a, "confirmedB": report.confirmed_b,
        "partial": !complete && (report.confirmed_a || report.confirmed_b
            || report.preserved.iter().any(|file| file.a.is_some() || file.b.is_some())),
        "baselineRecorded": complete, "reload": true, "retry": !complete,
        "preserved": report.preserved.iter().map(|file| json!({ "path": file.rel,
            "confirmedA": file.a.is_some(), "confirmedB": file.b.is_some() })).collect::<Vec<_>>(),
    })
}
