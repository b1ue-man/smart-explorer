//! Pins cover both published handles and every cloned in-flight operation.
use super::clean_cache::IdleClean;
use super::engine::{lock, write_lock, Entry, MountEngine};
use super::types::EntryCondition;
use std::{io, ops::Deref, sync::{Arc, TryLockError, atomic::Ordering}};

pub(super) struct EntryPin(Arc<Entry>);

pub(super) struct OperationReaper<'a>(&'a MountEngine);
impl Drop for OperationReaper<'_> {
    fn drop(&mut self) { self.0.reap_after_operation(); }
}

impl EntryPin {
    /// Caller holds namespace ownership or an already-pinned handle.
    pub fn new(entry: Arc<Entry>) -> Self {
        entry.pins.fetch_add(1, Ordering::AcqRel);
        Self(entry)
    }
    pub fn release(self) -> Arc<Entry> {
        let entry = Arc::clone(&self.0);
        drop(self);
        entry
    }
}

impl Clone for EntryPin {
    fn clone(&self) -> Self { Self::new(Arc::clone(&self.0)) }
}
impl Deref for EntryPin {
    type Target = Arc<Entry>;
    fn deref(&self) -> &Self::Target { &self.0 }
}
impl Drop for EntryPin {
    fn drop(&mut self) {
        if self.0.pins.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.retirement_pending.store(true, Ordering::Release);
        }
    }
}

impl MountEngine {
    /// Called under namespace write ownership before replacing a pathname.
    /// A lazy handle cannot later be allowed to read the new namespace object.
    pub(super) fn preserve_lazy_destination(&self, path: &super::path::ProjectedPath,
        shared_destination_is_open: bool) -> io::Result<()> {
        let lazy = {
            let handles = lock(&self.handles)?;
            let mut lazy = Vec::new();
            for (id, handle) in handles.iter() {
                if let super::open_handle::OpenHandleKind::Metadata { callback_path, meta } = &handle.kind {
                    let opened_path = self.projector.project(callback_path)?;
                    if self.paths_equal(opened_path.backend(), path.backend()) {
                        lazy.push((*id, meta.clone()));
                    }
                }
            }
            lazy
        };
        if lazy.is_empty() { return Ok(()); }
        if !shared_destination_is_open {
            return Err(io::Error::new(io::ErrorKind::WouldBlock,
                "a lazy destination handle prevents replacing rename without delete sharing"));
        }
        let entry = self.materialize(path, super::types::OpenDisposition::OpenExisting)?;
        {
            let state = lock(&entry.state)?;
            for (_, meta) in &lazy {
                let same_spool = meta.id.as_deref() == Some(format!("mount-cache:{}", state.spool_name).as_str());
                if !same_spool && super::engine::baseline_from_meta(meta) != state.baseline {
                    return Err(io::Error::new(io::ErrorKind::WouldBlock,
                        "lazy destination baseline changed; refusing to replace its unpreserved object"));
                }
            }
        }
        let mut handles = lock(&self.handles)?;
        for (id, _) in lazy {
            if let Some(handle) = handles.get_mut(&id) {
                if matches!(handle.kind, super::open_handle::OpenHandleKind::Metadata { .. }) {
                    handle.kind = super::open_handle::OpenHandleKind::Materialized(EntryPin::new(Arc::clone(&entry)));
                }
            }
        }
        Ok(())
    }

    pub(super) fn operation_reaper(&self) -> OperationReaper<'_> { OperationReaper(self) }

    pub(super) fn reap_after_operation(&self) {
        if matches!(self.reap_unpinned(), Ok(true)) {
            let _ = self.clean_cache.trim(&self.spool, self.config.cache.retained_bytes());
        }
    }

    fn reap_unpinned(&self) -> io::Result<bool> {
        if !self.retirement_pending.swap(false, Ordering::AcqRel) { return Ok(false); }
        let _namespace = match self.namespace.try_write() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                self.retirement_pending.store(true, Ordering::Release);
                return Ok(false);
            }
            Err(TryLockError::Poisoned(_)) => return Err(io::Error::other("mount namespace poisoned")),
        };
        let mut entries = lock(&self.entries)?.values().cloned().collect::<Vec<_>>();
        let detached = lock(&self.detached)?.values().cloned().collect::<Vec<_>>();
        for entry in detached {
            if !entries.iter().any(|live| Arc::ptr_eq(live, &entry)) { entries.push(entry); }
        }
        for entry in entries {
            if entry.pins.load(Ordering::Acquire) == 0 {
                if let Err(error) = self.cleanup_committed_entry(&entry) {
                    self.retirement_pending.store(true, Ordering::Release);
                    return Err(error);
                }
            }
        }
        Ok(true)
    }

    pub fn maintain_cache(&self) -> io::Result<()> {
        self.reap_unpinned()?;
        self.clean_cache.trim(&self.spool, self.config.cache.retained_bytes())?;
        self.maintain_space()
    }

    /// Caller owns namespace write protection. Pins cannot appear from a path
    /// lookup during retirement, and a zero count proves no handle can clone one.
    pub(super) fn cleanup_committed_entry(&self, entry: &Arc<Entry>) -> io::Result<()> {
        if entry.pins.load(Ordering::Acquire) != 0 { return Ok(()); }
        let mut state = lock(&entry.state)?;
        // A close can retain an unpinned Arc while another maintenance pass
        // retires it and a new Entry adopts its spool. Never dispose twice.
        if state.retired { return Ok(()); }
        if !state.delete_committed
            && (state.condition != EntryCondition::Clean || state.delete_token.is_some()) {
            return Ok(());
        }
        // An append/forget can be durable despite returning an error. In-memory
        // Clean alone is never authority to discard a recovery-referenced spool.
        if self.spool.is_recovery_referenced(&state.spool_name)? { return Ok(()); }
        let key = self.cache_key(&state.remote_path);
        let mut entries = lock(&self.entries)?;
        let mut detached = lock(&self.detached)?;
        let attached = entries.get(&key)
            .is_some_and(|current| Arc::ptr_eq(current, entry));
        if !state.delete_committed && attached {
            let bytes = self.spool.open_file(&state.spool_name, false)?.metadata()?.len();
            let mut record = IdleClean::new(state.remote_path.clone(), key.clone(),
                state.spool_name.clone(), state.baseline.clone(), bytes, state.clean_since);
            let limit = self.config.cache.retained_bytes();
            record.reusable = limit != 0 && bytes <= limit
                && record.created.elapsed() < super::clean_cache::MAX_CONTENT_AGE;
            self.clean_cache.retain(record)?;
        } else {
            self.spool.remove_file(&state.spool_name)?;
        }
        if attached { entries.remove(&key); }
        detached.remove(&state.spool_name);
        state.retired = true;
        Ok(())
    }

    pub fn evict_clean(&self, callback_path: &str) -> io::Result<bool> {
        let _namespace = write_lock(&self.namespace)?;
        let path = self.projector.project(callback_path)?;
        self.validate_projected_case(&path)?;
        if let Some(entry) = self.entry_for_path(path.backend())? {
            if entry.pins.load(Ordering::Acquire) != 0 { return Ok(false); }
            self.cleanup_committed_entry(&entry)?;
        }
        self.clean_cache.evict_path(&self.spool, &self.cache_key(path.backend()))
    }
}
