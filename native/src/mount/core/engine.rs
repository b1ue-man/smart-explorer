use super::journal::{PersistedDelete, PersistedEntry};
use super::metadata_cache::MetadataCache;
use super::metadata_point_cache::MetadataPointCache;
pub(super) use super::open_handle::{OpenHandle, OpenHandleKind};
use super::path::{PathProjector, ProjectedPath};
use super::spool::WholeFileSpool;
use super::types::{
    Baseline, DeleteToken, EntryCondition, HandleId, MountConfig, MountConflict,
    MountRuntimeConfig, NamespaceIntent,
};
use crate::vfs::{BackendHandle, VfsMeta};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};

pub struct MountEngine {
    pub(super) config: MountRuntimeConfig,
    pub(super) backend: BackendHandle,
    pub(super) case_sensitive_paths: bool,
    pub(super) verify_backend_ancestors: bool,
    pub(super) projector: PathProjector,
    pub(super) spool: Arc<WholeFileSpool>,
    pub(super) namespace: RwLock<()>,
    pub(super) materializations: Mutex<HashMap<String, Weak<super::materialization::MaterializationSlot>>>,
    pub(super) entries: Mutex<HashMap<String, Arc<Entry>>>,
    pub(super) handles: Mutex<HashMap<HandleId, OpenHandle>>,
    pub(super) deletes: Mutex<HashMap<DeleteToken, PersistedDelete>>,
    pub(super) namespace_conflicts: Mutex<HashMap<String, NamespaceIntent>>,
    pub(super) metadata_cache: MetadataCache,
    pub(super) metadata_points: MetadataPointCache,
    pub(super) metadata_epoch: AtomicU64,
    pub(super) next_handle: AtomicU64,
    pub(super) next_delete: AtomicU64,
    pub(super) clean_cache: super::clean_cache::CleanCache,
    pub(super) cache_space: Arc<super::cache_space::CacheSpace>,
    pub(super) detached: Mutex<HashMap<String, Arc<Entry>>>,
    pub(super) retirement_pending: Arc<AtomicBool>,
}

pub(super) struct Entry {
    pub state: Mutex<EntryState>,
    pub(super) pins: AtomicUsize,
    pub(super) retirement_pending: Arc<AtomicBool>,
}

pub(super) struct EntryState {
    pub remote_path: String,
    pub spool_name: String,
    pub baseline: Baseline,
    pub condition: EntryCondition,
    pub delete_token: Option<u64>,
    pub delete_committed: bool,
    pub(super) clean_since: std::time::Instant,
    /// Ownership was transferred to the idle index or the spool was disposed.
    pub(super) retired: bool,
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

    pub(super) fn from_persisted(entry: PersistedEntry) -> Self {
        Self {
            remote_path: entry.remote_path,
            spool_name: entry.spool_name,
            baseline: entry.baseline,
            condition: entry.condition,
            delete_token: entry.delete_token,
            delete_committed: false,
            clean_since: std::time::Instant::now(),
            retired: false,
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
