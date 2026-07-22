use super::engine::{lock, parent_path, write_lock, MountEngine};
use super::journal::{DeletePhase, PersistedDelete};
use super::types::{Baseline, DeleteToken, EntryCondition};
use crate::vfs::{unique_staging_path, DeleteDisposition};
use std::io;

impl MountEngine {
    pub fn begin_delete(&self, callback_path: &str, is_directory: bool) -> io::Result<DeleteToken> {
        self.require_writable()?;
        let _namespace = write_lock(&self.namespace)?;
        if self.backend.delete_disposition() == DeleteDisposition::Unsupported {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "backend does not support deletion",
            ));
        }
        let path = self.project_checked(callback_path)?;
        if path.relative().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mount root cannot be deleted",
            ));
        }
        let entry = self.entry_for_path(path.backend())?;
        let mut entry_state = match &entry {
            Some(entry) => Some(lock(&entry.state)?),
            None => None,
        };
        if entry_state
            .as_ref()
            .is_some_and(|state| state.delete_token.is_some())
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "mounted entry is already pending deletion",
            ));
        }
        let meta = match self.backend.stat(path.backend()) {
            Ok(meta) => Some(meta),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let local_only = !is_directory
            && meta.is_none()
            && entry_state
                .as_ref()
                .is_some_and(|state| state.baseline == Baseline::Missing);
        if !local_only {
            let meta = meta.as_ref().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "mounted delete target is absent")
            })?;
            if meta.is_symlink || meta.is_dir != is_directory {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "delete target type changed or is link-like",
                ));
            }
            if is_directory
                && (!self.backend.list_dir(path.backend())?.is_empty()
                    || self.has_visible_cached_child(path.backend())?)
            {
                return Err(io::Error::new(
                    io::ErrorKind::DirectoryNotEmpty,
                    "mounted directory is not empty",
                ));
            }
        }

        let raw = self
            .next_delete
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |current| current.checked_add(1),
            )
            .map_err(|_| {
                io::Error::new(io::ErrorKind::Other, "mount delete token space exhausted")
            })?;
        let token = DeleteToken(raw);
        let quarantine = if local_only {
            String::new()
        } else {
            unique_staging_path(&*self.backend, path.backend(), "mount-delete")?
        };
        let mut delete = PersistedDelete {
            token: raw,
            original_path: path.backend().to_string(),
            quarantine_path: quarantine,
            id: meta.and_then(|meta| meta.id),
            is_directory,
            phase: if local_only {
                DeletePhase::LocalOnly
            } else {
                DeletePhase::Unresolved
            },
        };
        self.spool.persist_delete(&delete)?;
        if local_only {
            lock(&self.deletes)?.insert(token, delete);
            if let Some(state) = entry_state.as_mut() {
                state.delete_token = Some(raw);
                if let Err(error) = self.spool.persist_entry(&state.persisted()) {
                    state.delete_token = None;
                    let _ = self.persist_restored_entry(state);
                    let _ = self.spool.forget_delete(raw);
                    lock(&self.deletes)?.remove(&token);
                    return Err(error);
                }
            }
            return Ok(token);
        }
        let quarantine_result = self
            .backend
            .rename_no_replace(&delete.original_path, &delete.quarantine_path);
        self.invalidate_metadata(&delete.original_path, true);
        self.invalidate_metadata(&delete.quarantine_path, true);
        if let Err(error) = quarantine_result {
            let source_exists = self.backend.try_exists(&delete.original_path);
            let quarantine_exists = self.backend.try_exists(&delete.quarantine_path);
            match (source_exists, quarantine_exists) {
                (Ok(true), Ok(false)) => {
                    self.spool.forget_delete(raw)?;
                    return Err(error);
                }
                (Ok(false), Ok(true)) => {
                    // The no-replace move took effect despite its missing/error
                    // response. Make that namespace boundary durable and let
                    // Cleanup/recovery own physical quarantine collection.
                    delete.phase = DeletePhase::Moved;
                    if let Err(journal_error) = self.spool.persist_delete(&delete) {
                        delete.phase = DeletePhase::Unresolved;
                        lock(&self.deletes)?.insert(token, delete);
                        return Err(io::Error::new(
                            journal_error.kind(),
                            format!(
                                "delete quarantine move became visible after an ambiguous response ({error}), but its committed phase could not be journaled: {journal_error}"
                            ),
                        ));
                    }
                }
                (source, quarantine) => {
                    lock(&self.deletes)?.insert(token, delete);
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "delete quarantine move is ambiguous ({error}); source={source:?}, quarantine={quarantine:?}"
                        ),
                    ));
                }
            }
        } else {
            delete.phase = DeletePhase::Moved;
            if let Err(error) = self.spool.persist_delete(&delete) {
                let rolled_back = self
                    .backend
                    .rename_no_replace(&delete.quarantine_path, &delete.original_path)
                    .is_ok();
                if rolled_back {
                    let _ = self.spool.forget_delete(raw);
                } else {
                    // The pre-dispatch Unresolved record is durable. A failed
                    // Moved append may nevertheless have reached storage, so a
                    // restart must neither restore nor collect the quarantine.
                    delete.phase = DeletePhase::Unresolved;
                    lock(&self.deletes)?.insert(token, delete);
                }
                return Err(error);
            }
        }
        lock(&self.deletes)?.insert(token, delete.clone());

        if let Some(state) = entry_state.as_mut() {
            state.delete_token = Some(raw);
            if let Err(error) = self.spool.persist_entry(&state.persisted()) {
                if self
                    .backend
                    .rename_no_replace(&delete.quarantine_path, &delete.original_path)
                    .is_ok()
                {
                    state.delete_token = None;
                    let _ = self.persist_restored_entry(state);
                    let _ = self.spool.forget_delete(raw);
                    lock(&self.deletes)?.remove(&token);
                }
                return Err(error);
            }
        }
        Ok(token)
    }

    pub fn cancel_delete(&self, token: DeleteToken) -> io::Result<()> {
        let _namespace = write_lock(&self.namespace)?;
        self.cancel_delete_locked(token)
    }

    fn cancel_delete_locked(&self, token: DeleteToken) -> io::Result<()> {
        let delete = self.delete_for_token(token)?;
        if delete.phase == DeletePhase::LocalOnly {
            self.clear_restored_entry(token)?;
            self.spool.forget_delete(token.0)?;
            lock(&self.deletes)?.remove(&token);
            return Ok(());
        }
        let source_exists = self.backend.try_exists(&delete.original_path)?;
        let quarantine_exists = self.backend.try_exists(&delete.quarantine_path)?;
        match (source_exists, quarantine_exists) {
            (false, true) => self
                .backend
                .rename_no_replace(&delete.quarantine_path, &delete.original_path)?,
            (true, false) => {}
            (true, true) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "cannot roll back deletion because both paths exist",
                ));
            }
            (false, false) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "cannot roll back deletion because both paths are absent",
                ));
            }
        }
        self.invalidate_metadata(&delete.original_path, true);
        self.invalidate_metadata(&delete.quarantine_path, true);
        self.clear_restored_entry(token)?;
        self.spool.forget_delete(token.0)?;
        lock(&self.deletes)?.remove(&token);
        Ok(())
    }

    pub fn commit_delete(&self, token: DeleteToken) -> io::Result<()> {
        let _namespace = write_lock(&self.namespace)?;
        self.commit_delete_locked(token)
    }

    pub(super) fn commit_delete_locked(&self, token: DeleteToken) -> io::Result<()> {
        let delete = self.delete_for_token(token)?;
        if delete.phase == DeletePhase::LocalOnly {
            return self.finalize_committed_delete(token);
        }
        if matches!(
            delete.phase,
            DeletePhase::Unresolved | DeletePhase::Prepared
        ) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "unresolved delete quarantine requires explicit recovery",
            ));
        }
        self.invalidate_metadata(&delete.original_path, true);
        self.invalidate_metadata(&delete.quarantine_path, true);
        match self.backend.stat(&delete.quarantine_path) {
            Ok(meta) => {
                if meta.is_symlink || meta.is_dir != delete.is_directory {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "quarantined delete target changed type",
                    ));
                }
                if let (Some(expected), Some(actual)) = (delete.id.as_deref(), meta.id.as_deref()) {
                    if expected != actual {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "quarantined delete target changed identity",
                        ));
                    }
                }
                if delete.is_directory {
                    self.backend.remove_dir(&delete.quarantine_path)?;
                } else {
                    self.backend
                        .remove_file_id(&delete.quarantine_path, delete.id.as_deref())?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if self.backend.try_exists(&delete.original_path)? {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "delete was rolled back instead of committed",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        self.finalize_committed_delete(token)
    }

    pub fn delete(&self, callback_path: &str, is_directory: bool) -> io::Result<()> {
        let token = self.begin_delete(callback_path, is_directory)?;
        self.commit_delete(token)
    }

    pub fn pending_deletes(&self) -> io::Result<Vec<DeleteToken>> {
        Ok(lock(&self.deletes)?.keys().copied().collect())
    }

    pub(super) fn recover_pending_deletes(&self) -> io::Result<()> {
        let tokens = self.pending_deletes()?;
        for token in tokens {
            let delete = self.delete_for_token(token)?;
            if delete.phase == DeletePhase::LocalOnly {
                // A successful DeleteFile request already hid this local-only
                // object. If the host vanished before/inside void Cleanup,
                // preserve the application's delete intent. An explicit
                // cancellation durably clears the entry token first.
                self.finalize_committed_delete(token)?;
                continue;
            }
            let source_exists = self.backend.try_exists(&delete.original_path)?;
            let quarantine_exists = self.backend.try_exists(&delete.quarantine_path)?;
            match (source_exists, quarantine_exists) {
                (false, true) => {
                    // After restart, even a replayed Moved record may be the
                    // result of an append whose sync reported failure. Keep the
                    // quarantine and journal for explicit recovery; neither
                    // restoring nor collecting it is safe automatically.
                }
                (true, false) => {
                    self.clear_restored_entry(token)?;
                    self.spool.forget_delete(token.0)?;
                    lock(&self.deletes)?.remove(&token);
                }
                (false, false) => self.finalize_committed_delete(token)?,
                (true, true) => {
                    // Ambiguous external interference: keep both the journal
                    // and quarantine for explicit recovery; never delete one.
                }
            }
        }
        Ok(())
    }

    pub(super) fn delete_for_token(&self, token: DeleteToken) -> io::Result<PersistedDelete> {
        lock(&self.deletes)?
            .get(&token)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown delete token"))
    }

    pub(super) fn clear_restored_entry(&self, token: DeleteToken) -> io::Result<()> {
        let entries = lock(&self.entries)?.values().cloned().collect::<Vec<_>>();
        for entry in entries {
            let mut state = lock(&entry.state)?;
            if state.delete_token == Some(token.0) {
                state.delete_token = None;
                self.persist_restored_entry(&state)?;
            }
        }
        Ok(())
    }

    fn persist_restored_entry(&self, state: &super::engine::EntryState) -> io::Result<()> {
        if state.condition == EntryCondition::Clean {
            self.spool
                .forget_entry(&state.remote_path, &state.spool_name)
        } else {
            self.spool.persist_entry(&state.persisted())
        }
    }

    pub(super) fn finalize_committed_delete(&self, token: DeleteToken) -> io::Result<()> {
        let entries = lock(&self.entries)?.values().cloned().collect::<Vec<_>>();
        let mut committed_entries = Vec::new();
        for entry in entries {
            let mut state = lock(&entry.state)?;
            if state.delete_token == Some(token.0) {
                self.spool
                    .forget_entry(&state.remote_path, &state.spool_name)?;
                state.delete_token = None;
                state.delete_committed = true;
                let path = state.remote_path.clone();
                drop(state);
                // The namespace name is gone as soon as Cleanup commits the
                // delete. Old FILE_SHARE_DELETE handles retain this Arc and its
                // spool, but they must not prevent a new object from using the
                // same name before their eventual Close callbacks.
                let mut live_entries = lock(&self.entries)?;
                let key = self.cache_key(&path);
                if live_entries
                    .get(&key)
                    .is_some_and(|current| std::sync::Arc::ptr_eq(current, &entry))
                {
                    live_entries.remove(&key);
                }
                committed_entries.push(entry.clone());
            }
        }
        for entry in committed_entries {
            self.cleanup_committed_entry(&entry)?;
        }
        self.spool.forget_delete(token.0)?;
        lock(&self.deletes)?.remove(&token);
        Ok(())
    }

    pub(super) fn cleanup_committed_entry(
        &self,
        entry: &std::sync::Arc<super::engine::Entry>,
    ) -> io::Result<()> {
        if lock(&self.handles)?
            .values()
            .any(|handle| std::sync::Arc::ptr_eq(&handle.entry, entry))
        {
            return Ok(());
        }
        let state = lock(&entry.state)?;
        let removable_clean =
            state.condition == EntryCondition::Clean && state.delete_token.is_none();
        if !state.delete_committed && !removable_clean {
            return Ok(());
        }
        let path = state.remote_path.clone();
        let spool_name = state.spool_name.clone();
        drop(state);
        let mut entries = lock(&self.entries)?;
        let key = self.cache_key(&path);
        if entries
            .get(&key)
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, entry))
        {
            entries.remove(&key);
        }
        drop(entries);
        self.spool.remove_file(&spool_name)
    }

    fn has_visible_cached_child(&self, parent: &str) -> io::Result<bool> {
        let entries = lock(&self.entries)?.values().cloned().collect::<Vec<_>>();
        for entry in entries {
            let state = lock(&entry.state)?;
            if state.delete_token.is_none()
                && self.paths_equal(parent_path(&state.remote_path), parent)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
