use super::engine::{baseline_from_meta, lock, require_regular, Entry, EntryState, MountEngine};
use super::path::ProjectedPath;
use super::types::{
    Baseline, EntryCondition, FlushOutcome, MountConflict, OpenDisposition, RenameOutcome,
};
use std::io;
use std::sync::Arc;

struct ReplaceDestination {
    entry: Option<Arc<Entry>>,
    baseline: Baseline,
}

impl MountEngine {
    pub(super) fn rename_replace_file(
        &self,
        source: &ProjectedPath,
        destination: &ProjectedPath,
        shared_destination_is_open: bool,
    ) -> io::Result<RenameOutcome> {
        if source.backend() == destination.backend() {
            return Ok(RenameOutcome::Complete);
        }
        if self.paths_equal(source.backend(), destination.backend()) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "case-only replace rename is unavailable without confirmed case-sensitive backend semantics",
            ));
        }
        if !self.backend.rename_overwrites() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "backend has no proven atomic namespace replace primitive",
            ));
        }

        let destination_plan =
            self.prepare_replace_destination(destination, shared_destination_is_open)?;
        let source_entry = match self.entry_for_path(source.backend())? {
            Some(entry) => entry,
            None => self.materialize(source, OpenDisposition::OpenExisting)?,
        };
        if destination_plan
            .entry
            .as_ref()
            .is_some_and(|entry| Arc::ptr_eq(entry, &source_entry))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rename source and destination share one cache entry",
            ));
        }
        self.replace_materialized_file(source, destination, &source_entry, destination_plan)
    }

    fn prepare_replace_destination(
        &self,
        destination: &ProjectedPath,
        shared_destination_is_open: bool,
    ) -> io::Result<ReplaceDestination> {
        self.preserve_lazy_destination(destination, shared_destination_is_open)?;
        let entry = self.entry_for_path(destination.backend())?;
        let Some(entry) = entry else {
            let baseline = match self.backend.stat(destination.backend()) {
                Ok(metadata) => {
                    require_regular(&metadata)?;
                    baseline_from_meta(&metadata)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => Baseline::Missing,
                Err(error) => return Err(error),
            };
            return Ok(ReplaceDestination {
                entry: None,
                baseline,
            });
        };

        let destination_is_open = lock(&self.handles)?
            .values()
            .any(|opened| opened.references(&entry));
        if destination_is_open && !shared_destination_is_open {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "an open destination handle does not share delete access",
            ));
        }
        let mut state = lock(&entry.state)?;
        if state.delete_token.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "a pending destination delete prevents replacing rename",
            ));
        }
        if state.condition != EntryCondition::Clean {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "uncommitted destination changes prevent replacing rename",
            ));
        }

        let current = match self.backend.stat(destination.backend()) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let current_baseline = current.as_ref().map(baseline_from_meta);
        let regular = current
            .as_ref()
            .map_or(true, |metadata| !metadata.is_dir && !metadata.is_symlink);
        let unchanged = regular && baseline_matches(&state.baseline, &current_baseline);
        if !unchanged {
            let conflict = MountConflict {
                path: state.remote_path.clone(),
                baseline: state.baseline.clone(),
                current: current_baseline,
                detail: "replace destination changed since its cached baseline".into(),
            };
            self.spool
                .persist_entry(&state.with_condition(EntryCondition::Conflict(conflict.clone())))?;
            state.condition = EntryCondition::Conflict(conflict);
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "remote destination changed before replacing rename",
            ));
        }

        let plan = ReplaceDestination {
            entry: Some(entry.clone()),
            baseline: state.baseline.clone(),
        };
        drop(state);
        Ok(plan)
    }

    fn replace_materialized_file(
        &self,
        source: &ProjectedPath,
        destination_path: &ProjectedPath,
        source_entry: &Arc<Entry>,
        destination: ReplaceDestination,
    ) -> io::Result<RenameOutcome> {
        self.flush_replace_source(source_entry)?;

        // Holding the source state prevents an already-open source handle from
        // writing between the verified flush and the atomic backend promotion.
        let mut state = lock(&source_entry.state)?;
        if state.condition != EntryCondition::Clean || state.delete_token.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "source changed while replacing rename was being prepared",
            ));
        }
        let source_metadata = self.backend.stat(source.backend())?;
        require_regular(&source_metadata)?;
        let current_baseline = baseline_from_meta(&source_metadata);
        if state.baseline != current_baseline {
            return self.mark_source_conflict(&mut state, current_baseline);
        }
        // Open destination handles may continue to address their old object
        // after a delete-sharing replace. Keep its state locked from the final
        // revalidation through promotion so a concurrent write cannot race the
        // namespace replacement.
        let mut destination_state = match destination.entry.as_ref() {
            Some(entry) => Some(lock(&entry.state)?),
            None => None,
        };
        self.revalidate_destination(
            destination_path,
            &destination,
            destination_state.as_deref_mut(),
        )?;

        // Reserve ownership before dispatch. On failure the live entry still
        // owns the same object; cleanup removes both registry references.
        if let Some(entry) = destination.entry.as_ref() {
            let name = destination_state.as_ref().map(|state| state.spool_name.clone())
                .ok_or_else(|| io::Error::other("replace destination state is absent"))?;
            lock(&self.detached)?.insert(name, Arc::clone(entry));
        }
        self.invalidate_content(source.backend(), true);
        self.invalidate_content(destination_path.backend(), true);

        // A durable dirty record makes a missing/ambiguous mutation reply retain
        // the source cache instead of allowing a clean unmount.
        if let Err(error) = self.spool.persist_entry(&state.with_condition(EntryCondition::Dirty)) {
            state.condition = EntryCondition::Conflict(MountConflict {
                path: state.remote_path.clone(), baseline: state.baseline.clone(), current: None,
                detail: format!("replace preparation journal durability is uncertain: {error}"),
            });
            return Err(error);
        }
        state.condition = EntryCondition::Dirty;
        let mut ambiguous_promotion = None;
        let promotion_result = self
            .backend
            .promote_staged(source.backend(), destination_path.backend());
        self.invalidate_metadata(source.backend(), true);
        self.invalidate_metadata(destination_path.backend(), true);
        if let Err(error) = promotion_result {
            let source_after = self.observe_path(source.backend());
            let destination_after = self.observe_path(destination_path.backend());
            let pre_state_intact = source_after.matches(&current_baseline, Some(false))
                && destination_after.matches(&destination.baseline, Some(false));
            if pre_state_intact {
                if self
                    .spool
                    .forget_entry(&state.remote_path, &state.spool_name)
                    .is_ok()
                {
                    state.condition = EntryCondition::Clean;
                }
                return Err(error);
            }
            let commit_confirmed = matches!(&source_after, super::commit::PathObservation::Missing)
                && destination_after.matches(&current_baseline, Some(false));
            if !commit_confirmed {
                ambiguous_promotion = Some((
                    format!(
                        "remote replacing rename may already be committed after an ambiguous response: {error}; source={}; destination={}",
                        source_after.summary(),
                        destination_after.summary()
                    ),
                    destination_after.current(),
                ));
            }
        }

        if let Some(destination_state) = destination_state.as_deref_mut() {
            // The old object is now detached from its namespace name. Existing
            // handles retain the Arc and spool; their later flush is local-only
            // and the last close removes that old spool without touching the
            // new destination entry.
            destination_state.delete_committed = true;
        }

        let source_path = source.backend().to_string();
        let destination_path = destination_path.backend().to_string();
        let mut moved = state.persisted();
        moved.remote_path = destination_path.clone();
        moved.baseline = destination.baseline.clone();
        let journal_move_error = self.spool.move_entry(&source_path, &moved).err();

        state.remote_path = destination_path.clone();
        state.baseline = destination.baseline.clone();
        let map_error = match lock(&self.entries) {
            Ok(mut entries) => {
                entries.remove(&self.cache_key(&source_path));
                entries.remove(&self.cache_key(&destination_path));
                entries.insert(self.cache_key(&destination_path), source_entry.clone());
                None
            }
            Err(error) => Some(error),
        };
        drop(destination_state);

        let detached_cleanup_error = destination
            .entry
            .as_ref()
            .and_then(|entry| self.cleanup_committed_entry(entry).err());
        if journal_move_error.is_some() || map_error.is_some() || detached_cleanup_error.is_some() {
            let mut details = Vec::new();
            if let Some(error) = journal_move_error.as_ref() {
                details.push(format!(
                    "recovery journal did not follow the rename: {error}"
                ));
            }
            if let Some(error) = map_error.as_ref() {
                details.push(format!(
                    "live namespace cache did not follow the rename: {error}"
                ));
            }
            if let Some(error) = detached_cleanup_error.as_ref() {
                details.push(format!("replaced cache cleanup is pending: {error}"));
            }
            let current = self
                .backend
                .stat(&destination_path)
                .ok()
                .map(|metadata| baseline_from_meta(&metadata));
            let conflict = MountConflict {
                path: destination_path,
                baseline: state.baseline.clone(),
                current,
                detail: format!("remote rename committed; {}", details.join("; ")),
            };
            // If MoveEntry failed, the one authoritative durable record may
            // still live under the old source path. Appending a destination
            // entry could make two journal records reference one spool.
            if journal_move_error.is_none() {
                let _ = self.spool.persist_entry(
                    &state.with_condition(EntryCondition::Conflict(conflict.clone())),
                );
            }
            state.condition = EntryCondition::Conflict(conflict.clone());
            return Ok(RenameOutcome::CommittedPendingVerification(conflict));
        }

        if let Some((detail, current)) = ambiguous_promotion {
            let conflict = self.post_commit_conflict(&mut state, current, &detail);
            return Ok(RenameOutcome::CommittedPendingVerification(conflict));
        }

        let outcome = match self.backend.stat(&destination_path) {
            Ok(metadata) if !metadata.is_dir && !metadata.is_symlink => {
                let committed_baseline = baseline_from_meta(&metadata);
                match self
                    .spool
                    .forget_entry(&state.remote_path, &state.spool_name)
                {
                    Ok(()) => {
                        state.baseline = committed_baseline;
                        state.condition = EntryCondition::Clean;
                        state.clean_since = std::time::Instant::now();
                        RenameOutcome::Complete
                    }
                    Err(error) => {
                        let conflict = self.post_commit_conflict(
                            &mut state,
                            Some(committed_baseline),
                            &format!(
                                "remote rename committed but its recovery journal could not be cleared: {error}"
                            ),
                        );
                        RenameOutcome::CommittedPendingVerification(conflict)
                    }
                }
            }
            Ok(metadata) => {
                let conflict = self.post_commit_conflict(
                    &mut state,
                    Some(baseline_from_meta(&metadata)),
                    "backend reported a non-regular destination after successful promotion",
                );
                RenameOutcome::CommittedPendingVerification(conflict)
            }
            Err(error) => {
                let conflict = self.post_commit_conflict(
                    &mut state,
                    None,
                    &format!(
                        "remote rename committed but its destination could not be verified: {error}"
                    ),
                );
                RenameOutcome::CommittedPendingVerification(conflict)
            }
        };
        Ok(outcome)
    }

    fn revalidate_destination(
        &self,
        destination_path: &ProjectedPath,
        destination: &ReplaceDestination,
        mut state: Option<&mut EntryState>,
    ) -> io::Result<()> {
        if state.as_ref().is_some_and(|state| {
            state.condition != EntryCondition::Clean || state.delete_token.is_some()
        }) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "destination changed while replacing rename was being prepared",
            ));
        }
        let current = match self.backend.stat(destination_path.backend()) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let current_baseline = current.as_ref().map(baseline_from_meta);
        let regular = current
            .as_ref()
            .map_or(true, |metadata| !metadata.is_dir && !metadata.is_symlink);
        if regular && baseline_matches(&destination.baseline, &current_baseline) {
            return Ok(());
        }
        let Some(state) = state.as_deref_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "remote destination changed before replacing rename",
            ));
        };
        let conflict = MountConflict {
            path: state.remote_path.clone(),
            baseline: state.baseline.clone(),
            current: current_baseline,
            detail: "replace destination changed immediately before promotion".into(),
        };
        self.spool
            .persist_entry(&state.with_condition(EntryCondition::Conflict(conflict.clone())))?;
        state.condition = EntryCondition::Conflict(conflict);
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "remote destination changed before replacing rename",
        ))
    }

    fn flush_replace_source(&self, entry: &Arc<Entry>) -> io::Result<()> {
        let condition = {
            let state = lock(&entry.state)?;
            if state.delete_token.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "a deleting source cannot replace another file",
                ));
            }
            state.condition.clone()
        };
        match condition {
            EntryCondition::Clean => Ok(()),
            EntryCondition::Dirty => match self.flush_entry(entry)? {
                FlushOutcome::NoChanges | FlushOutcome::Committed => Ok(()),
                FlushOutcome::CommittedPendingVerification(_) | FlushOutcome::Conflict(_) => {
                    Err(source_conflict())
                }
            },
            EntryCondition::Conflict(_) => Err(source_conflict()),
        }
    }

    fn mark_source_conflict(
        &self,
        state: &mut super::engine::EntryState,
        current: Baseline,
    ) -> io::Result<RenameOutcome> {
        let conflict = MountConflict {
            path: state.remote_path.clone(),
            baseline: state.baseline.clone(),
            current: Some(current),
            detail: "remote source changed before replacing rename".into(),
        };
        self.spool
            .persist_entry(&state.with_condition(EntryCondition::Conflict(conflict.clone())))?;
        state.condition = EntryCondition::Conflict(conflict);
        Err(source_conflict())
    }
}

fn source_conflict() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "unresolved source changes prevent replacing rename",
    )
}

fn baseline_matches(expected: &Baseline, current: &Option<Baseline>) -> bool {
    match (expected, current) {
        (Baseline::Missing, None) => true,
        (expected @ Baseline::Present { .. }, Some(actual)) => expected == actual,
        _ => false,
    }
}
