//! Durable unconfirmed jobs. Handles/credentials are rebound explicitly.
use super::super::super::job_record::{JobRecord, SourceGuard, StoredEndpoint, StoredFilter};
use super::view::{JobView, Side};
use super::record_io::{self, read_record, write_record, record_context, RecordContext, RecordKind};
use crate::transfer::{Endpoint, JobItems, TransferJob};
use crate::types::{Conflict, CopyMode};
use crate::vfs::{Backend, LocalBackend};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

pub(super) struct JobRun {
    pub(super) path: PathBuf,
    _lock: std::fs::File,
    pub(super) context: Option<RecordContext>,
    pub(super) data: Mutex<JobRecord>,
    completed: Vec<crate::transfer::ResolvedRoot>,
    files: Vec<crate::transfer::ResolvedRoot>,
}

fn valid_id(id: &str) -> bool { id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) }
fn root() -> PathBuf { crate::support_dirs::app_data_file("transfer-jobs") }
fn path(id: &str) -> io::Result<PathBuf> {
    if !valid_id(id) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "Ungültige Transfer-Jobkennung")); }
    Ok(root().join(format!("{id}.json")))
}

pub(super) fn endpoint(side: Side<'_>) -> StoredEndpoint {
    match side {
        Side::Local => StoredEndpoint::of(&Endpoint::Local),
        Side::Remote(backend) => StoredEndpoint { local: false, namespace: backend.namespace_identity(), state: backend.state_identity() },
    }
}

pub(super) fn with_backend<T>(side: Side<'_>, operation: impl FnOnce(&dyn Backend) -> T) -> T {
    match side { Side::Local => operation(&LocalBackend::new("/")), Side::Remote(backend) => operation(backend) }
}

pub(super) fn snapshot(side: Side<'_>, path: &str) -> SourceGuard {
    with_backend(side, |backend| match backend.stat(path) {
        Ok(meta) => SourceGuard { path: path.to_string(), directory: meta.is_dir, link: meta.is_symlink, id: meta.id, object: if meta.is_dir && !meta.is_symlink { crate::vfs::directory_identity(backend, path).ok().flatten() } else if !meta.is_symlink && !meta.special { crate::vfs::file_identity(backend, path).ok().flatten() } else { None }, size: meta.size, mtime_ms: meta.mtime_ms, verified: true },
        Err(_) => SourceGuard { path: path.to_string(), directory: false, link: false, id: None, object: None, size: 0, mtime_ms: 0, verified: false },
    })
}

fn source_paths(items: &JobItems) -> Vec<&str> {
    match items { JobItems::Roots { paths, .. } => paths.iter().map(String::as_str).collect(), JobItems::Pairs(pairs) => pairs.iter().map(|pair| pair.source.as_str()).collect() }
}

fn contract(view: &JobView<'_>, target_root: &str, id: &str) -> io::Result<JobRecord> {
    let paths = source_paths(view.items);
    let vault_locations = record_io::record_locations(view.source, view.target, &paths, target_root)?;
    Ok(JobRecord { version: 1, id: id.to_string(), source: endpoint(view.source), target: endpoint(view.target), target_dir: target_root.to_string(), target_object: None, vault_locations, items: view.items.clone(), layout: view.layout, filter: view.filter.map(StoredFilter::of), file_policy: match view.conflict { Conflict::Rename => 0, Conflict::Overwrite => 1, Conflict::Skip => 2 }, folders: view.folders.clone(), moving: view.mode == CopyMode::Move, guards: Vec::new(), complete: false, mutation_started: false, file_journal: true })
}

pub(super) fn begin(view: &JobView<'_>, target_root: &str, cancel: &std::sync::atomic::AtomicBool) -> io::Result<Option<JobRun>> {
    if !view.recoverable { return Ok(None); }
    let Some(id) = view.id else { return Ok(None); };
    let path = path(id)?;
    crate::support_dirs::ensure_private_dir(&root())?;
    let lock = crate::support_dirs::open_private_lock(&root().join(format!("{id}.lock")))?;
    lock.try_lock().map_err(|error| io::Error::other(format!("Job {id} wird bereits ausgeführt: {error}")))?;
    let expected = contract(view, target_root, id)?;
    let context = record_context(&expected.vault_locations)?;
    let (data, completed, files) = match read_record::<JobRecord>(&path, RecordKind::Job, id, u64::MAX, context.is_some()) {
        Ok(data) => {
            if view.recovery == crate::transfer::RecoveryMode::New { return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Eine neue Übertragung darf keine vorhandene Job-ID übernehmen")); }
            if !data.file_journal && data.mutation_started { return Err(io::Error::other("Alter Auftrag ohne Dateipublikationsbelege; eine sichere Wiederaufnahme ist unbestätigt")); }
            let mut candidate = data.clone(); candidate.guards.clear(); candidate.complete = false; candidate.target_object = None; candidate.mutation_started = false; candidate.file_journal = true;
            if candidate != expected { return Err(io::Error::other(format!("Auftrag {id} hat einen abweichenden Quellen-/Ziel-/Policyvertrag"))); }
            if data.complete { return Err(io::Error::other(format!("Auftrag {id} ist bereits abgeschlossen"))); }
            verify_target(view, &data)?;
            if !data.mutation_started { super::file_recovery::initialize(&path, &data, context.as_ref())?; }
            let receipts = super::file_recovery::verify_saved(view, &data, &path, context.as_ref(), cancel)?;
            let completed = verify_sources(view, &data, &receipts)?;
            let files = receipts.into_values().map(|(_, root)| root).collect();
            (data, completed, files)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if view.recovery == crate::transfer::RecoveryMode::Resume { return Err(io::Error::other("Gespeicherter Auftrag fehlt; Wiederaufnahme bleibt unbestätigt")); }
            let mut data = expected;
            data.target_object = with_backend(view.target, |backend| crate::vfs::directory_identity(backend, target_root)).ok().flatten();
            let sources = source_paths(view.items);
            data.guards = sources.into_iter().map(|path| snapshot(view.source, path)).collect();
            write_record(&path, RecordKind::Job, id, context.as_ref(), &data)?;
            (data, Vec::new(), Vec::new())
        }
        Err(error) => return Err(error),
    };
    let mut data = data;
    if !data.file_journal {
        for guard in &mut data.guards {
            if !guard.directory && guard.object.is_none() { *guard = snapshot(view.source, &guard.path); }
        }
    }
    data.file_journal = true;
    super::file_recovery::initialize(&path, &data, context.as_ref())?;
    Ok(Some(JobRun { path, _lock: lock, data: Mutex::new(data), context, completed, files }))
}

fn verify_target(view: &JobView<'_>, record: &JobRecord) -> io::Result<()> {
    if record.target != endpoint(view.target) { return Err(io::Error::other("Zielverbindung des gespeicherten Auftrags stimmt nicht überein")); }
    let actual = with_backend(view.target, |backend| crate::vfs::directory_identity(backend, &record.target_dir))?;
    if record.target_object.is_none() || record.target_object != actual { return Err(io::Error::other("Gespeicherter Zielroot ist geändert oder unbestätigt")); }
    Ok(())
}

pub(super) fn preflight(view: &JobView<'_>) -> io::Result<Option<String>> {
    if !view.recoverable { return Ok(None); }
    let Some(id) = view.id else { return Ok(None); };
    let locations = record_io::record_locations(view.source, view.target, &source_paths(view.items), view.target_dir)?;
    let context = record_context(&locations)?;
    let record = match read_record::<JobRecord>(&path(id)?, RecordKind::Job, id, u64::MAX, context.is_some()) { Ok(record) => record, Err(error) if error.kind() == io::ErrorKind::NotFound && view.recovery == crate::transfer::RecoveryMode::New => return Ok(None), Err(error) => return Err(error) };
    if view.recovery == crate::transfer::RecoveryMode::New { return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Job-ID ist bereits vergeben")); }
    let requested = if view.target_dir == record.target_dir { view.target_dir.to_string() }
        else if view.target.is_local() && !view.source.is_local() { super::folders::existing_target_root(view.target_dir)? }
        else { return Err(io::Error::other("Wiederaufnahme hat einen abweichenden Zielroot")); };
    if requested != record.target_dir { return Err(io::Error::other("Wiederaufnahme verweist auf einen geänderten Zielroot")); }
    verify_target(view, &record)?;
    Ok(Some(record.target_dir))
}

fn verify_sources(view: &JobView<'_>, record: &JobRecord, files: &super::file_recovery::FileRoots) -> io::Result<Vec<crate::transfer::ResolvedRoot>> {
    let mut completed = Vec::new();
    let paths = source_paths(view.items);
    if record.guards.len() != paths.len() || record.guards.iter().zip(&paths).any(|(guard, path)| guard.path != *path) { return Err(io::Error::new(io::ErrorKind::InvalidData, "Gespeicherte Quellbelege decken die Auswahl nicht vollständig ab")); }
    for (index, before) in record.guards.iter().enumerate() {
        if !before.verified { return Err(io::Error::other(format!("Quellbeleg von Auftrag {} ist unbestätigt", record.id))); }
        let after = snapshot(view.source, &before.path);
        if record.moving && !after.verified {
            if !before.directory {
                let selected = match &record.items { JobItems::Pairs(pairs) => format!("{}\0{}", pairs[index].source, pairs[index].rel), _ => before.path.clone() };
                if let Some((_, root)) = files.get(&selected).filter(|(guard, _)| guard == before) { completed.push(root.clone()); continue; }
            }
            if let Some(relative) = confirmed_move(view, record, before)? { completed.push(crate::transfer::ResolvedRoot { source: before.path.clone(), rel: relative }); continue; }
        }
        let same = after.verified && before.directory == after.directory && before.link == after.link && before.id == after.id
            && if before.directory { before.object.is_some() && before.object == after.object || before.id.is_some() && before.id == after.id }
            else { before.size == after.size && before.mtime_ms == after.mtime_ms && (before.object.is_none() || before.object == after.object) };
        if !same { return Err(io::Error::other(format!("Quellidentität von Auftrag {} ist geändert oder unbestätigt", record.id))); }
    }
    Ok(completed)
}

fn confirmed_move(view: &JobView<'_>, record: &JobRecord, guard: &SourceGuard) -> io::Result<Option<String>> {
    let path = crate::support_dirs::app_data_file("transfer-aliases").join(format!("{}.json", record.id));
    let context = record_context(&record.vault_locations)?;
    let aliases: super::super::super::folder_alias::FolderAliases = match read_record(&path, RecordKind::Alias, &record.id, u64::MAX, context.is_some()) {
        Ok(aliases) => aliases,
        Err(error) if error.kind() == io::ErrorKind::NotFound && record.source.local && record.target.local && record.filter.is_none() && record.layout == crate::transfer::Layout::Tree && record.folders.for_root(&guard.path) == crate::transfer::FolderConflict::Merge => {
            let JobItems::Roots { base, .. } = &record.items else { return Ok(None); };
            let relative = super::super::engine_names::root_rel(&guard.path, base.as_deref());
            let actual = with_backend(view.target, |backend| crate::vfs::directory_identity(backend, &super::super::engine_names::join_rel(&record.target_dir, &relative)))?;
            return Ok((guard.object.is_some() && guard.object == actual).then_some(relative));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None), Err(error) => return Err(error),
    };
    super::folder_alias_store::validate_aliases(&aliases)?;
    if aliases.scope.job != record.id || aliases.scope.native_target != record.target.local || aliases.scope.vault_locations != record.vault_locations || aliases.scope.source_namespace != record.source.namespace || aliases.scope.target_namespace != record.target.namespace || aliases.scope.target_root != record.target_dir { return Ok(None); }
    let Some(alias) = aliases.roots.get(&guard.path).filter(|alias| alias.move_source && alias.identity.is_some()) else { return Ok(None); };
    if guard.object != alias.identity { return Ok(None); }
    with_backend(view.target, |backend| crate::vfs::directory_identity(backend, &super::super::engine_names::join_rel(&record.target_dir, &alias.relative)))
        .map(|identity| (identity == alias.identity).then(|| alias.relative.clone()))
}

impl JobRun {
    pub fn completed_roots(&self) -> &[crate::transfer::ResolvedRoot] { &self.completed }
    pub fn saved_file_roots(&self) -> &[crate::transfer::ResolvedRoot] { &self.files }
    pub fn prepare_apply(&self) -> io::Result<bool> {
        let mut data = self.data.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let required = data.mutation_started && data.guards.iter().any(|guard| guard.directory && !matches!(data.folders.for_root(&guard.path), crate::transfer::FolderConflict::Merge | crate::transfer::FolderConflict::Skip));
        if !data.mutation_started { data.mutation_started = true; write_record(&self.path, RecordKind::Job, &data.id, self.context.as_ref(), &*data)?; }
        Ok(required)
    }
    pub fn confirm_sources(&self, view: &JobView<'_>) -> io::Result<()> {
        let mut data = self.data.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = false;
        if data.target_object.is_none() {
            let actual = with_backend(view.target, |backend| crate::vfs::directory_identity(backend, &data.target_dir)).ok().flatten();
            if actual.is_some() { data.target_object = actual; changed = true; }
        }
        for guard in &mut data.guards {
            if !guard.verified {
                let current = snapshot(view.source, &guard.path);
                if current.verified { *guard = current; changed = true; }
            }
        }
        if changed { write_record(&self.path, RecordKind::Job, &data.id, self.context.as_ref(), &*data)?; }
        Ok(())
    }

    pub fn finish(&self, complete: bool) -> io::Result<()> {
        let mut data = self.data.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        data.complete = complete;
        write_record(&self.path, RecordKind::Job, &data.id, self.context.as_ref(), &*data)
    }
}

pub(crate) fn unconfirmed_jobs() -> io::Result<Vec<JobRecord>> {
    let entries = match std::fs::read_dir(root()) { Ok(entries) => entries, Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()), Err(error) => return Err(error) };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")).filter(|id| valid_id(id)) else { continue; };
        let record: JobRecord = read_record(&path(id)?, RecordKind::Job, id, u64::MAX, false)?;
        if record.version == 1 && record.id == id && !record.complete { records.push(record); }
    }
    records.sort_by(|one, two| one.id.cmp(&two.id));
    Ok(records)
}

pub(crate) fn reopen_job(id: &str, source: Endpoint, target: Endpoint) -> Result<TransferJob, String> {
    let record: JobRecord = read_record(&path(id).map_err(|error| error.to_string())?, RecordKind::Job, id, u64::MAX, false).map_err(|error| error.to_string())?;
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
    let entries = match std::fs::read_dir(root()) { Ok(entries) => entries, Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((CatalogState { ready: true, ..Default::default() }, Vec::new())), Err(error) => return Err(error) };
    let mut state = CatalogState { ready: true, ..Default::default() };
    let mut contexts = Vec::new();
    for entry in entries {
        let entry = entry?; let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")).filter(|id| valid_id(id)) else { continue; };
        let loaded = (|| {
            let (record, sealed): (JobRecord, bool) = record_io::read_record_state(&path(id)?, RecordKind::Job, id, u64::MAX, false)?;
            if record.version != 1 || record.id != id { return Err(io::Error::other("Ungültiger Jobdatensatz")); }
            if record.complete { return Ok(None); }
            let context = record_context(&record.vault_locations)?;
            if sealed != context.is_some() { return Err(io::Error::other("Vaultbindung des Jobdatensatzes ist unbestätigt")); }
            if let Some(context) = &context { crate::vault::check_transfer_blob_context_live(context)?; }
            Ok(Some((record, context)))
        })();
        match loaded {
            Ok(Some((record, context))) => {
                if let Some(context) = context { contexts.push((id.to_string(), context)); }
                state.records.push(record);
            }
            Ok(None) => {},
            Err(_) => state.unavailable.push(id.to_string()),
        }
    }
    if !state.unavailable.is_empty() { state.error = Some(format!("Gesperrte oder unbestätigte Job-IDs: {}", state.unavailable.join(", "))); }
    Ok((state, contexts))
}

pub(crate) struct RecoveryCatalog(std::sync::Arc<Mutex<CatalogState>>);

#[cfg(test)]
#[path = "file_recovery_tests.rs"]
mod file_recovery_tests;
impl RecoveryCatalog {
    pub fn start() -> io::Result<Self> {
        let state = std::sync::Arc::new(Mutex::new(CatalogState::default()));
        let background = state.clone();
        std::thread::Builder::new().name("transfer-recovery-catalog".into()).spawn(move || {
            let (result, contexts) = match load_catalog() { Ok(loaded) => loaded, Err(error) => (CatalogState { ready: true, error: Some(error.to_string()), ..Default::default() }, Vec::new()) };
            *background.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = result;
            let weak = std::sync::Arc::downgrade(&background); drop(background);
            record_io::monitor_contexts(weak, contexts, |state, id| {
                state.records.retain(|record| record.id != id);
                if !state.unavailable.iter().any(|record| record == id) { state.unavailable.push(id.to_string()); }
                state.error = Some(format!("Gesperrte oder unbestätigte Job-IDs: {}", state.unavailable.join(", ")));
            });
        })?;
        Ok(Self(state))
    }
    pub fn snapshot(&self) -> CatalogState { self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone() }
}
