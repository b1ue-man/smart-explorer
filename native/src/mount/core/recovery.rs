use super::engine::{lock, read_lock, write_lock, MountEngine};
use super::types::{EntryCondition, FlushOutcome};
use std::io;

impl MountEngine {
    /// Replays only changes that are still provably retryable. Conflicts remain
    /// mounted and visible for manual recovery; an I/O failure aborts startup
    /// without discarding the durable spool so Retry can try again later.
    pub fn retry_pending_changes(&self) -> io::Result<()> {
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
