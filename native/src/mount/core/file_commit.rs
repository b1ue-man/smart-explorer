use super::engine::{baseline_from_meta, lock, Entry, EntryState, MountEngine};
use super::types::{Baseline, EntryCondition, FlushOutcome, MountConflict};
use crate::vfs::unique_staging_path;
use std::{
    io::{self, Write},
    sync::Arc,
};
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
        let spool_len = source.metadata()?.len();
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
            // The spool keeps the content for the retry; a stage left next to
            // the user's file would show up as `<name>.se-mount-…`.
            self.discard_own_stage(&staged, spool_len);
            return Err(error);
        }
        // Uploading a whole-file spool may take minutes. Revalidate immediately
        // before the atomic promotion so a remote edit during that transfer is
        // not silently overwritten.
        match self.detect_conflict(&state) {
            Ok(Some(mut conflict)) => {
                // Never overwrite the changed remote file, and never leave this
                // save under the stage spelling: publish it as a conflict copy
                // that keeps the user's file name and type.
                if let Some(copy) = self.publish_conflict_copy(&staged, &state.remote_path) {
                    conflict.detail = format!(
                        "{}; die gespeicherte Fassung liegt als {copy}",
                        conflict.detail
                    );
                }
                let persisted = state.with_condition(EntryCondition::Conflict(conflict.clone()));
                self.spool.persist_entry(&persisted)?;
                state.condition = EntryCondition::Conflict(conflict.clone());
                return Ok(FlushOutcome::Conflict(conflict));
            }
            Ok(None) => {}
            Err(error) => {
                // Verification is inconclusive: the destination stays as it
                // is, the spool keeps the content for the retry.
                self.discard_own_stage(&staged, spool_len);
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
                // observation that proves the promotion did not take effect;
                // the spool keeps the content for the retry.
                self.discard_own_stage(&staged, spool_len);
                return Err(error);
            }
            if matches!(state.baseline, Baseline::Missing)
                && error.kind() == io::ErrorKind::AlreadyExists
                && staged_state.is_plain_file()
            {
                // A no-replace publication refused by a name that appeared in
                // the meantime did not take effect: keep the other file and
                // publish this save beside it, never under the stage spelling.
                let mut conflict = MountConflict {
                    path: state.remote_path.clone(),
                    baseline: state.baseline.clone(),
                    current: destination.current(),
                    detail: "a file with this name appeared on the remote while saving".into(),
                };
                if let Some(copy) = self.publish_conflict_copy(&staged, &state.remote_path) {
                    conflict.detail = format!(
                        "{}; die gespeicherte Fassung liegt als {copy}",
                        conflict.detail
                    );
                }
                let persisted = state.with_condition(EntryCondition::Conflict(conflict.clone()));
                self.spool.persist_entry(&persisted)?;
                state.condition = EntryCondition::Conflict(conflict.clone());
                return Ok(FlushOutcome::Conflict(conflict));
            }
            let mut detail = format!(
                "remote save may already be committed after an ambiguous promotion response: {error}; destination={}; staging={}",
                destination.summary(),
                staged_state.summary()
            );
            if staged_state.is_plain_file() {
                // The stage still holds this save. Whatever the promotion did,
                // the save must not stay visible under the stage spelling.
                if let Some(copy) = self.publish_conflict_copy(&staged, &state.remote_path) {
                    detail.push_str(&format!("; die gespeicherte Fassung liegt als {copy}"));
                }
            }
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
        entry.schedule_retirement();
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
                (expected @ Baseline::Present { .. }, Some(actual)) => {
                    expected.same_remote_state(actual)
                }
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
