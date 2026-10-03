//! Versions are selected through opaque in-process tokens, never client paths.
use std::sync::{Mutex, PoisonError};
use std::sync::atomic::{AtomicU64, Ordering};
use serde_json::{json, Value};
use crate::bisync::versions::{VersionEntry, VersionReason, VersionSide, VersionStore};
use crate::bisync::PairSide;
use crate::mobile::{ApiError, Runtime};
use super::args::{invalid, io_error, str_arg};
use super::sync_jobs::find_job;
use super::sync_run::{open_pair_for, spawn_for_snapshot};

struct Listed { job: String, source: String, target: String, entries: Vec<(String, VersionEntry)> }
static LISTED: Mutex<Option<Listed>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

pub(super) fn list(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job = find_job(str_arg(args, "id")?)?;
    let task = spawn_for_snapshot(rt, job.clone(), format!("Versionen: {}", job.name), move |ctx| {
        let pair = open_pair_for(ctx, &job)?;
        let cancel = ctx.cancel_flag();
        let sides = [VersionSide { side: PairSide::A, backend: &*pair.a, root: &pair.root_a },
            VersionSide { side: PairSide::B, backend: &*pair.b, root: &pair.root_b }];
        let entries = crate::bisync::versions::list_versions(&pair.pair, &sides, &cancel)
            .map_err(|e| io_error("Versionen laden", e))?;
        let entries: Vec<_> = entries.into_iter().filter(|entry|
            entry.job_id.as_ref().is_none_or(|id| id == &job.id))
            .map(|entry| (format!("v{}", NEXT.fetch_add(1, Ordering::Relaxed)), entry)).collect();
        let rows: Vec<_> = entries.iter().map(|(token, entry)| json!({
            "token": token, "path": entry.rel, "side": entry.side.map(PairSide::as_str),
            "runId": entry.run_id, "preservedMs": entry.preserved_ms, "size": entry.size,
            "reason": entry.reason.map(|reason| match reason { VersionReason::Replaced => "replaced",
                VersionReason::Deleted => "deleted", VersionReason::Resolved => "resolved", VersionReason::Restored => "restored" }),
            "store": match entry.store { VersionStore::SyncRoot => "sync_root", VersionStore::AppData => "app_data" },
        })).collect();
        *LISTED.lock().unwrap_or_else(PoisonError::into_inner) = Some(Listed {
            job: job.id, source: job.source, target: job.target, entries,
        });
        Ok(json!({ "items": rows }))
    })?;
    Ok(json!({ "taskId": task }))
}

pub(super) fn restore(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let job = find_job(str_arg(args, "id")?)?;
    let token = str_arg(args, "token")?;
    let entry = {
        let listed = LISTED.lock().unwrap_or_else(PoisonError::into_inner);
        listed.as_ref().filter(|listed| listed.job == job.id && listed.source == job.source && listed.target == job.target)
            .and_then(|listed| listed.entries.iter().find(|(id, _)| id == token).map(|(_, entry)| entry.clone()))
            .ok_or_else(|| invalid("Versionen bitte erneut laden; der Job oder die Auswahl hat sich geändert."))?
    };
    let side = match entry.side {
        Some(side) => side,
        None => PairSide::parse(str_arg(args, "side")?).ok_or_else(|| invalid("Bitte Quelle oder Ziel wählen."))?,
    };
    let task = spawn_for_snapshot(rt, job.clone(), format!("Wiederherstellen: {}", entry.rel), move |ctx| {
        let pair = open_pair_for(ctx, &job)?;
        let cancel = ctx.cancel_flag();
        let lock_id = crate::bisync::pair_lock_id(&*pair.a, &pair.root_a, &*pair.b, &pair.root_b);
        let lock = crate::bisync::PairLock::acquire(&lock_id).map_err(|e| io_error("Paarsperre", e))?;
        let sides = [VersionSide { side: PairSide::A, backend: &*pair.a, root: &pair.root_a },
            VersionSide { side: PairSide::B, backend: &*pair.b, root: &pair.root_b }];
        let present = crate::bisync::versions::list_versions(&pair.pair, &sides, &cancel)
            .map_err(|e| io_error("Version erneut prüfen", e))?;
        if !present.contains(&entry) { return Err(ApiError::new("conflict", "Die Version ist nicht mehr vorhanden. Bitte erneut laden.")); }
        let destination = match side { PairSide::A => &sides[0], PairSide::B => &sides[1] };
        crate::bisync::versions::restore_version(&lock, &pair.pair, &entry, destination, &cancel)
            .map_err(|e| io_error("Version wiederherstellen", e))?;
        ctx.message("Version wiederhergestellt; der nächste Sync übernimmt sie auf die andere Seite.");
        Ok(json!({ "restored": true, "path": entry.rel, "side": side.as_str() }))
    })?;
    Ok(json!({ "taskId": task }))
}
