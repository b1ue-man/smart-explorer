//! Durable unconfirmed jobs. Handles/credentials are rebound explicitly.
use super::record_io::{
    self, read_record, record_context, write_record, RecordContext, RecordKind,
};
use super::view::{JobView, Side};
use crate::transfer::job_record::{JobRecord, SourceGuard, StoredEndpoint, StoredFilter};
use crate::transfer::{Endpoint, JobItems, TransferJob};
use crate::types::{Conflict, CopyMode};
use crate::vfs::{Backend, LocalBackend};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

#[path = "job_store_catalog.rs"]
mod catalog;
#[path = "job_store_validation.rs"]
mod validation;

pub(crate) use catalog::{reopen_job, unconfirmed_jobs, CatalogState, RecoveryCatalog};
pub(super) use validation::{endpoint, snapshot, with_backend};
use validation::{source_paths, verify_sources, verify_target};

pub(super) struct JobRun {
    pub(super) path: PathBuf,
    _lock: std::fs::File,
    pub(super) context: Option<RecordContext>,
    pub(super) data: Mutex<JobRecord>,
    /// The job's crash log (receipts of every publication).
    pub(super) journal: super::file_recovery::Journal,
    completed: Vec<crate::transfer::ResolvedRoot>,
    files: Vec<crate::transfer::ResolvedRoot>,
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn root() -> PathBuf {
    crate::support_dirs::app_data_file("transfer-jobs")
}
fn path(id: &str) -> io::Result<PathBuf> {
    if !valid_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ungültige Transfer-Jobkennung",
        ));
    }
    Ok(root().join(format!("{id}.json")))
}

fn contract(view: &JobView<'_>, target_root: &str, id: &str) -> io::Result<JobRecord> {
    let paths = source_paths(view.items);
    let vault_locations =
        record_io::record_locations(view.source, view.target, &paths, target_root)?;
    Ok(JobRecord {
        version: 1,
        id: id.to_string(),
        source: endpoint(view.source),
        target: endpoint(view.target),
        target_dir: target_root.to_string(),
        target_object: None,
        vault_locations,
        items: view.items.clone(),
        layout: view.layout,
        filter: view.filter.map(StoredFilter::of),
        file_policy: match view.conflict {
            Conflict::Rename => 0,
            Conflict::Overwrite => 1,
            Conflict::Skip => 2,
        },
        folders: view.folders.clone(),
        moving: view.mode == CopyMode::Move,
        guards: Vec::new(),
        complete: false,
        mutation_started: false,
        file_journal: true,
    })
}

pub(super) fn begin(
    view: &JobView<'_>,
    target_root: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> io::Result<Option<JobRun>> {
    if !view.recoverable {
        return Ok(None);
    }
    let Some(id) = view.id else {
        return Ok(None);
    };
    let path = path(id)?;
    crate::support_dirs::ensure_private_dir(&root())?;
    let lock = crate::support_dirs::open_private_lock(&root().join(format!("{id}.lock")))?;
    lock.try_lock()
        .map_err(|error| io::Error::other(format!("Job {id} wird bereits ausgeführt: {error}")))?;
    let expected = contract(view, target_root, id)?;
    let context = record_context(&expected.vault_locations)?;
    let (data, completed, files, journal) = match read_record::<JobRecord>(
        &path,
        RecordKind::Job,
        id,
        u64::MAX,
        context.is_some(),
    ) {
        Ok(data) => {
            if view.recovery == crate::transfer::RecoveryMode::New {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Eine neue Übertragung darf keine vorhandene Job-ID übernehmen",
                ));
            }
            if !data.file_journal && data.mutation_started {
                return Err(io::Error::other("Alter Auftrag ohne Dateipublikationsbelege; eine sichere Wiederaufnahme ist unbestätigt"));
            }
            let mut candidate = data.clone();
            candidate.guards.clear();
            candidate.complete = false;
            candidate.target_object = None;
            candidate.mutation_started = false;
            candidate.file_journal = true;
            if candidate != expected {
                return Err(io::Error::other(format!(
                    "Auftrag {id} hat einen abweichenden Quellen-/Ziel-/Policyvertrag"
                )));
            }
            if data.complete && !super::file_recovery::missing_retry::live(view) {
                return Err(io::Error::other(format!(
                    "Auftrag {id} ist bereits abgeschlossen"
                )));
            }
            verify_target(view, &data)?;
            let mut data = data;
            if super::file_recovery::missing_retry::live(view) {
                data.complete = false;
                if !data.mutation_started {
                    data.guards = source_paths(view.items)
                        .into_iter()
                        .map(|path| snapshot(view.source, path))
                        .collect();
                    data.target_object = with_backend(view.target, |backend| {
                        crate::vfs::directory_identity(backend, target_root)
                    })
                    .ok()
                    .flatten();
                }
                write_record(&path, RecordKind::Job, id, context.as_ref(), &data)?;
            }
            // Created only before any mutation; a lost log of a started job
            // never looks unstarted.
            let journal = super::file_recovery::Journal::open(
                &path,
                &data,
                context.clone(),
                !data.mutation_started,
            )?;
            let receipts = super::file_recovery::verify_saved(view, &data, &journal, cancel)?;
            if super::file_recovery::missing_retry::live(view) {
                for guard in data.guards.iter_mut().filter(|guard| !guard.verified) {
                    let current = snapshot(view.source, &guard.path);
                    if current.verified {
                        *guard = current;
                    }
                }
                write_record(&path, RecordKind::Job, id, context.as_ref(), &data)?;
            }
            let completed = verify_sources(view, &data, &receipts)?;
            let files = receipts.into_values().map(|(_, root)| root).collect();
            (data, completed, files, journal)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if view.recovery == crate::transfer::RecoveryMode::Resume
                && !super::file_recovery::missing_retry::unstarted(view, &path)?
            {
                return Err(io::Error::other(
                    "Gespeicherter Auftrag fehlt; Wiederaufnahme bleibt unbestätigt",
                ));
            }
            let mut data = expected;
            data.target_object = with_backend(view.target, |backend| {
                crate::vfs::directory_identity(backend, target_root)
            })
            .ok()
            .flatten();
            let sources = source_paths(view.items);
            data.guards = sources
                .into_iter()
                .map(|path| snapshot(view.source, path))
                .collect();
            write_record(&path, RecordKind::Job, id, context.as_ref(), &data)?;
            let journal = super::file_recovery::Journal::open(&path, &data, context.clone(), true)?;
            (data, Vec::new(), Vec::new(), journal)
        }
        Err(error) => return Err(error),
    };
    let mut data = data;
    if !data.file_journal {
        for guard in &mut data.guards {
            if !guard.directory && guard.object.is_none() {
                *guard = snapshot(view.source, &guard.path);
            }
        }
    }
    data.file_journal = true;
    Ok(Some(JobRun {
        path,
        _lock: lock,
        data: Mutex::new(data),
        journal,
        context,
        completed,
        files,
    }))
}

pub(super) fn preflight(view: &JobView<'_>) -> io::Result<Option<String>> {
    if !view.recoverable {
        return Ok(None);
    }
    let Some(id) = view.id else {
        return Ok(None);
    };
    let locations = record_io::record_locations(
        view.source,
        view.target,
        &source_paths(view.items),
        view.target_dir,
    )?;
    let context = record_context(&locations)?;
    let record = match read_record::<JobRecord>(
        &path(id)?,
        RecordKind::Job,
        id,
        u64::MAX,
        context.is_some(),
    ) {
        Ok(record) => record,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && (view.recovery == crate::transfer::RecoveryMode::New
                    || super::file_recovery::missing_retry::unstarted(view, &path(id)?)?) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    if view.recovery == crate::transfer::RecoveryMode::New {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Job-ID ist bereits vergeben",
        ));
    }
    let requested = if view.target_dir == record.target_dir {
        view.target_dir.to_string()
    } else if view.target.is_local() && !view.source.is_local() {
        super::folders::existing_target_root(view.target_dir)?
    } else {
        return Err(io::Error::other(
            "Wiederaufnahme hat einen abweichenden Zielroot",
        ));
    };
    if requested != record.target_dir {
        return Err(io::Error::other(
            "Wiederaufnahme verweist auf einen geänderten Zielroot",
        ));
    }
    verify_target(view, &record)?;
    Ok(Some(record.target_dir))
}

impl JobRun {
    pub fn completed_roots(&self) -> &[crate::transfer::ResolvedRoot] {
        &self.completed
    }
    pub fn saved_file_roots(&self) -> &[crate::transfer::ResolvedRoot] {
        &self.files
    }
    pub fn prepare_apply(&self) -> io::Result<bool> {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let required = data.mutation_started
            && data.guards.iter().any(|guard| {
                guard.directory
                    && !matches!(
                        data.folders.for_root(&guard.path),
                        crate::transfer::FolderConflict::Merge
                            | crate::transfer::FolderConflict::Skip
                    )
            });
        if !data.mutation_started {
            data.mutation_started = true;
            write_record(
                &self.path,
                RecordKind::Job,
                &data.id,
                self.context.as_ref(),
                &*data,
            )?;
        }
        Ok(required)
    }
    pub fn confirm_sources(&self, view: &JobView<'_>) -> io::Result<()> {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = false;
        if data.target_object.is_none() {
            let actual = with_backend(view.target, |backend| {
                crate::vfs::directory_identity(backend, &data.target_dir)
            })
            .ok()
            .flatten();
            if actual.is_some() {
                data.target_object = actual;
                changed = true;
            }
        }
        for guard in &mut data.guards {
            if !guard.verified {
                let current = snapshot(view.source, &guard.path);
                if current.verified {
                    *guard = current;
                    changed = true;
                }
            }
        }
        if changed {
            write_record(
                &self.path,
                RecordKind::Job,
                &data.id,
                self.context.as_ref(),
                &*data,
            )?;
        }
        Ok(())
    }

    pub fn finish(&self, complete: bool) -> io::Result<()> {
        // Acknowledgements written without their own sync are durable
        // before the job record says how the run ended.
        self.journal.flush()?;
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.complete = complete;
        write_record(
            &self.path,
            RecordKind::Job,
            &data.id,
            self.context.as_ref(),
            &*data,
        )
    }
}

#[cfg(test)]
#[path = "file_recovery_tests.rs"]
mod file_recovery_tests;
