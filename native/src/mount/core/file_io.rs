use super::engine::{
    baseline_from_meta, invalid_data, lock, not_found, read_lock, require_regular, write_lock,
    Entry, EntryState, MountEngine, OpenHandle,
};
use super::path::ProjectedPath;
use super::types::{
    Baseline, EntryCondition, FlushOutcome, HandleId, MountConflict, MountMode, OpenDisposition,
    OpenFileOptions,
};
use crate::vfs::unique_staging_path;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

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
        let _namespace = read_lock(&self.namespace)?;
        let path = self.project_checked(callback_path)?;
        let entry = self.materialize(&path, options.disposition)?;
        if matches!(
            options.disposition,
            OpenDisposition::TruncateExisting | OpenDisposition::CreateAlways
        ) {
            self.truncate_entry(&entry, 0)?;
        }
        let raw = self
            .next_handle
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "mount handle space exhausted"))?;
        let handle = HandleId(raw);
        lock(&self.handles)?.insert(
            handle,
            OpenHandle {
                entry,
                writable: options.writable,
            },
        );
        Ok(handle)
    }

    pub fn read(&self, handle: HandleId, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        let entry = self.handle(handle)?.entry;
        let state = lock(&entry.state)?;
        let mut file = self.spool.open_file(&state.spool_name, false)?;
        file.seek(SeekFrom::Start(offset))?;
        file.read(output)
    }

    pub fn write(&self, handle: HandleId, offset: u64, input: &[u8]) -> io::Result<usize> {
        let opened = self.handle(handle)?;
        if !opened.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            ));
        }
        let mut state = lock(&opened.entry.state)?;
        self.mark_dirty(&mut state)?;
        let mut file = self.spool.open_file(&state.spool_name, true)?;
        file.seek(SeekFrom::Start(offset))?;
        file.write(input)
    }

    pub fn append(&self, handle: HandleId, input: &[u8]) -> io::Result<usize> {
        let opened = self.handle(handle)?;
        if !opened.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            ));
        }
        let mut state = lock(&opened.entry.state)?;
        self.mark_dirty(&mut state)?;
        let mut file = self.spool.open_file(&state.spool_name, true)?;
        file.seek(SeekFrom::End(0))?;
        file.write(input)
    }

    pub fn len(&self, handle: HandleId) -> io::Result<u64> {
        let entry = self.handle(handle)?.entry;
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
        self.truncate_entry(&opened.entry, length)
    }

    pub fn flush(&self, handle: HandleId) -> io::Result<FlushOutcome> {
        self.flush_entry(&self.handle(handle)?.entry)
    }

    pub fn close(&self, handle: HandleId) -> io::Result<()> {
        let _namespace = write_lock(&self.namespace)?;
        let opened = lock(&self.handles)?
            .remove(&handle)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown mount handle"))?;
        self.cleanup_committed_entry(&opened.entry)
    }

    pub(super) fn materialize(
        &self,
        path: &ProjectedPath,
        disposition: OpenDisposition,
    ) -> io::Result<Arc<Entry>> {
        let cache_key = self.cache_key(path.backend());
        if let Some(entry) = self.entry_for_path(path.backend())? {
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
            return Ok(entry);
        }
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
        let (baseline, condition) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = self.spool.remove_file(&allocated.name);
                return Err(error);
            }
        };
        let state = EntryState {
            remote_path: path.backend().to_string(),
            spool_name: allocated.name,
            baseline,
            condition,
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
        let opened = handles
            .get(&handle)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown mount handle"))?;
        Ok(OpenHandle {
            entry: opened.entry.clone(),
            writable: opened.writable,
        })
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
