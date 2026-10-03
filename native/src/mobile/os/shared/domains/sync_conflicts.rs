//! Conflict context per job (in memory, like the desktop session): the pair
//! of the last run or dry run, its open conflicts, and resolved entries whose
//! baseline update is not saved yet. Saving merges the resolved entries into
//! the stored baseline, so a newer background run's baseline is kept.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{json, Value};

use super::args::{canceled, invalid, str_arg};
use super::sync_jobs::find_job;
use super::sync_run::{job_owned, open_pair_for, report_errors, run_bisync, spawn_for_snapshot};
use crate::bisync::{Baseline, Conflict, ResolvePhase, Sig};
use crate::linemerge::Row;
use crate::mobile::{ApiError, Runtime};
use crate::vfs::BackendHandle;

/// Largest text side the line merge accepts through its regular-file reader.
const MAX_MERGE_BYTES: u64 = 16 * 1024 * 1024;

pub(super) struct PairContext {
    pub a: BackendHandle,
    pub root_a: String,
    pub b: BackendHandle,
    pub root_b: String,
    pub pair: String,
}

/// Loaded line-merge rows of one conflict plus the file states and texts they
/// came from (the rows lose line endings; "keep both" writes the texts as read).
pub(super) struct MergeDraft {
    pub cid: String,
    pub rows: Vec<Row>,
    pub state_a: (u64, i64),
    pub state_b: (u64, i64),
    pub text_a: String,
    pub text_b: String,
    pub retry: bool,
    pub pending: Option<crate::bisync::PendingMerge>,
}

struct JobConflicts {
    pair: Arc<PairContext>,
    items: Vec<(String, Conflict)>,
    resolved: Baseline,
    merge: Option<MergeDraft>,
    state: Option<crate::bisync::StateKey>,
    pending: BTreeSet<String>,
}

static CONFLICTS: Mutex<BTreeMap<String, JobConflicts>> = Mutex::new(BTreeMap::new());
/// Serializes load-merge-save of baselines.
static SAVING: Mutex<()> = Mutex::new(());
static NEXT_CID: AtomicU64 = AtomicU64::new(1);

fn with_state<T>(work: impl FnOnce(&mut BTreeMap<String, JobConflicts>) -> T) -> T {
    let mut guard: MutexGuard<'_, _> = CONFLICTS.lock().unwrap_or_else(PoisonError::into_inner);
    work(&mut guard)
}

/// Replaces the job's context with the result of a run or dry run.
pub(super) fn store_run(job_id: &str, pair: PairContext, conflicts: Vec<Conflict>) {
    store_recorded_run(job_id, pair, conflicts, None);
}

pub(super) fn store_recorded_run(job_id: &str, pair: PairContext, conflicts: Vec<Conflict>, state: Option<crate::bisync::StateKey>) {
    store_with_pending(job_id, pair, conflicts, state, Vec::new());
}

fn store_with_pending(job_id: &str, pair: PairContext, mut conflicts: Vec<Conflict>,
    state: Option<crate::bisync::StateKey>, pending: Vec<Conflict>) -> usize {
    let pending_names = pending.iter().map(|conflict| conflict.rel.clone()).collect();
    if !pending.is_empty() {
        let keys = crate::bisync::pair_key_policy(&*pair.a, &pair.root_a, &*pair.b, &pair.root_b);
        let protected: BTreeSet<_> = pending.iter().map(|conflict| keys.key(&conflict.rel).into_owned()).collect();
        conflicts.retain(|conflict| !protected.contains(keys.key(&conflict.rel).as_ref()));
        conflicts.extend(pending);
    }
    let items: Vec<_> = conflicts
        .into_iter()
        .map(|conflict| {
            let cid = format!("c{}", NEXT_CID.fetch_add(1, Ordering::Relaxed));
            (cid, conflict)
        })
        .collect();
    let count = items.len();
    let context = JobConflicts {
        pair: Arc::new(pair),
        items,
        resolved: Baseline::new(),
        merge: None,
        state,
        pending: pending_names,
    };
    with_state(|state| state.insert(job_id.to_string(), context));
    count
}

pub(super) fn recorded_state(job_id: &str) -> Result<crate::bisync::StateKey, ApiError> {
    with_state(|state| state.get(job_id).and_then(|context| context.state.clone()))
        .ok_or_else(|| invalid("Bitte Konflikte neu prüfen; der aufgezeichnete Sync-Zustand fehlt."))
}

pub(super) fn forget(job_id: &str) {
    with_state(|state| state.remove(job_id));
}

/// Pair and conflict of `cid`, for a resolution or merge.
pub(super) fn lookup(job_id: &str, cid: &str) -> Result<(Arc<PairContext>, Conflict), ApiError> {
    with_state(|state| {
        let context = state.get(job_id).ok_or_else(not_loaded)?;
        let conflict = context
            .items
            .iter()
            .find(|(id, _)| id == cid)
            .map(|(_, conflict)| conflict.clone())
            .ok_or_else(|| ApiError::new("not_found", "Konflikt ist nicht mehr offen."))?;
        Ok((context.pair.clone(), conflict))
    })
}

fn not_loaded() -> ApiError {
    ApiError::new(
        "not_found",
        "Konfliktliste nicht geladen – bitte „Konflikte prüfen“.",
    )
}

pub(super) fn set_merge(job_id: &str, draft: MergeDraft) {
    with_state(|state| {
        if let Some(context) = state.get_mut(job_id) {
            context.merge = Some(draft);
        }
    });
}

/// Row count of the loaded merge of `cid` (`None` = not loaded).
pub(super) fn merge_len(job_id: &str, cid: &str) -> Option<usize> {
    with_state(|state| {
        state
            .get(job_id)?
            .merge
            .as_ref()
            .filter(|draft| draft.cid == cid)
            .map(|draft| draft.rows.len())
    })
}

pub(super) fn take_merge(job_id: &str, cid: &str) -> Option<MergeDraft> {
    with_state(|state| {
        let context = state.get_mut(job_id)?;
        if context.merge.as_ref().is_some_and(|draft| draft.cid == cid) {
            context.merge.take()
        } else {
            None
        }
    })
}

/// Records a finished resolution; saves the baseline once nothing is open.
/// Returns the number of conflicts still open.
pub(super) fn apply_resolution(
    job_id: &str,
    cid: &str,
    rel: &str,
    signatures: (Option<Sig>, Option<Sig>),
) -> Result<usize, ApiError> {
    let remaining = with_state(|state| {
        let context = state.get_mut(job_id)?;
        context.items.retain(|(id, _)| id != cid);
        if context.merge.as_ref().is_some_and(|draft| draft.cid == cid) {
            context.merge = None;
        }
        if context.state.is_none() { context.resolved.insert(rel.to_string(), signatures); }
        Some(context.items.len())
    })
    .ok_or_else(not_loaded)?;
    if remaining == 0 {
        save_resolved(job_id)?;
    }
    Ok(remaining)
}

/// Merges the unsaved resolved entries into the stored baseline.
fn save_resolved(job_id: &str) -> Result<(), ApiError> {
    let _saving = SAVING.lock().unwrap_or_else(PoisonError::into_inner);
    let Some((pair, resolved)) = with_state(|state| {
        state
            .get(job_id)
            .filter(|context| !context.resolved.is_empty())
            .map(|context| (context.pair.pair.clone(), context.resolved.clone()))
    }) else {
        return Ok(());
    };
    let path = crate::bisync::baseline_path(&pair);
    let saved = crate::bisync::load_baseline(&path).and_then(|mut baseline| {
        baseline.extend(resolved.iter().map(|(rel, sigs)| (rel.clone(), *sigs)));
        crate::bisync::save_baseline(&path, &baseline)
    });
    if let Err(error) = saved {
        return Err(ApiError::new(
            "internal",
            format!("Synchronisierungsstand konnte nicht gespeichert werden: {error}"),
        ));
    }
    with_state(|state| {
        if let Some(context) = state.get_mut(job_id) {
            context
                .resolved
                .retain(|rel, sigs| resolved.get(rel) != Some(&*sigs));
        }
    });
    Ok(())
}

/// Before a run or dry run replaces the context: nothing may be in progress
/// and resolved entries must be saved.
pub(super) fn settle_before_run(job_id: &str) -> Result<(), ApiError> {
    if job_owned(job_id) {
        return Err(ApiError::new(
            "busy",
            "Für diesen Sync-Job läuft bereits ein Vorgang – bitte warten.",
        ));
    }
    settle_reserved(job_id)
}

pub(super) fn settle_reserved(job_id: &str) -> Result<(), ApiError> {
    save_resolved(job_id).map_err(|error| ApiError::new("conflict", error.message))
}

fn side(signature: Option<Sig>, duplicates: Option<&crate::bisync::DuplicateConflict>, keep_a: bool) -> Value {
    let mut value = match signature {
        Some(sig) => json!({ "exists": true, "size": sig.size, "mtimeMs": sig.mtime_ms }),
        None => json!({ "exists": false, "size": 0, "mtimeMs": 0 }),
    };
    if let Some(group) = duplicates {
        value["needsVariantChoice"] = json!(group.needs_variant_choice(keep_a));
        value["variants"] = json!(group.variants(keep_a).iter().map(|v| json!({
            "id": v.id, "size": v.content_size, "mtimeMs": v.signature.mtime_ms,
            "checksum": v.content_md5,
        })).collect::<Vec<_>>());
    }
    value
}

pub(super) fn conflicts(args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?;
    Ok(with_state(|state| match state.get(job_id) {
        None => json!({ "available": false, "items": [] }),
        Some(context) => {
            let items: Vec<Value> = context
                .items
                .iter()
                .map(|(cid, conflict)| {
                    let pending = context.pending.contains(&conflict.rel);
                    let text = pending || (conflict.duplicates.is_none() && [conflict.a, conflict.b]
                        .iter()
                        .all(|sig| sig.is_some_and(|sig| sig.size <= MAX_MERGE_BYTES)));
                    json!({
                        "cid": cid,
                        "path": conflict.rel,
                        "a": side(conflict.a, conflict.duplicates.as_ref(), true),
                        "b": side(conflict.b, conflict.duplicates.as_ref(), false),
                        "text": text,
                        "pendingMerge": pending,
                    })
                })
                .collect();
            json!({ "available": true, "items": items })
        }
    }))
}

pub(super) fn check(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job = find_job(str_arg(args, "id")?)?;
    super::args::reject_app_internal(&job.source)?;
    super::args::reject_app_internal(&job.target)?;
    settle_before_run(&job.id)?;
    let title = format!("Konflikte prüfen: {}", job.name);
    let task = spawn_for_snapshot(rt, job.clone(), title, move |ctx| {
        ctx.message("Verbinde…");
        let pair = open_pair_for(ctx, &job)?;
        ctx.message("Probelauf…");
        let out = run_bisync(ctx, &job, &pair, true)?;
        if ctx.cancelled() {
            return Err(canceled("Abgebrochen"));
        }
        report_errors(ctx, &out.errors);
        if out.conflicts.is_empty() {
            if let Some((path, message)) = out.errors.first() {
                return Err(ApiError::new("internal", format!("{path}: {message}")));
            }
        }
        let first_error = out.errors.first().map(|(path, message)| format!("{path}: {message}"));
        let block = out.blocked.as_ref().map(|block| crate::syncjobs::Blocked {
            kind: crate::syncjobs::block_kind(block), detail: block.message(),
            since: super::args::now_secs(), confirmed: false,
        });
        if out.busy { return Err(ApiError::new("busy", "Das Paar läuft bereits.")); }
        let pending = if out.errors.is_empty() && !out.canceled {
            match out.state.as_ref() {
                Some(key) => super::sync_merge::recovery::conflicts(&pair, key, &ctx.cancel_flag())?,
                None => Vec::new(),
            }
        } else { Vec::new() };
        if out.errors.is_empty() && !out.canceled {
            crate::syncjobs::update_job_state(&job.id, |state| { state.blocked = block; })
                .map_err(|e| ApiError::new("internal", format!("Prüfergebnis speichern: {e}")))?;
        }
        let count = store_with_pending(&job.id, pair, out.conflicts, out.state, pending);
        if let Some(error) = first_error {
            ctx.set_failure_result(json!({ "conflicts": count, "errors": out.errors.len() }));
            return Err(ApiError::new("internal", error));
        }
        ctx.message(&format!("{count} Konflikte gefunden"));
        Ok(json!({ "conflicts": count, "errors": out.errors.len() }))
    })?;
    Ok(json!({ "taskId": task }))
}

fn phase_label(phase: ResolvePhase) -> &'static str {
    match phase {
        ResolvePhase::Preparing => "prüft beide Seiten",
        ResolvePhase::BackingUp => "sichert die ersetzte Version",
        ResolvePhase::Copying => "überträgt die gewählte Version",
        ResolvePhase::Deleting => "übernimmt die Löschung",
        ResolvePhase::ReadingSignatures => "prüft das Ergebnis",
    }
}

pub(super) fn resolve(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?.to_string();
    let cid = str_arg(args, "cid")?.to_string();
    let keep_a = match str_arg(args, "choice")? {
        "a" => true,
        "b" => false,
        other => return Err(invalid(format!("Unbekannte Wahl „{other}“."))),
    };
    let (pair, conflict) = lookup(&job_id, &cid)?;
    let state = recorded_state(&job_id)?;
    let variant_id = args.get("variantId").filter(|v| !v.is_null())
        .map(|v| v.as_str().map(str::to_owned).ok_or_else(|| invalid("Ungültige Datei-ID")))
        .transpose()?;
    let title = format!("Konflikt lösen: {}", conflict.rel);
    let job = find_job(&job_id)?;
    let task = spawn_for_snapshot(rt, job, title, move |ctx| {
        let cancel = ctx.cancel_flag();
        if super::sync_merge::recovery::load(&pair, &state, &conflict.rel, &cancel)?.is_some() {
            return Err(invalid("Hier ist eine Zusammenführung offen. Bitte den gespeicherten Auftrag unverändert wiederholen."));
        }
        let signatures = crate::bisync::resolve_recorded(
            &*pair.a,
            &pair.root_a,
            &*pair.b,
            &pair.root_b,
            &conflict,
            keep_a,
            variant_id.as_deref(),
            &state,
            &cancel,
            |phase| ctx.message(phase_label(phase)),
        )
        .map_err(|error| super::args::io_error("Konflikt konnte nicht gelöst werden", error))?;
        let remaining = apply_resolution(&job_id, &cid, &conflict.rel, signatures)?;
        Ok(json!({ "remaining": remaining }))
    })?;
    Ok(json!({ "taskId": task }))
}

/// Like a resolution, skipping the last open conflict saves the baseline.
pub(super) fn skip(args: &Value) -> Result<Value, ApiError> {
    let job_id = str_arg(args, "id")?;
    let cid = str_arg(args, "cid")?;
    let none_open = with_state(|state| {
        let context = state.get_mut(job_id).ok_or_else(not_loaded)?;
        context.items.retain(|(id, _)| id != cid);
        if context.merge.as_ref().is_some_and(|draft| draft.cid == cid) {
            context.merge = None;
        }
        Ok::<_, ApiError>(context.items.is_empty())
    })?;
    // Outside `with_state`: saving takes the state lock itself.
    if none_open {
        save_resolved(job_id)?;
    }
    Ok(json!({}))
}

pub(super) fn finish(args: &Value) -> Result<Value, ApiError> {
    save_resolved(str_arg(args, "id")?)?;
    Ok(json!({}))
}

#[cfg(test)]
#[path = "sync_conflict_variant_task_tests.rs"]
mod variant_tests;
