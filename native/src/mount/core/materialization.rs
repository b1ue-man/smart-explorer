//! Single-flight content acquisition with an owned, unpublished spool candidate.
use super::clean_cache::{affected, IdleClean, MAX_CONTENT_AGE};
use super::engine::{baseline_from_meta, invalid_data, lock, not_found, read_lock,
    require_regular, Entry, EntryState, MountEngine};
use super::entry_lifecycle::EntryPin;
use super::path::ProjectedPath;
use super::types::{Baseline, EntryCondition, OpenDisposition};
use std::{io::{self, Read}, sync::{Arc, Mutex, Weak, atomic::{AtomicU64, AtomicUsize, Ordering}},
    time::Instant};

#[derive(Default)]
pub(super) struct MaterializationSlot {
    gate: Mutex<()>,
    generation: AtomicU64,
}

struct PreparedMaterialization<'a> {
    engine: &'a MountEngine,
    record: Option<IdleClean>,
    condition: EntryCondition,
}

impl Drop for PreparedMaterialization<'_> {
    fn drop(&mut self) {
        if let Some(mut record) = self.record.take() {
            // Failed disposal is not forgotten: it remains owned/accounted but
            // cannot be reused, even when the original download was incomplete.
            if self.engine.spool.remove_file(&record.spool_name).is_err() {
                if let Ok(metadata) = self.engine.spool.open_file(&record.spool_name, false)
                    .and_then(|file| file.metadata()) {
                    record.bytes = metadata.len();
                }
                record.reusable = false;
                let _ = self.engine.clean_cache.retain(record);
            }
        }
    }
}

impl MountEngine {
    fn materialization_guard(&self, path: &str) -> io::Result<Arc<MaterializationSlot>> {
        let mut slots = lock(&self.materializations)?;
        slots.retain(|_, slot| slot.strong_count() > 0);
        let key = self.cache_key(path);
        if let Some(slot) = slots.get(&key).and_then(Weak::upgrade) { return Ok(slot); }
        let slot = Arc::new(MaterializationSlot::default());
        slots.insert(key, Arc::downgrade(&slot));
        Ok(slot)
    }

    /// Mutation callers invalidate BEFORE dispatch while holding namespace
    /// write ownership. Never acquire a slot gate here (the opposite lock order).
    pub(super) fn invalidate_content(&self, path: &str, recursive: bool) {
        let key = self.cache_key(path);
        if let Ok(slots) = self.materializations.lock() {
            for (_, slot) in slots.iter().filter(|(path, _)| affected(path, &key, recursive)) {
                if let Some(slot) = slot.upgrade() {
                    slot.generation.fetch_add(1, Ordering::AcqRel);
                }
            }
        }
        let _ = self.clean_cache.invalidate(&key, recursive);
    }

    pub(super) fn materialize_at(&self, callback_path: &str,
        disposition: OpenDisposition) -> io::Result<EntryPin> {
        let reserved = self.projector.project(callback_path)?;
        if reserved.relative().is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "mount root is not a regular file"));
        }
        let slot = self.materialization_guard(reserved.backend())?;
        let _path = lock(&slot.gate)?;
        let (path, generation) = {
            let _namespace = read_lock(&self.namespace)?;
            let path = self.project_checked(callback_path)?;
            if let Some(entry) = self.materialize_cached(&path, disposition)? {
                return Ok(EntryPin::new(entry));
            }
            (path, slot.generation.load(Ordering::Acquire))
        };
        let prepared = self.materialize_fetch(&path, disposition)?;
        let _namespace = read_lock(&self.namespace)?;
        if slot.generation.load(Ordering::Acquire) != generation {
            return Err(io::Error::new(io::ErrorKind::WouldBlock,
                "mounted namespace changed during content acquisition"));
        }
        self.materialize_install(&path, prepared, disposition).map(EntryPin::new)
    }

    /// Namespace-owned rename path: deliberately does not acquire a path gate.
    pub(super) fn materialize(&self, path: &ProjectedPath,
        disposition: OpenDisposition) -> io::Result<Arc<Entry>> {
        if let Some(entry) = self.materialize_cached(path, disposition)? { return Ok(entry); }
        let prepared = self.materialize_fetch(path, disposition)?;
        let entry = self.materialize_install(path, prepared, disposition)?;
        self.retirement_pending.store(true, Ordering::Release);
        Ok(entry)
    }

    fn materialize_cached(&self, path: &ProjectedPath,
        disposition: OpenDisposition) -> io::Result<Option<Arc<Entry>>> {
        let Some(entry) = self.entry_for_path(path.backend())? else { return Ok(None); };
        let state = lock(&entry.state)?;
        if disposition == OpenDisposition::CreateNew {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "file already exists"));
        }
        if state.delete_token.is_some() { return Err(not_found(path.backend())); }
        drop(state);
        Ok(Some(entry))
    }

    fn materialize_fetch(&self, path: &ProjectedPath,
        disposition: OpenDisposition) -> io::Result<PreparedMaterialization<'_>> {
        let remote = match self.backend.stat(path.backend()) {
            Ok(meta) => Some(meta),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let create = match (remote.is_some(), disposition) {
            (false, OpenDisposition::OpenExisting | OpenDisposition::TruncateExisting) =>
                return Err(not_found(path.backend())),
            (true, OpenDisposition::CreateNew) => return Err(io::Error::new(
                io::ErrorKind::AlreadyExists, "file already exists")),
            (false, _) => true,
            (true, _) => false,
        };
        let truncates = matches!(disposition,
            OpenDisposition::TruncateExisting | OpenDisposition::CreateAlways);
        if let Some(meta) = &remote { require_regular(meta)?; }
        let baseline = remote.as_ref().map(baseline_from_meta).unwrap_or(Baseline::Missing);
        if !create && !truncates {
            if let Some(record) = self.clean_cache.claim(&self.cache_key(path.backend()))? {
                let prepared = PreparedMaterialization { engine: self, record: Some(record),
                    condition: EntryCondition::Clean };
                let record = prepared.record.as_ref().ok_or_else(|| invalid_data("missing cache claim"))?;
                self.verify_unique_cached_alias(path.backend(), &record.path)?;
                let length = self.spool.open_file(&record.spool_name, false)
                    .and_then(|file| file.metadata()).map(|meta| meta.len());
                if record.created.elapsed() < MAX_CONTENT_AGE && record.baseline == baseline
                    && length.as_ref().is_ok_and(|length| *length == record.bytes)
                    && remote.as_ref().is_some_and(|meta| meta.size == record.bytes) {
                    return Ok(prepared);
                }
                if let Err(error) = length {
                    if error.kind() != io::ErrorKind::NotFound { return Err(error); }
                }
                drop(prepared);
            }
        }
        let bytes = if create || truncates { 0 } else {
            remote.as_ref().ok_or_else(|| invalid_data("missing remote baseline"))?.size
                .checked_add(1).ok_or_else(|| invalid_data("remote length cannot be bounded"))?
        };
        self.clean_cache.trim(&self.spool, self.config.cache.retained_bytes())?;
        let _growth = self.reserve_growth(bytes)?;
        let allocated = self.spool.allocate()?;
        let mut prepared = PreparedMaterialization { engine: self,
            record: Some(IdleClean::new(path.backend().to_string(), self.cache_key(path.backend()),
                allocated.name, baseline, bytes, Instant::now())),
            condition: if create || truncates { EntryCondition::Dirty } else { EntryCondition::Clean } };
        if !create && !truncates {
            let meta = remote.as_ref().ok_or_else(|| invalid_data("missing remote baseline"))?;
            let reader = self.backend.open_read_id(path.backend(), meta.id.as_deref())?;
            let mut reader = reader.take(bytes);
            let copied = io::copy(&mut reader, &mut &allocated.file)?;
            // Release proxy request ownership before the second stat.
            drop(reader);
            if copied != meta.size {
                return Err(invalid_data("remote content length differs from its advertised size"));
            }
            allocated.file.sync_data()?;
            let fresh = self.backend.stat(path.backend())?;
            require_regular(&fresh)?;
            if baseline_from_meta(&fresh) != baseline_from_meta(meta) {
                return Err(io::Error::new(io::ErrorKind::WouldBlock,
                    "remote file changed while it was being materialized"));
            }
        } else { allocated.file.sync_data()?; }
        if let Some(record) = prepared.record.as_mut() {
            record.bytes = allocated.file.metadata()?.len();
        }
        // Successful acquisition relinquishes its OS handle before publication.
        drop(allocated.file);
        Ok(prepared)
    }

    fn materialize_install(&self, path: &ProjectedPath, mut prepared: PreparedMaterialization<'_>,
        disposition: OpenDisposition) -> io::Result<Arc<Entry>> {
        if let Some(existing) = self.materialize_cached(path, disposition)? { return Ok(existing); }
        let mut entries = lock(&self.entries)?;
        let key = self.cache_key(path.backend());
        if let Some(existing) = entries.get(&key) {
            if disposition == OpenDisposition::CreateNew {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists,
                    "file was concurrently materialized"));
            }
            return Ok(Arc::clone(existing));
        }
        let record = prepared.record.take().ok_or_else(|| invalid_data("missing prepared spool"))?;
        let entry = Arc::new(Entry { pins: AtomicUsize::new(0),
            retirement_pending: Arc::clone(&self.retirement_pending), state: Mutex::new(EntryState {
            remote_path: path.backend().to_string(), spool_name: record.spool_name,
            baseline: record.baseline, condition: prepared.condition.clone(),
            delete_token: None, delete_committed: false, clean_since: record.created, retired: false,
        }) });
        let mut state = lock(&entry.state)?;
        entries.insert(key, Arc::clone(&entry));
        drop(entries);
        if state.condition != EntryCondition::Clean {
            // A possibly durable journal append must never point to a deleted
            // spool, even when its sync reported an ambiguous failure.
            if let Err(error) = self.spool.persist_entry(&state.persisted()) {
                state.condition = EntryCondition::Conflict(super::types::MountConflict {
                    path: state.remote_path.clone(), baseline: state.baseline.clone(), current: None,
                    detail: format!("initial spool journal durability is uncertain: {error}"),
                });
                return Err(error);
            }
        }
        drop(state);
        Ok(entry)
    }
}
