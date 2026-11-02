use super::engine::{lock, MountEngine};
use super::journal::DeletePhase;
use std::io;

impl MountEngine {
    pub(super) fn retry_delete_transactions_locked(&self) -> io::Result<()> {
        for token in self.pending_deletes()? {
            let mut delete = self.delete_for_token(token)?;
            if delete.phase == DeletePhase::LocalOnly {
                self.finalize_committed_delete(token)?;
                continue;
            }
            let source_exists = self.backend.try_exists(&delete.original_path)?;
            let quarantine_exists = self.backend.try_exists(&delete.quarantine_path)?;
            match (source_exists, quarantine_exists) {
                (false, true) => {
                    if matches!(
                        delete.phase,
                        DeletePhase::Unresolved | DeletePhase::Prepared
                    ) {
                        delete.phase = DeletePhase::Moved;
                        self.spool.persist_delete(&delete)?;
                        lock(&self.deletes)?.insert(token, delete);
                    }
                    self.commit_delete_locked(token)?;
                }
                (true, false) => {
                    self.clear_restored_entry(token)?;
                    self.spool.forget_delete(token.0)?;
                    lock(&self.deletes)?.remove(&token);
                }
                (false, false) => self.finalize_committed_delete(token)?,
                (true, true) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "delete recovery found both the original and quarantine path",
                    ));
                }
            }
        }
        Ok(())
    }
}
