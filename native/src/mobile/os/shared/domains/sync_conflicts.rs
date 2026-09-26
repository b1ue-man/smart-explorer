//! Conflict context per job (in memory, like the desktop session): the pair
//! of the last run or dry run, its open conflicts, and resolved entries whose
//! baseline update is not saved yet. Saving merges the resolved entries into
//! the stored baseline, so a newer background run's baseline is kept.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{json, Value};

use super::args::{canceled, invalid, str_arg};
use super::sync_jobs::find_job;
use super::sync_run::{open_pair, report_errors, run_bisync, running_task, spawn_for_job};
use crate::bisync::{Baseline, Conflict, ResolvePhase, Sig};
use crate::linemerge::Row;
use crate::mobile::{ApiError, Runtime};
use crate::vfs::BackendHandle;

/// Largest text side the line merge accepts (`remote_util::read_text`).
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
}

struct JobConflicts {
    pair: Arc<PairContext>,
    items: Vec<(String, Conflict)>,
    resolved: Baseline,
    merge: Option<MergeDraft>,
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
    let items = conflicts
        .into_iter()
        .map(|conflict| {
            let cid = format!("c{}", NEXT_CID.fetch_add(1, Ordering::Relaxed));
            (cid, conflict)
        })
        .collect();
    let context = JobConflicts {
        pair: Arc::new(pair),
        items,
        resolved: Baseline::new(),
        merge: None,
    };
    with_state(|state| state.insert(job_id.to_string(), context));
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
        context.resolved.insert(rel.to_string(), signatures);
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
    if running_task(job_id).is_some() {
        return Err(ApiError::new(
            "busy",
            "Für diesen Sync-Job läuft bereits ein Vorgang – bitte warten.",
        ));
    }
    save_resolved(job_id).map_err(|error| ApiError::new("conflict", error.message))
}

fn side(signature: Option<Sig>) -> Value {
    match signature {
        Some(sig) => json!({ "exists": true, "size": sig.size, "mtimeMs": sig.mtime_ms }),
        None => json!({ "exists": false, "size": 0, "mtimeMs": 0 }),
    }
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
                    let text = [conflict.a, conflict.b]
                        .iter()
                        .all(|sig| sig.is_some_and(|sig| sig.size <= MAX_MERGE_BYTES));
                    json!({
                        "cid": cid,
                        "path": conflict.rel,
                        "a": side(conflict.a),
                        "b": side(conflict.b),
                        "text": text,
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
    let job_id = job.id.clone();
    let title = format!("Konflikte prüfen: {}", job.name);
    let task = spawn_for_job(rt, &job_id, title, move |ctx| {
        ctx.message("Verbinde…");
        let pair = open_pair(&job)?;
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
        let count = out.conflicts.len();
        store_run(&job.id, pair, out.conflicts);
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
    let title = format!("Konflikt lösen: {}", conflict.rel);
    let owner = job_id.clone();
    let task = spawn_for_job(rt, &owner, title, move |ctx| {
        let cancel = ctx.cancel_flag();
        let signatures = crate::bisync::resolve_checked(
            &*pair.a,
            &pair.root_a,
            &*pair.b,
            &pair.root_b,
            &conflict,
            keep_a,
            &pair.pair,
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
