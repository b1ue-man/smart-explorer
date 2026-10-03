//! Fresh job endpoints, pair lock and engine-authorized version restoration.
use super::sync_versions_ui::{VersionIdentity, VersionSnapshot, VersionTask};
use crate::bisync::{PairSide, versions::{VersionEntry,VersionSide}};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

pub(in crate::app) fn start(id:String, restore:Option<(VersionIdentity,VersionEntry,PairSide)>) -> Result<VersionTask,String> {
    let restoring = restore.is_some();
    let cancel = Arc::new(AtomicBool::new(false)); let worker_cancel = cancel.clone();
    let (tx,rx) = crossbeam_channel::bounded(1);
    let worker = std::thread::Builder::new().name("sync-versions".into()).spawn(move || {
        let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::SyncRun);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(&id,restore,&worker_cancel)))
            .unwrap_or_else(|_| Err("Versionsaktion endete unerwartet; Original prüfen.".into()));
        let _ = tx.send(result);
    }).map_err(|e| e.to_string())?;
    Ok(VersionTask { worker:Some(worker),rx,cancel,restoring })
}
fn job(id:&str) -> Result<crate::syncjobs::SyncJob,String> {
    crate::syncjobs::load().map_err(|e| e.to_string())?.into_iter().find(|job| job.id == id)
        .ok_or_else(|| "Setup wurde entfernt; Versionen bleiben unverändert.".into())
}
fn execute(id:&str, restore:Option<(VersionIdentity,VersionEntry,PairSide)>, cancel:&AtomicBool) -> Result<VersionSnapshot,String> {
    let job = job(id)?; job.validate()?;
    if cancel.load(Ordering::Acquire) { return Err("Versionsaktion abgebrochen".into()); }
    let (a, root_a) = crate::connect::resolve_endpoint(&job.source)?;
    let (b, root_b) = crate::connect::resolve_endpoint(&job.target)?;
    let fresh = self::job(id)?;
    if fresh.source != job.source || fresh.target != job.target { return Err("Setup wurde umgestellt; Versionen erneut öffnen.".into()); }
    let a = crate::vfs::sync_backend(a); let b = crate::vfs::sync_backend(b);
    let pair = crate::bisync::pair_id_for(&*a,&root_a,&*b,&root_b);
    let lock_id = crate::bisync::pair_lock_id(&*a,&root_a,&*b,&root_b);
    let identity = VersionIdentity { source:job.source, target:job.target, root_a:root_a.clone(), root_b:root_b.clone(), pair:pair.clone(), lock:lock_id.clone() };
    let lock = crate::bisync::PairLock::acquire(&lock_id).map_err(|e| e.to_string())?;
    let sides = [VersionSide { side:PairSide::A,backend:&*a,root:&root_a },VersionSide { side:PairSide::B,backend:&*b,root:&root_b }];
    let mut message = None;
    if let Some((expected,entry,side)) = restore {
        if expected != identity || entry.job_id.as_deref().is_some_and(|owner| owner != id)
            || entry.side.is_some_and(|original| original != side) {
            return Err("Die Version gehört nicht mehr zu diesem Setup oder dieser Seite; erneut laden.".into());
        }
        crate::bisync::versions::restore_version(&lock,&pair,&entry,&sides[if side==PairSide::A { 0 } else { 1 }],cancel)
            .map_err(|e| format!("Wiederherstellung: {e}"))?;
        message = Some(format!("„{}“ auf {} wiederhergestellt; vorherige Datei als Version erhalten",entry.rel,side.label()));
    }
    let listed = crate::bisync::versions::list_versions(&pair,&sides,cancel);
    let (mut entries,load_error) = match listed {
        Ok(entries) => (entries,None),
        Err(error) if message.is_some() => (Vec::new(),Some(format!("Wiederherstellung abgeschlossen, Liste nicht neu geladen: {error}"))),
        Err(error) => return Err(error.to_string()),
    };
    entries.retain(|entry| entry.job_id.as_deref().is_none_or(|owner| owner == id));
    entries.sort_by(|left,right| right.preserved_ms.cmp(&left.preserved_ms).then_with(||left.rel.cmp(&right.rel)));
    Ok(VersionSnapshot { identity,entries,message,load_error })
}
