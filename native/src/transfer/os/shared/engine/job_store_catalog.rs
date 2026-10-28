//! Background discovery and live vault monitoring of stored jobs. Completed
//! jobs are only kept for `COMPLETED_RETENTION` (M21, 2026-10-28): their
//! record and crash log serve the "transfer missing files" retry of a run
//! that just ended; afterwards they would grow `transfer-jobs` and this
//! catalog's start forever.
use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Duration;

use super::{
    path, read_record, record_context, record_io, root, valid_id, Endpoint, JobRecord, Mutex,
    RecordContext, RecordKind, TransferJob,
};

/// A completed job's record and crash log stay this long after its end.
const COMPLETED_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// Removes the completed job `id` when its record is older than
/// `COMPLETED_RETENTION` and no run holds it (its lock is taken first, so
/// a live retry of the same job is never cut short). The record goes first,
/// then the crash log folder, the folder aliases (under their own lock) and
/// last the job's lock file; a crash between leaves files no job reads.
/// Failures leave the job as it is.
fn prune_completed(id: &str) {
    let Ok(record) = path(id) else {
        return;
    };
    let old = std::fs::metadata(&record)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age >= COMPLETED_RETENTION);
    if !old {
        return;
    }
    let lock_path = root().join(format!("{id}.lock"));
    let Ok(lock) = crate::support_dirs::open_private_lock(&lock_path) else {
        return;
    };
    if lock.try_lock().is_err() {
        return;
    }
    if std::fs::remove_file(&record).is_err() {
        return;
    }
    let _ = std::fs::remove_dir_all(record.with_extension("files"));
    prune_aliases(id);
    remove_lock_file(&lock_path, lock);
}

/// The folder aliases of the pruned job `id` (`transfer-aliases`), unless a
/// run holds them.
fn prune_aliases(id: &str) {
    let aliases = crate::support_dirs::app_data_file("transfer-aliases");
    let lock_path = aliases.join(format!("{id}.lock"));
    let lock = match std::fs::symlink_metadata(&lock_path) {
        Ok(_) => match crate::support_dirs::open_private_lock(&lock_path) {
            Ok(lock) if lock.try_lock().is_ok() => Some(lock),
            _ => return,
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return,
    };
    let _ = std::fs::remove_file(aliases.join(format!("{id}.json")));
    if let Some(lock) = lock {
        remove_lock_file(&lock_path, lock);
    }
}

/// Removes the lock file `path` whose lock `lock` holds: while it is held
/// where the system allows that (no later opener finds the old file), else
/// right after its release (Windows keeps open files). Best effort.
fn remove_lock_file(path: &Path, lock: File) {
    let removed = std::fs::remove_file(path).is_ok();
    drop(lock);
    if !removed {
        let _ = std::fs::remove_file(path);
    }
}

pub(crate) fn unconfirmed_jobs() -> io::Result<Vec<JobRecord>> {
    let entries = match std::fs::read_dir(root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| valid_id(id))
        else {
            continue;
        };
        let record: JobRecord = read_record(&path(id)?, RecordKind::Job, id, u64::MAX, false)?;
        if record.version == 1 && record.id == id && !record.complete {
            records.push(record);
        }
    }
    records.sort_by(|one, two| one.id.cmp(&two.id));
    Ok(records)
}

pub(crate) fn reopen_job(
    id: &str,
    source: Endpoint,
    target: Endpoint,
) -> Result<TransferJob, String> {
    let record: JobRecord = read_record(
        &path(id).map_err(|error| error.to_string())?,
        RecordKind::Job,
        id,
        u64::MAX,
        false,
    )
    .map_err(|error| error.to_string())?;
    record.bind(source, target)
}

#[derive(Clone, Default)]
pub(crate) struct CatalogState {
    pub ready: bool,
    pub records: Vec<JobRecord>,
    pub error: Option<String>,
    pub unavailable: Vec<String>,
}

fn load_catalog() -> io::Result<(CatalogState, Vec<(String, RecordContext)>)> {
    let entries = match std::fs::read_dir(root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((
                CatalogState {
                    ready: true,
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        Err(error) => return Err(error),
    };
    let mut state = CatalogState {
        ready: true,
        ..Default::default()
    };
    let mut contexts = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| valid_id(id))
        else {
            continue;
        };
        let loaded = (|| {
            let (record, sealed): (JobRecord, bool) =
                record_io::read_record_state(&path(id)?, RecordKind::Job, id, u64::MAX, false)?;
            if record.version != 1 || record.id != id {
                return Err(io::Error::other("Ungültiger Jobdatensatz"));
            }
            if record.complete {
                prune_completed(id);
                return Ok(None);
            }
            let context = record_context(&record.vault_locations)?;
            if sealed != context.is_some() {
                return Err(io::Error::other(
                    "Vaultbindung des Jobdatensatzes ist unbestätigt",
                ));
            }
            if let Some(context) = &context {
                crate::vault::check_transfer_blob_context_live(context)?;
            }
            Ok(Some((record, context)))
        })();
        match loaded {
            Ok(Some((record, context))) => {
                if let Some(context) = context {
                    contexts.push((id.to_string(), context));
                }
                state.records.push(record);
            }
            Ok(None) => {}
            Err(_) => state.unavailable.push(id.to_string()),
        }
    }
    if !state.unavailable.is_empty() {
        state.error = Some(format!(
            "Gesperrte oder unbestätigte Job-IDs: {}",
            state.unavailable.join(", ")
        ));
    }
    Ok((state, contexts))
}

pub(crate) struct RecoveryCatalog(std::sync::Arc<Mutex<CatalogState>>);

impl RecoveryCatalog {
    pub fn start() -> io::Result<Self> {
        let state = std::sync::Arc::new(Mutex::new(CatalogState::default()));
        let background = state.clone();
        std::thread::Builder::new()
            .name("transfer-recovery-catalog".into())
            .spawn(move || {
                let (result, contexts) = match load_catalog() {
                    Ok(loaded) => loaded,
                    Err(error) => (
                        CatalogState {
                            ready: true,
                            error: Some(error.to_string()),
                            ..Default::default()
                        },
                        Vec::new(),
                    ),
                };
                *background
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = result;
                let weak = std::sync::Arc::downgrade(&background);
                drop(background);
                record_io::monitor_contexts(weak, contexts, |state, id| {
                    state.records.retain(|record| record.id != id);
                    if !state.unavailable.iter().any(|record| record == id) {
                        state.unavailable.push(id.to_string());
                    }
                    state.error = Some(format!(
                        "Gesperrte oder unbestätigte Job-IDs: {}",
                        state.unavailable.join(", ")
                    ));
                });
            })?;
        Ok(Self(state))
    }
    pub fn snapshot(&self) -> CatalogState {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M21: a pruned job leaves neither its lock file nor its folder aliases.
    #[test]
    fn transfer_engine_task_pruned_job_leaves_no_lock_or_alias_files() {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).expect("random job id");
        let id: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        crate::support_dirs::ensure_private_dir(&root()).expect("job folder");
        let aliases = crate::support_dirs::app_data_file("transfer-aliases");
        crate::support_dirs::ensure_private_dir(&aliases).expect("alias folder");
        let record = path(&id).expect("record path");
        std::fs::write(&record, b"{}").expect("record");
        let old = std::time::SystemTime::now() - COMPLETED_RETENTION - Duration::from_secs(60);
        File::options()
            .write(true)
            .open(&record)
            .and_then(|file| file.set_modified(old))
            .expect("an old record");
        std::fs::create_dir_all(record.with_extension("files")).expect("crash log");
        let alias = aliases.join(format!("{id}.json"));
        std::fs::write(&alias, b"{}").expect("alias");
        let alias_lock = aliases.join(format!("{id}.lock"));
        drop(crate::support_dirs::open_private_lock(&alias_lock).expect("alias lock"));
        prune_completed(&id);
        let leftovers = [
            record.clone(),
            record.with_extension("files"),
            root().join(format!("{id}.lock")),
            alias,
            alias_lock,
        ];
        for leftover in leftovers {
            assert!(!leftover.exists(), "{}", leftover.display());
        }
    }
}
