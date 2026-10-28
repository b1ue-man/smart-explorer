use super::engine::{lock, read_lock, write_lock, MountEngine};
use super::types::{EntryCondition, FlushOutcome};
use std::io;

impl MountEngine {
    /// Replays only changes that are still provably retryable. Conflicts remain
    /// mounted and visible for manual recovery; an I/O failure aborts startup
    /// without discarding the durable spool so Retry can try again later.
    pub fn retry_pending_changes(&self) -> io::Result<()> {
        self.retry_pending(true)
    }

    /// Publishes every pending change now, also with write-back (a vault
    /// that is about to lock, S2-V4).
    pub(super) fn publish_pending_changes(&self) -> io::Result<()> {
        self.retry_pending(false)
    }

    /// `background`: dirty files may resume their upload in the background.
    fn retry_pending(&self, background: bool) -> io::Result<()> {
        {
            let _namespace = write_lock(&self.namespace)?;
            self.retry_delete_transactions_locked()?;
            self.retry_namespace_intents()?;
        }
        let _namespace = read_lock(&self.namespace)?;
        let entries = lock(&self.entries)?.values().cloned().collect::<Vec<_>>();
        for entry in entries {
            let condition = lock(&entry.state)?.condition.clone();
            if condition != EntryCondition::Dirty {
                continue;
            }
            // G4 ML1: with write-back the pending upload resumes in the
            // background; the journal entry stays `Dirty` until it is saved.
            if background && self.queue_upload(&entry)? {
                continue;
            }
            match self.flush_entry(&entry)? {
                FlushOutcome::NoChanges
                | FlushOutcome::Committed
                | FlushOutcome::CommittedPendingVerification(_)
                | FlushOutcome::Conflict(_) => {}
            }
        }
        Ok(())
    }
}
