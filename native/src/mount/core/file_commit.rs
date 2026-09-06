use super::engine::{baseline_from_meta, lock, Entry, EntryState, MountEngine};
use super::types::{Baseline, EntryCondition, FlushOutcome, MountConflict};
use crate::vfs::unique_staging_path;
use std::{io::{self, Write}, sync::Arc};
impl MountEngine {
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
        self.invalidate_content(&state.remote_path, false);
        self.invalidate_content(&staged, false);
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
        state.clean_since = std::time::Instant::now();
        if entry.pins.load(std::sync::atomic::Ordering::Acquire) == 0 {
            self.retirement_pending.store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(FlushOutcome::Committed)
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
