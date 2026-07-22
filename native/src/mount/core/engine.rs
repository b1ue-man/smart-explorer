use super::case_semantics::{identity_key, validate_backend_case_path};
use super::journal::{DeletePhase, PersistedDelete, PersistedEntry};
use super::metadata_cache::MetadataCache;
use super::metadata_point_cache::MetadataPointCache;
use super::path::{validate_windows_component, PathProjector, ProjectedPath};
use super::spool::{prepare_spool_root, WholeFileSpool};
use super::startup::validate_backend_root;
use super::types::{
    BackendRoot, Baseline, DeleteToken, EntryCondition, HandleId, MountConfig, MountConflict,
    MountRuntimeConfig, NamespaceIntent,
};
use crate::vfs::{BackendHandle, VfsMeta};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};

pub struct MountEngine {
    pub(super) config: MountRuntimeConfig,
    pub(super) backend: BackendHandle,
    pub(super) case_sensitive_paths: bool,
    pub(super) verify_backend_ancestors: bool,
    pub(super) projector: PathProjector,
    pub(super) spool: Arc<WholeFileSpool>,
    pub(super) namespace: RwLock<()>,
    pub(super) materializations: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    pub(super) entries: Mutex<HashMap<String, Arc<Entry>>>,
    pub(super) handles: Mutex<HashMap<HandleId, OpenHandle>>,
    pub(super) deletes: Mutex<HashMap<DeleteToken, PersistedDelete>>,
    pub(super) namespace_conflicts: Mutex<HashMap<String, NamespaceIntent>>,
    pub(super) metadata_cache: MetadataCache,
    pub(super) metadata_points: MetadataPointCache,
    pub(super) metadata_epoch: AtomicU64,
    pub(super) next_handle: AtomicU64,
    pub(super) next_delete: AtomicU64,
}

pub(super) struct Entry {
    pub state: Mutex<EntryState>,
}

pub(super) struct EntryState {
    pub remote_path: String,
    pub spool_name: String,
    pub baseline: Baseline,
    pub condition: EntryCondition,
    pub delete_token: Option<u64>,
    pub delete_committed: bool,
}

pub(super) struct OpenHandle {
    pub entry: Arc<Entry>,
    pub writable: bool,
}

impl MountEngine {
    pub fn open(
        config: MountConfig,
        backend: BackendHandle,
        spool_root: impl AsRef<Path>,
    ) -> io::Result<Self> {
        config.validate()?;
        let root = config.source.root().clone();
        Self::open_at_root(
            config.runtime(),
            root,
            backend,
            spool_root.as_ref(),
            true,
            true,
        )
    }

    pub fn config(&self) -> &MountRuntimeConfig {
        &self.config
    }

    pub fn dirty_entries(&self) -> io::Result<Vec<(String, EntryCondition)>> {
        let _namespace = read_lock(&self.namespace)?;
        let entries = lock(&self.entries)?.values().cloned().collect::<Vec<_>>();
        let mut dirty = HashMap::new();
        for entry in entries {
            let state = lock(&entry.state)?;
            if state.condition != EntryCondition::Clean {
                dirty.insert(
                    self.cache_key(&state.remote_path),
                    (state.remote_path.clone(), state.condition.clone()),
                );
            }
        }
        let deletes = lock(&self.deletes)?.values().cloned().collect::<Vec<_>>();
        for delete in deletes {
            let path = delete.original_path;
            dirty.insert(
                self.cache_key(&path),
                (
                    path.clone(),
                    EntryCondition::Conflict(MountConflict {
                        path,
                        baseline: Baseline::Missing,
                        current: None,
                        detail: "remote delete transaction remains unresolved; preserve the mount cache and retry or resolve the original and quarantine paths"
                            .into(),
                    }),
                ),
            );
        }
        for intent in lock(&self.namespace_conflicts)?.values() {
            let conflict = &intent.conflict;
            dirty.insert(
                self.cache_key(&conflict.path),
                (
                    conflict.path.clone(),
                    EntryCondition::Conflict(conflict.clone()),
                ),
            );
        }
        let mut dirty = dirty.into_values().collect::<Vec<_>>();
        dirty.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(dirty)
    }

    pub fn evict_clean(&self, callback_path: &str) -> io::Result<bool> {
        let _namespace = write_lock(&self.namespace)?;
        let path = self.projector.project(callback_path)?;
        self.validate_projected_case(&path)?;
        let key = self.cache_key(path.backend());
        let Some(entry) = self.entry_for_path(path.backend())? else {
            return Ok(false);
        };
        if lock(&self.handles)?
            .values()
            .any(|opened| Arc::ptr_eq(&opened.entry, &entry))
        {
            return Ok(false);
        }
        let state = lock(&entry.state)?;
        if state.condition != EntryCondition::Clean || state.delete_token.is_some() {
            return Ok(false);
        }
        let spool_name = state.spool_name.clone();
        drop(state);
        let mut entries = lock(&self.entries)?;
        if !entries
            .get(&key)
            .is_some_and(|current| Arc::ptr_eq(current, &entry))
        {
            return Ok(false);
        }
        entries.remove(&key);
        drop(entries);
        self.spool.remove_file(&spool_name)?;
        Ok(true)
    }

    pub(super) fn project_checked(&self, callback_path: &str) -> io::Result<ProjectedPath> {
        let projected = self.projector.project(callback_path)?;
        self.validate_projected_case(&projected)?;
        if !self.verify_backend_ancestors {
            return Ok(projected);
        }
        let root = self.backend.stat(self.projector.root().as_str())?;
        if !root.is_dir || root.is_symlink {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "configured mount root is not a plain directory",
            ));
        }
        for ancestor in self.projector.ancestor_paths(&projected) {
            let meta = self.backend.stat(&ancestor)?;
            if !meta.is_dir || meta.is_symlink {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "mounted path crosses a non-directory or link-like ancestor",
                ));
            }
        }
        Ok(projected)
    }

    pub(super) fn entry_for_path(&self, remote_path: &str) -> io::Result<Option<Arc<Entry>>> {
        let entry = lock(&self.entries)?
            .get(&self.cache_key(remote_path))
            .cloned();
        if let Some(entry) = entry.as_ref() {
            let state = lock(&entry.state)?;
            if state.baseline != Baseline::Missing {
                self.verify_unique_cached_alias(remote_path, &state.remote_path)?;
            }
        }
        Ok(entry)
    }

    pub(super) fn open_at_root(
        config: MountRuntimeConfig,
        root: BackendRoot,
        backend: BackendHandle,
        spool_root: &Path,
        verify_backend_ancestors: bool,
        validate_remote: bool,
    ) -> io::Result<Self> {
        config.metadata.validate()?;
        let projector = PathProjector::new(root);
        let case_sensitive_paths = backend.case_sensitive_paths(projector.root().as_str());
        if validate_remote {
            validate_backend_root(&*backend, projector.root().as_str())?;
        }
        let spool_root = prepare_spool_root(spool_root)?;
        let (spool, recovered) = WholeFileSpool::open(&spool_root, &config.id)?;
        let spool = Arc::new(spool);
        let mut entries = HashMap::new();
        let mut spool_names = HashSet::new();
        for persisted in recovered.entries.into_values() {
            validate_recovered_path(projector.root().as_str(), &persisted.remote_path, false)?;
            validate_backend_case_path(
                case_sensitive_paths,
                projector.root().as_str(),
                &persisted.remote_path,
            )?;
            if persisted.condition == EntryCondition::Clean && persisted.delete_token.is_none() {
                return Err(invalid_data(
                    "clean journal entry lacks a pending delete transaction",
                ));
            }
            if !spool_names.insert(persisted.spool_name.clone()) {
                return Err(invalid_data("two journal entries reference one spool file"));
            }
            let key = identity_key(case_sensitive_paths, &persisted.remote_path);
            if entries
                .insert(
                    key,
                    Arc::new(Entry {
                        state: Mutex::new(EntryState::from_persisted(persisted)),
                    }),
                )
                .is_some()
            {
                return Err(invalid_data(
                    "journal contains case-aliasing mount cache entries",
                ));
            }
        }
        let next_delete = recovered
            .deletes
            .keys()
            .copied()
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid_data("delete token space exhausted"))?;
        let mut delete_targets = HashSet::new();
        for (token, delete) in &recovered.deletes {
            validate_recovered_delete(projector.root().as_str(), *token, delete)?;
            validate_backend_case_path(
                case_sensitive_paths,
                projector.root().as_str(),
                &delete.original_path,
            )?;
            if !delete.quarantine_path.is_empty() {
                validate_backend_case_path(
                    case_sensitive_paths,
                    projector.root().as_str(),
                    &delete.quarantine_path,
                )?;
            }
            if !delete_targets.insert(identity_key(case_sensitive_paths, &delete.original_path)) {
                return Err(invalid_data(
                    "journal contains case-aliasing delete transactions",
                ));
            }
        }
        let deletes = recovered
            .deletes
            .into_iter()
            .map(|(token, delete)| (DeleteToken(token), delete))
            .collect::<HashMap<_, _>>();
        let mut namespace_conflicts = HashMap::new();
        for intent in recovered.namespace_conflicts.into_values() {
            let conflict = &intent.conflict;
            validate_recovered_path(projector.root().as_str(), &conflict.path, false)?;
            validate_backend_case_path(
                case_sensitive_paths,
                projector.root().as_str(),
                &conflict.path,
            )?;
            let key = identity_key(case_sensitive_paths, &conflict.path);
            if let Some(source) = intent.source_path.as_deref() {
                validate_recovered_path(projector.root().as_str(), source, false)?;
                validate_backend_case_path(
                    case_sensitive_paths,
                    projector.root().as_str(),
                    source,
                )?;
            }
            if namespace_conflicts.insert(key, intent).is_some() {
                return Err(invalid_data(
                    "journal contains case-aliasing namespace conflicts",
                ));
            }
        }
        for entry in entries.values() {
            let state = lock(&entry.state)?;
            if let Some(token) = state.delete_token {
                let delete = deletes.get(&DeleteToken(token)).ok_or_else(|| {
                    invalid_data("journal entry references an unknown delete token")
                })?;
                if delete.original_path != state.remote_path {
                    return Err(invalid_data(
                        "journal entry and delete transaction target different paths",
                    ));
                }
            }
        }
        let metadata_cache = MetadataCache::new(projector.root().as_str(), case_sensitive_paths);
        let metadata_points = MetadataPointCache::new(case_sensitive_paths);
        let engine = Self {
            config,
            backend,
            case_sensitive_paths,
            verify_backend_ancestors,
            projector,
            spool,
            namespace: RwLock::new(()),
            materializations: Mutex::new(HashMap::new()),
            entries: Mutex::new(entries),
            handles: Mutex::new(HashMap::new()),
            deletes: Mutex::new(deletes),
            namespace_conflicts: Mutex::new(namespace_conflicts),
            metadata_cache,
            metadata_points,
            metadata_epoch: AtomicU64::new(0),
            next_handle: AtomicU64::new(1),
            next_delete: AtomicU64::new(next_delete),
        };
        if validate_remote {
            engine.recover_pending_deletes()?;
        }
        Ok(engine)
    }

    pub(super) fn materialization_guard(&self, path: &str) -> io::Result<Arc<Mutex<()>>> {
        let mut materializations = lock(&self.materializations)?;
        materializations.retain(|_, guard| guard.strong_count() > 0);
        let key = self.cache_key(path);
        if let Some(guard) = materializations.get(&key).and_then(Weak::upgrade) {
            return Ok(guard);
        }
        let guard = Arc::new(Mutex::new(()));
        materializations.insert(key, Arc::downgrade(&guard));
        Ok(guard)
    }
}

impl EntryState {
    pub(super) fn persisted(&self) -> PersistedEntry {
        PersistedEntry {
            remote_path: self.remote_path.clone(),
            spool_name: self.spool_name.clone(),
            baseline: self.baseline.clone(),
            condition: self.condition.clone(),
            delete_token: self.delete_token,
        }
    }

    pub(super) fn with_condition(&self, condition: EntryCondition) -> PersistedEntry {
        let mut persisted = self.persisted();
        persisted.condition = condition;
        persisted
    }

    fn from_persisted(entry: PersistedEntry) -> Self {
        Self {
            remote_path: entry.remote_path,
            spool_name: entry.spool_name,
            baseline: entry.baseline,
            condition: entry.condition,
            delete_token: entry.delete_token,
            delete_committed: false,
        }
    }
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> io::Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "mount engine lock is poisoned"))
}

pub(super) fn read_lock<T>(lock: &RwLock<T>) -> io::Result<RwLockReadGuard<'_, T>> {
    lock.read()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "mount namespace lock is poisoned"))
}

pub(super) fn write_lock<T>(lock: &RwLock<T>) -> io::Result<RwLockWriteGuard<'_, T>> {
    lock.write()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "mount namespace lock is poisoned"))
}

pub(super) fn baseline_from_meta(meta: &VfsMeta) -> Baseline {
    Baseline::Present {
        id: meta.id.clone(),
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        content_md5: meta.content_md5.clone(),
    }
}

pub(super) fn require_regular(meta: &VfsMeta) -> io::Result<()> {
    if meta.is_dir || meta.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "mounted file must be a plain regular file",
        ));
    }
    Ok(())
}

fn require_below_root(root: &str, path: &str) -> io::Result<()> {
    let valid = path == root
        || (root == "/" && path.starts_with('/') && !path.starts_with("//"))
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'));
    if !valid || path.split('/').any(|part| matches!(part, "." | "..")) {
        return Err(invalid_data(
            "journal path escapes the configured backend root",
        ));
    }
    Ok(())
}

fn validate_recovered_path(root: &str, path: &str, allow_root: bool) -> io::Result<()> {
    require_below_root(root, path)?;
    if path == root {
        return if allow_root {
            Ok(())
        } else {
            Err(invalid_data("journal entry may not target the mount root"))
        };
    }
    let suffix = path
        .strip_prefix(root)
        .ok_or_else(|| invalid_data("journal path is outside the mount root"))?
        .trim_start_matches('/');
    for component in suffix.split('/') {
        validate_windows_component(component).map_err(|error| invalid_data(error.to_string()))?;
    }
    Ok(())
}

fn validate_recovered_delete(root: &str, token: u64, delete: &PersistedDelete) -> io::Result<()> {
    if token == 0 || delete.token != token {
        return Err(invalid_data("delete journal token mismatch"));
    }
    validate_recovered_path(root, &delete.original_path, false)?;
    if delete.phase == DeletePhase::LocalOnly {
        if delete.is_directory || !delete.quarantine_path.is_empty() {
            return Err(invalid_data("invalid local-only delete journal entry"));
        }
        return Ok(());
    }
    validate_recovered_path(root, &delete.quarantine_path, false)?;
    let suffix = delete
        .quarantine_path
        .strip_prefix(&delete.original_path)
        .and_then(|suffix| suffix.strip_prefix(".se-mount-delete-"))
        .ok_or_else(|| invalid_data("delete quarantine is not an exact generated sibling"))?;
    if suffix.len() != 16 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_data("delete quarantine suffix is invalid"));
    }
    Ok(())
}

pub(super) fn parent_path(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => "",
    }
}

pub(super) fn not_found(path: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("mounted path not found: {path}"),
    )
}

pub(super) fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
