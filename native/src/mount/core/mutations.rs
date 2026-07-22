use super::engine::{baseline_from_meta, lock, write_lock, MountEngine};
use super::types::{
    Baseline, EntryCondition, FlushOutcome, MountConflict, MountMode, NamespaceIntent,
    NamespaceOperation, NamespaceOutcome, RenameOutcome,
};
use std::collections::HashMap;
use std::io;

impl MountEngine {
    pub fn mkdir(&self, callback_path: &str) -> io::Result<NamespaceOutcome> {
        self.require_writable()?;
        let _namespace = write_lock(&self.namespace)?;
        let path = self.project_checked(callback_path)?;
        if path.relative().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "mount root already exists",
            ));
        }
        if self.backend.try_exists(path.backend())? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "mounted path already exists",
            ));
        }
        let intent = NamespaceIntent {
            conflict: MountConflict {
                path: path.backend().to_string(),
                baseline: Baseline::Missing,
                current: None,
                detail: "directory creation is durably prepared but not yet reconciled".into(),
            },
            operation: NamespaceOperation::CreateDirectory,
            source_path: None,
            source_baseline: None,
            source_is_directory: None,
        };
        self.persist_namespace_intent(&intent)?;
        let mutation_error = self.backend.mkdir_all(path.backend()).err();
        self.invalidate_metadata(path.backend(), false);
        let result = match (mutation_error, self.backend.stat(path.backend())) {
            (None, Ok(created)) if created.is_dir && !created.is_symlink => {
                self.forget_namespace_conflict(path.backend())?;
                Ok(NamespaceOutcome::Complete)
            }
            (Some(error), Ok(created)) if created.is_dir && !created.is_symlink => {
                let detail =
                    format!("remote directory exists after an ambiguous create response: {error}");
                self.retain_namespace_conflict(MountConflict {
                    path: path.backend().to_string(),
                    baseline: Baseline::Missing,
                    current: Some(baseline_from_meta(&created)),
                    detail: detail.clone(),
                });
                Ok(NamespaceOutcome::CommittedPendingVerification {
                    path: path.backend().to_string(),
                    detail,
                })
            }
            (Some(mutation), Err(verify)) if verify.kind() == io::ErrorKind::NotFound => {
                self.forget_namespace_conflict(path.backend())?;
                Err(mutation)
            }
            (mutation_error, verification) => {
                let current = verification.as_ref().ok().map(baseline_from_meta);
                let detail = match (mutation_error, verification) {
                    (Some(mutation), Err(verify)) => format!(
                        "remote directory create was dispatched but neither its response nor final state is conclusive: {mutation}; verification: {verify}"
                    ),
                    (None, Err(verify)) => format!(
                        "remote directory create was acknowledged but its final state could not be verified: {verify}"
                    ),
                    (Some(mutation), Ok(_)) => format!(
                        "remote directory has an unsafe type after an ambiguous create response: {mutation}"
                    ),
                    (None, Ok(_)) => {
                        "remote directory create was acknowledged but the resulting entry is not a plain directory".into()
                    }
                };
                self.retain_namespace_conflict(MountConflict {
                    path: path.backend().to_string(),
                    baseline: Baseline::Missing,
                    current,
                    detail: detail.clone(),
                });
                Ok(NamespaceOutcome::CommittedPendingVerification {
                    path: path.backend().to_string(),
                    detail,
                })
            }
        };
        result
    }

    pub fn rename(
        &self,
        source_callback_path: &str,
        destination_callback_path: &str,
        replace_existing: bool,
    ) -> io::Result<RenameOutcome> {
        self.rename_with_shared_destination(
            source_callback_path,
            destination_callback_path,
            replace_existing,
            false,
        )
    }

    pub(crate) fn rename_with_shared_destination(
        &self,
        source_callback_path: &str,
        destination_callback_path: &str,
        replace_existing: bool,
        shared_destination_is_open: bool,
    ) -> io::Result<RenameOutcome> {
        self.require_writable()?;
        let _namespace = write_lock(&self.namespace)?;
        let source = self.project_checked(source_callback_path)?;
        let destination = self.project_checked(destination_callback_path)?;
        if source.relative().is_empty() || destination.relative().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mount root cannot be renamed",
            ));
        }
        if source.backend() == destination.backend() {
            return Ok(RenameOutcome::Complete);
        }
        if self.paths_equal(source.backend(), destination.backend()) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "case-only rename is unavailable without confirmed case-sensitive backend semantics",
            ));
        }
        if self.is_descendant(destination.backend(), source.backend()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mounted entry cannot be moved inside itself",
            ));
        }
        if self.namespace_conflict_exists(source.backend())?
            || self.namespace_conflict_exists(destination.backend())?
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "an unresolved namespace mutation prevents rename",
            ));
        }
        if replace_existing {
            return self.rename_replace_file(&source, &destination, shared_destination_is_open);
        }
        if self.entry_for_path(destination.backend())?.is_some()
            || self.backend.try_exists(destination.backend())?
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "rename destination already exists",
            ));
        }

        let affected = self.rename_affected_entries(source.backend())?;
        if affected.len() == 1 {
            let entry = &affected[0];
            let mut entries = lock(&self.entries)?;
            let mut state = lock(&entry.state)?;
            if self.paths_equal(&state.remote_path, source.backend())
                && state.baseline == Baseline::Missing
                && state.condition == EntryCondition::Dirty
                && state.delete_token.is_none()
            {
                let old_path = state.remote_path.clone();
                state.remote_path = destination.backend().to_string();
                let moved = state.persisted();
                if let Err(error) = self.spool.move_entry(&old_path, &moved) {
                    state.remote_path = old_path;
                    return Err(error);
                }
                drop(state);
                entries.remove(&self.cache_key(source.backend()));
                entries.insert(self.cache_key(destination.backend()), entry.clone());
                self.invalidate_metadata(source.backend(), false);
                self.invalidate_metadata(destination.backend(), false);
                return Ok(RenameOutcome::Complete);
            }
        }
        for entry in &affected {
            let state = lock(&entry.state)?;
            if state.delete_token.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "deleting mounted entries cannot be renamed",
                ));
            }
            let condition = state.condition.clone();
            drop(state);
            match condition {
                EntryCondition::Clean => {}
                EntryCondition::Dirty => match self.flush_entry(entry)? {
                    FlushOutcome::NoChanges | FlushOutcome::Committed => {}
                    FlushOutcome::CommittedPendingVerification(_) | FlushOutcome::Conflict(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "remote conflict prevents rename",
                        ));
                    }
                },
                EntryCondition::Conflict(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "unresolved remote conflict prevents rename",
                    ));
                }
            }
        }
        let mut guarded = Vec::with_capacity(affected.len());
        for entry in &affected {
            let state = lock(&entry.state)?;
            if state.condition != EntryCondition::Clean || state.delete_token.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "mounted entry changed while rename was being prepared",
                ));
            }
            guarded.push(state);
        }
        let source_meta = self.backend.stat(source.backend())?;
        if source_meta.is_symlink {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "link-like mounted entries cannot be renamed",
            ));
        }
        let mut planned_paths = Vec::with_capacity(guarded.len());
        for state in &guarded {
            let suffix = self
                .descendant_suffix(&state.remote_path, source.backend())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "rename path drift"))?;
            planned_paths.push((
                state.remote_path.clone(),
                format!("{}{suffix}", destination.backend()),
            ));
        }
        // Acquire every fallible local bookkeeping guard before dispatching
        // the remote mutation. Once rename_no_replace returns success, only
        // infallible HashMap/state updates remain and Windows cannot be told to
        // replay an already-committed namespace change.
        let mut entries = (!affected.is_empty())
            .then(|| lock(&self.entries))
            .transpose()?;
        let source_baseline = baseline_from_meta(&source_meta);
        let destination_path = destination.backend().to_string();
        let intent = NamespaceIntent {
            conflict: MountConflict {
                path: destination_path.clone(),
                baseline: Baseline::Missing,
                current: None,
                detail: format!(
                    "no-replace rename from {} is durably prepared but not yet reconciled",
                    source.backend()
                ),
            },
            operation: NamespaceOperation::RenameNoReplace,
            source_path: Some(source.backend().to_string()),
            source_baseline: Some(source_baseline.clone()),
            source_is_directory: Some(source_meta.is_dir),
        };
        self.persist_namespace_intent(&intent)?;
        let mut ambiguous_dispatch = None;
        let rename_result = self
            .backend
            .rename_no_replace(source.backend(), destination.backend());
        self.invalidate_metadata(source.backend(), true);
        self.invalidate_metadata(destination.backend(), true);
        if let Err(error) = rename_result {
            let source_after = self.observe_path(source.backend());
            let destination_after = self.observe_path(destination.backend());
            let pre_state_intact = source_after.matches(&source_baseline, Some(source_meta.is_dir))
                && destination_after.matches(&Baseline::Missing, None);
            if pre_state_intact {
                let _ = self.forget_namespace_conflict(destination.backend());
                return Err(error);
            }
            let commit_confirmed = matches!(&source_after, super::commit::PathObservation::Missing)
                && destination_after.matches(&source_baseline, Some(source_meta.is_dir));
            if !commit_confirmed {
                let conflict = MountConflict {
                    path: destination_path.clone(),
                    baseline: Baseline::Missing,
                    current: destination_after.current(),
                    detail: format!(
                        "remote no-replace rename from {} may already be committed after an ambiguous response: {error}; source={}; destination={}",
                        source.backend(),
                        source_after.summary(),
                        destination_after.summary()
                    ),
                };
                self.retain_namespace_conflict(conflict.clone());
                ambiguous_dispatch = Some(conflict);
            }
        }

        if let Some(entries) = entries.as_mut() {
            let mut replacements = HashMap::new();
            for ((entry, state), (old_path, new_path)) in
                affected.iter().zip(guarded.iter_mut()).zip(planned_paths)
            {
                state.remote_path = new_path;
                entries.remove(&self.cache_key(&old_path));
                replacements.insert(self.cache_key(&state.remote_path), entry.clone());
            }
            entries.extend(replacements);
        }
        if let Some(conflict) = ambiguous_dispatch {
            return Ok(RenameOutcome::CommittedPendingVerification(conflict));
        }
        match self.forget_namespace_conflict(destination.backend()) {
            Ok(()) => Ok(RenameOutcome::Complete),
            Err(error) => {
                let destination_after = self.observe_path(destination.backend());
                let conflict = MountConflict {
                    path: destination_path,
                    baseline: Baseline::Missing,
                    current: destination_after.current(),
                    detail: format!(
                        "remote no-replace rename from {} committed, but its durable intent marker could not be cleared: {error}; destination={}",
                        source.backend(),
                        destination_after.summary()
                    ),
                };
                self.retain_namespace_conflict(conflict.clone());
                Ok(RenameOutcome::CommittedPendingVerification(conflict))
            }
        }
    }

    pub fn flush_path(&self, callback_path: &str) -> io::Result<FlushOutcome> {
        let path = self.project_checked(callback_path)?;
        let entry = self
            .entry_for_path(path.backend())?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "path is not materialized"))?;
        self.flush_entry(&entry)
    }

    pub(super) fn require_writable(&self) -> io::Result<()> {
        if self.config.mode != MountMode::ReadWrite {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mount is read-only",
            ));
        }
        Ok(())
    }

    fn rename_affected_entries(
        &self,
        source: &str,
    ) -> io::Result<Vec<std::sync::Arc<super::engine::Entry>>> {
        let entries = lock(&self.entries)?;
        let mut affected = Vec::new();
        for entry in entries.values() {
            let state = lock(&entry.state)?;
            if self.descendant_suffix(&state.remote_path, source).is_some() {
                affected.push(entry.clone());
            }
        }
        Ok(affected)
    }
}
