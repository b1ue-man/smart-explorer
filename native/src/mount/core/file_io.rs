use super::engine::{
    baseline_from_meta, invalid_data, lock, not_found, read_lock, require_regular, write_lock,
    Entry, EntryState, MountEngine, OpenHandle, OpenHandleKind,
};
use super::path::ProjectedPath;
use super::types::{
    Baseline, EntryCondition, FlushOutcome, HandleId, MountConflict, MountMode, OpenDisposition,
    OpenFileOptions,
};
use crate::vfs::{unique_staging_path, VfsMeta};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// A fetched whole-file spool copy that is not yet visible in the entry map.
struct PreparedMaterialization {
    spool_name: String,
    baseline: Baseline,
    condition: EntryCondition,
}

impl MountEngine {
    pub fn open_file(&self, callback_path: &str, options: OpenFileOptions) -> io::Result<HandleId> {
        let creates_or_truncates = matches!(
            options.disposition,
            OpenDisposition::OpenOrCreate
                | OpenDisposition::CreateNew
                | OpenDisposition::TruncateExisting
                | OpenDisposition::CreateAlways
        );
        if (options.writable || creates_or_truncates) && self.config.mode != MountMode::ReadWrite {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mount is read-only",
            ));
        }
        let reserved_path = self.projector.project(callback_path)?;
        if reserved_path.relative().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mount root is not a regular file",
            ));
        }
        let path_guard = self.materialization_guard(reserved_path.backend())?;
        let _path_reservation = lock(&path_guard)?;
        let truncates = matches!(
            options.disposition,
            OpenDisposition::TruncateExisting | OpenDisposition::CreateAlways
        );
        let path = {
            let _namespace = read_lock(&self.namespace)?;
            let path = self.project_checked(callback_path)?;
            if let Some(entry) = self.materialize_cached(&path, options.disposition)? {
                if truncates {
                    self.truncate_entry(&entry, 0)?;
                }
                return self.insert_handle(OpenHandleKind::Materialized(entry), options.writable);
            }
            path
        };
        // The whole-file fetch can take minutes; running it outside the
        // namespace lock keeps closes and renames (writers) from queuing
        // behind it and, transitively, stalling every other callback. The
        // per-path materialization guard still serializes this path, and
        // installation revalidates against concurrent namespace changes.
        let prepared = self.materialize_fetch(&path, options.disposition)?;
        let entry = {
            let _namespace = read_lock(&self.namespace)?;
            self.materialize_install(&path, prepared, options.disposition)?
        };
        if truncates {
            self.truncate_entry(&entry, 0)?;
        }
        self.insert_handle(OpenHandleKind::Materialized(entry), options.writable)
    }

    /// Opens a regular file for attributes/control operations without fetching
    /// its contents into the whole-file spool. Dokany passes already-expanded
    /// kernel access rights, so a handle created here must never service data
    /// I/O; a later data open receives its own materialized handle.
    pub(crate) fn open_metadata_file(
        &self,
        callback_path: &str,
        metadata: VfsMeta,
    ) -> io::Result<HandleId> {
        require_regular(&metadata)?;
        let _namespace = read_lock(&self.namespace)?;
        let path = self.projector.project(callback_path)?;
        self.validate_projected_case(&path)?;
        if path.relative().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mount root is not a regular file",
            ));
        }
        self.insert_handle(OpenHandleKind::Metadata(metadata), false)
    }

    pub fn read(&self, handle: HandleId, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        let entry = self.handle(handle)?.materialized_entry()?;
        let state = lock(&entry.state)?;
        let mut file = self.spool.open_file(&state.spool_name, false)?;
        file.seek(SeekFrom::Start(offset))?;
        // Windows callers treat a short count as end-of-file, so fill the
        // buffer until the spool is exhausted rather than returning one
        // arbitrary partial read.
        let mut total = 0;
        while total < output.len() {
            match file.read(&mut output[total..]) {
                Ok(0) => break,
                Ok(read) => total += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(total)
    }

    pub fn write(&self, handle: HandleId, offset: u64, input: &[u8]) -> io::Result<usize> {
        let opened = self.handle(handle)?;
        if !opened.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            ));
        }
        let entry = opened.materialized_entry()?;
        let mut state = lock(&entry.state)?;
        self.mark_dirty(&mut state)?;
        let mut file = self.spool.open_file(&state.spool_name, true)?;
        file.seek(SeekFrom::Start(offset))?;
        // A silently short write would be reported to Windows as success for
        // the smaller count and the remainder would never be retried.
        file.write_all(input)?;
        Ok(input.len())
    }

    pub fn append(&self, handle: HandleId, input: &[u8]) -> io::Result<usize> {
        let opened = self.handle(handle)?;
        if !opened.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            ));
        }
        let entry = opened.materialized_entry()?;
        let mut state = lock(&entry.state)?;
        self.mark_dirty(&mut state)?;
        let mut file = self.spool.open_file(&state.spool_name, true)?;
        file.seek(SeekFrom::End(0))?;
        file.write_all(input)?;
        Ok(input.len())
    }

    pub fn len(&self, handle: HandleId) -> io::Result<u64> {
        let entry = self.handle(handle)?.materialized_entry()?;
        let state = lock(&entry.state)?;
        Ok(self
            .spool
            .open_file(&state.spool_name, false)?
            .metadata()?
            .len())
    }

    pub fn truncate(&self, handle: HandleId, length: u64) -> io::Result<()> {
        let opened = self.handle(handle)?;
        if !opened.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            ));
        }
        self.truncate_entry(&opened.materialized_entry()?, length)
    }

    pub fn flush(&self, handle: HandleId) -> io::Result<FlushOutcome> {
        let opened = self.handle(handle)?;
        match opened.kind {
            OpenHandleKind::Materialized(entry) => self.flush_entry(&entry),
            OpenHandleKind::Metadata(_) => Ok(FlushOutcome::NoChanges),
        }
    }

    pub fn close(&self, handle: HandleId) -> io::Result<()> {
        let opened = lock(&self.handles)?
            .remove(&handle)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown mount handle"))?;
        match opened.kind {
            OpenHandleKind::Materialized(entry) => {
                // Only materialized closes need the namespace write lock (for
                // spool cleanup racing renames). Metadata handles — the bulk
                // of Explorer traffic — must not queue a writer behind long
                // read-holding callbacks and stall the whole drive.
                let _namespace = write_lock(&self.namespace)?;
                self.cleanup_committed_entry(&entry)
            }
            OpenHandleKind::Metadata(_) => Ok(()),
        }
    }

    /// Combined materialization for callers that already hold a namespace
    /// lock for their whole operation (the replacing-rename path).
    pub(super) fn materialize(
        &self,
        path: &ProjectedPath,
        disposition: OpenDisposition,
    ) -> io::Result<Arc<Entry>> {
        if let Some(entry) = self.materialize_cached(path, disposition)? {
            return Ok(entry);
        }
        let prepared = self.materialize_fetch(path, disposition)?;
        self.materialize_install(path, prepared, disposition)
    }

    /// The lock-free fast path: an already-cached entry, with the same
    /// disposition admission the combined path applies.
    fn materialize_cached(
        &self,
        path: &ProjectedPath,
        disposition: OpenDisposition,
    ) -> io::Result<Option<Arc<Entry>>> {
        let Some(entry) = self.entry_for_path(path.backend())? else {
            return Ok(None);
        };
        let state = lock(&entry.state)?;
        if disposition == OpenDisposition::CreateNew {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "file already exists",
            ));
        }
        if state.delete_token.is_some() {
            return Err(not_found(path.backend()));
        }
        drop(state);
        Ok(Some(entry))
    }

    /// Downloads (or prepares) the whole-file spool copy. Requires no
    /// namespace lock: installation revalidates the namespace afterwards, and
    /// the post-transfer stat detects remote drift.
    fn materialize_fetch(
        &self,
        path: &ProjectedPath,
        disposition: OpenDisposition,
    ) -> io::Result<PreparedMaterialization> {
        let remote = match self.backend.stat(path.backend()) {
            Ok(meta) => Some(meta),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let create = match (remote.is_some(), disposition) {
            (false, OpenDisposition::OpenExisting | OpenDisposition::TruncateExisting) => {
                return Err(not_found(path.backend()));
            }
            (true, OpenDisposition::CreateNew) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "file already exists",
                ));
            }
            (false, _) => true,
            (true, _) => false,
        };
        let allocated = self.spool.allocate()?;
        let prepared = (|| {
            if create {
                allocated.file.sync_data()?;
                return Ok((Baseline::Missing, EntryCondition::Dirty));
            }
            let meta = remote.as_ref().ok_or_else(|| {
                invalid_data("remote existence changed during materialization planning")
            })?;
            require_regular(meta)?;
            let baseline = baseline_from_meta(meta);
            let mut reader = self
                .backend
                .open_read_id(path.backend(), meta.id.as_deref())?;
            let mut writer = &allocated.file;
            io::copy(&mut reader, &mut writer)?;
            // A proxied reader owns its backend request permit until Drop.
            // Release it before the verification stat: sequential SFTP/Agent
            // backends advertise one in-flight request, so retaining the
            // completed reader here would make the stat wait on itself.
            drop(reader);
            allocated.file.sync_data()?;
            let fresh = self.backend.stat(path.backend())?;
            require_regular(&fresh)?;
            if baseline_from_meta(&fresh) != baseline {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "remote file changed while it was being materialized",
                ));
            }
            Ok((baseline, EntryCondition::Clean))
        })();
        match prepared {
            Ok((baseline, condition)) => Ok(PreparedMaterialization {
                spool_name: allocated.name,
                baseline,
                condition,
            }),
            Err(error) => {
                let _ = self.spool.remove_file(&allocated.name);
                Err(error)
            }
        }
    }

    /// Installs a fetched spool copy. The caller must hold a namespace lock;
    /// a concurrent entry or delete that appeared while fetching ran unlocked
    /// wins, and the redundant spool copy is discarded.
    fn materialize_install(
        &self,
        path: &ProjectedPath,
        prepared: PreparedMaterialization,
        disposition: OpenDisposition,
    ) -> io::Result<Arc<Entry>> {
        let cache_key = self.cache_key(path.backend());
        match self.materialize_cached(path, disposition) {
            Ok(Some(existing)) => {
                self.spool.remove_file(&prepared.spool_name)?;
                return Ok(existing);
            }
            Ok(None) => {}
            Err(error) => {
                let _ = self.spool.remove_file(&prepared.spool_name);
                return Err(error);
            }
        }
        let state = EntryState {
            remote_path: path.backend().to_string(),
            spool_name: prepared.spool_name,
            baseline: prepared.baseline,
            condition: prepared.condition,
            delete_token: None,
            delete_committed: false,
        };
        let entry = Arc::new(Entry {
            state: Mutex::new(state),
        });
        let mut entries = lock(&self.entries)?;
        if let Some(existing) = entries.get(&cache_key).cloned() {
            let spool_name = lock(&entry.state)?.spool_name.clone();
            self.spool.remove_file(&spool_name)?;
            if disposition == OpenDisposition::CreateNew {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "file was concurrently materialized",
                ));
            }
            return Ok(existing);
        }
        let state = lock(&entry.state)?;
        if state.condition != EntryCondition::Clean {
            if let Err(error) = self.spool.persist_entry(&state.persisted()) {
                let spool_name = state.spool_name.clone();
                drop(state);
                self.spool.remove_file(&spool_name)?;
                return Err(error);
            }
        }
        drop(state);
        entries.insert(cache_key, entry.clone());
        Ok(entry)
    }

    pub(super) fn flush_entry(&self, entry: &Arc<Entry>) -> io::Result<FlushOutcome> {
        let mut state = lock(&entry.state)?;
        if state.delete_committed {
            // A handle that survived FILE_SHARE_DELETE refers to the detached
            // pre-replace object. Its spool remains usable until last close,
            // but it must never overwrite the new namespace occupant.
            self.spool.open_file(&state.spool_name, true)?.sync_data()?;
            return Ok(FlushOutcome::NoChanges);
        }
        if state.delete_token.is_some() {
            self.spool.open_file(&state.spool_name, true)?.sync_data()?;
            return Ok(FlushOutcome::NoChanges);
        }
        match &state.condition {
            EntryCondition::Clean => return Ok(FlushOutcome::NoChanges),
            EntryCondition::Conflict(conflict) => {
                return Ok(FlushOutcome::Conflict(conflict.clone()));
            }
            EntryCondition::Dirty => {}
        }
        if let Some(conflict) = self.detect_conflict(&state)? {
            let persisted = state.with_condition(EntryCondition::Conflict(conflict.clone()));
            self.spool.persist_entry(&persisted)?;
            state.condition = EntryCondition::Conflict(conflict.clone());
            return Ok(FlushOutcome::Conflict(conflict));
        }
        let staged = unique_staging_path(&*self.backend, &state.remote_path, "mount")?;
        self.invalidate_metadata(&state.remote_path, false);
        let mut source = self.spool.open_file(&state.spool_name, true)?;
        source.sync_data()?;
        // A failed exclusive open does not transfer ownership of `staged`.
        // In particular, never clean that spelling up on AlreadyExists: it may
        // belong to a concurrent actor or to a case alias on the remote.
        let mut destination = self.backend.open_write_new(&staged)?;
        let upload = (|| {
            io::copy(&mut source, &mut destination)?;
            destination.flush()?;
            drop(destination);
            Ok(())
        })();
        if let Err(error) = upload {
            // A layered writer may have committed before its final reply was
            // lost. Without a stable item identity, path cleanup could remove
            // a concurrent replacement, so retain the stage.
            return Err(error);
        }
        // Uploading a whole-file spool may take minutes. Revalidate immediately
        // before the atomic promotion so a remote edit during that transfer is
        // not silently overwritten.
        match self.detect_conflict(&state) {
            Ok(Some(conflict)) => {
                // The stage spelling is not an ownership proof after the remote
                // upload. Preserve it rather than deleting a possible replacement.
                let persisted = state.with_condition(EntryCondition::Conflict(conflict.clone()));
                self.spool.persist_entry(&persisted)?;
                state.condition = EntryCondition::Conflict(conflict.clone());
                return Ok(FlushOutcome::Conflict(conflict));
            }
            Ok(None) => {}
            Err(error) => {
                // Verification is inconclusive; retain the stage as recovery
                // evidence and never delete an unknown current occupant.
                return Err(error);
            }
        }
        let promotion = match &state.baseline {
            Baseline::Missing => self
                .backend
                .promote_staged_no_replace(&staged, &state.remote_path),
            Baseline::Present { .. } => self.backend.promote_staged(&staged, &state.remote_path),
        };
        self.invalidate_metadata(&state.remote_path, false);
        if let Err(error) = promotion {
            let destination = self.observe_path(&state.remote_path);
            let staged_state = self.observe_path(&staged);
            if destination.matches(&state.baseline, Some(false)) && staged_state.is_plain_file() {
                // Both pre-mutation names are still intact. This is the only
                // observation that proves the promotion did not take effect.
                // It does not, however, prove that the current staging occupant
                // is still the exclusively opened item, so retain it.
                return Err(error);
            }
            let detail = format!(
                "remote save may already be committed after an ambiguous promotion response: {error}; destination={}; staging={}",
                destination.summary(),
                staged_state.summary()
            );
            let conflict = self.post_commit_conflict(&mut state, destination.current(), &detail);
            return Ok(FlushOutcome::CommittedPendingVerification(conflict));
        }
        let committed = match self.backend.stat(&state.remote_path) {
            Ok(committed) if !committed.is_dir && !committed.is_symlink => committed,
            Ok(committed) => {
                let conflict = self.post_commit_conflict(
                    &mut state,
                    Some(baseline_from_meta(&committed)),
                    "backend reported a non-regular file after successful promotion",
                );
                return Ok(FlushOutcome::CommittedPendingVerification(conflict));
            }
            Err(error) => {
                let conflict = self.post_commit_conflict(
                    &mut state,
                    None,
                    &format!(
                        "remote save was committed but its destination could not be verified: {error}"
                    ),
                );
                return Ok(FlushOutcome::CommittedPendingVerification(conflict));
            }
        };
        let committed_baseline = baseline_from_meta(&committed);
        if let Err(error) = self
            .spool
            .forget_entry(&state.remote_path, &state.spool_name)
        {
            let conflict = self.post_commit_conflict(
                &mut state,
                Some(committed_baseline),
                &format!(
                    "remote save was committed but its local recovery journal could not be cleared: {error}"
                ),
            );
            return Ok(FlushOutcome::CommittedPendingVerification(conflict));
        }
        state.baseline = committed_baseline;
        state.condition = EntryCondition::Clean;
        Ok(FlushOutcome::Committed)
    }

    pub(super) fn handle(&self, handle: HandleId) -> io::Result<OpenHandle> {
        let handles = lock(&self.handles)?;
        handles
            .get(&handle)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown mount handle"))
    }

    fn insert_handle(&self, kind: OpenHandleKind, writable: bool) -> io::Result<HandleId> {
        let raw = self
            .next_handle
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| io::Error::other("mount handle space exhausted"))?;
        let handle = HandleId(raw);
        lock(&self.handles)?.insert(handle, OpenHandle { kind, writable });
        Ok(handle)
    }

    fn truncate_entry(&self, entry: &Arc<Entry>, length: u64) -> io::Result<()> {
        let mut state = lock(&entry.state)?;
        self.mark_dirty(&mut state)?;
        self.spool
            .open_file(&state.spool_name, true)?
            .set_len(length)
    }

    fn mark_dirty(&self, state: &mut EntryState) -> io::Result<()> {
        if state.delete_committed {
            // Windows permits I/O through an already-open handle after its
            // name was replaced. Those bytes belong only to that detached
            // object and intentionally never re-enter the remote namespace.
            return Ok(());
        }
        match state.condition {
            EntryCondition::Clean => {
                let persisted = state.with_condition(EntryCondition::Dirty);
                self.spool.persist_entry(&persisted)?;
                state.condition = EntryCondition::Dirty;
                Ok(())
            }
            EntryCondition::Dirty => Ok(()),
            EntryCondition::Conflict(_) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "file has an unresolved remote conflict",
            )),
        }
    }

    fn detect_conflict(&self, state: &EntryState) -> io::Result<Option<MountConflict>> {
        let (current, unsafe_type) = match self.backend.stat(&state.remote_path) {
            Ok(meta) => (
                Some(baseline_from_meta(&meta)),
                meta.is_dir || meta.is_symlink,
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (None, false),
            Err(error) => return Err(error),
        };
        let matches = !unsafe_type
            && match (&state.baseline, &current) {
                (Baseline::Missing, None) => true,
                (expected @ Baseline::Present { .. }, Some(actual)) => expected == actual,
                _ => false,
            };
        Ok((!matches).then(|| MountConflict {
            path: state.remote_path.clone(),
            baseline: state.baseline.clone(),
            current,
            detail: "remote identity, size, modification time, or available content hash changed since the local baseline".into(),
        }))
    }
}
