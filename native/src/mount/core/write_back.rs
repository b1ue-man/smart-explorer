//! Asynchronous write-back of the mount engine (G4 ML1): `close` returns
//! once the file is safe in the local write cache, the upload follows in the
//! background (`upload_queue.rs`). `fsync`/`FlushFileBuffers` and every
//! namespace operation keep publishing synchronously (`flush`).
use super::engine::{lock, Entry, MountEngine, OpenHandleKind};
use super::types::{EntryCondition, FlushOutcome, HandleId, MountConflict};
use super::upload_queue::{MountUploadStatus, Uploaded};
use std::io;
use std::sync::Arc;

impl MountEngine {
    /// Starts background saves for this shared engine. Up to the backend's
    /// request capacity (the drive gate's limit, ML8) uploads run at once;
    /// workers start when there is work and end with the engine.
    pub fn start_write_back(self: &Arc<Self>) {
        let limit = self.backend.parallelism().max(1);
        self.write_back.queue().attach(Arc::downgrade(self), limit);
    }

    /// The save of a closing handle (FUSE `flush`, Dokany `Cleanup`). With
    /// write-back: the spool content is made durable, its `Dirty` journal
    /// record already is, and the upload is queued; the caller sees
    /// `NoChanges`. Without write-back, or for anything but a plain dirty
    /// file, this is `flush`.
    pub fn flush_on_close(&self, handle: HandleId) -> io::Result<FlushOutcome> {
        let _reap = self.operation_reaper();
        let opened = self.handle(handle)?;
        let OpenHandleKind::Materialized(entry) = opened.kind else {
            return Ok(FlushOutcome::NoChanges);
        };
        let queue = self.write_back.queue();
        if !queue.accepts() {
            return self.flush_entry(&entry);
        }
        let bytes = {
            let state = lock(&entry.state)?;
            let deferrable = state.condition == EntryCondition::Dirty
                && state.delete_token.is_none()
                && !state.delete_committed;
            if !deferrable {
                drop(state);
                return self.flush_entry(&entry);
            }
            let mut spool = self.spool.open_file(&state.spool_name, true)?;
            spool.sync_data()?;
            spool.metadata()?.len()
        };
        if queue.enqueue(Arc::clone(&entry), bytes) {
            Ok(FlushOutcome::NoChanges)
        } else {
            self.flush_entry(&entry)
        }
    }

    /// Queues a dirty entry for upload; `false` without write-back.
    pub(super) fn queue_upload(&self, entry: &Arc<Entry>) -> io::Result<bool> {
        let queue = self.write_back.queue();
        if !queue.accepts() {
            return Ok(false);
        }
        let bytes = {
            let state = lock(&entry.state)?;
            self.spool
                .open_file(&state.spool_name, false)
                .and_then(|file| file.metadata())
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        };
        Ok(queue.enqueue(Arc::clone(entry), bytes))
    }

    /// One background upload attempt of `entry`.
    pub(super) fn upload_once(&self, entry: &Arc<Entry>) -> Uploaded {
        let _reap = self.operation_reaper();
        match self.flush_entry_with(entry, true) {
            Ok(FlushOutcome::NoChanges | FlushOutcome::Committed) => Uploaded::Done,
            Ok(FlushOutcome::Conflict(found)) => conflict(found),
            Ok(FlushOutcome::CommittedPendingVerification(found)) => conflict(found),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Uploaded::Changed,
            Err(error) => match Self::still_dirty(entry) {
                Ok(false) => Uploaded::Done,
                Ok(true) | Err(_) => Uploaded::Failed(error.to_string()),
            },
        }
    }

    /// At unmount: no new background work; every queued upload, retries
    /// included, is tried once more now, and running ones end. What still
    /// fails stays `Dirty` in the journal for the next mount or Retry.
    pub fn finish_write_back(&self) {
        let queue = self.write_back.queue();
        queue.close();
        while let Some(entry) = queue.take_for_drain() {
            let outcome = self.upload_once(&entry);
            queue.done(&entry, outcome);
        }
        queue.wait_idle();
    }

    /// Pending background uploads; `None` without write-back.
    pub fn upload_status(&self) -> Option<MountUploadStatus> {
        self.write_back.queue().status()
    }

    /// Conflicts that background uploads found since the last call.
    pub fn take_upload_conflicts(&self) -> Vec<(String, EntryCondition)> {
        self.write_back.queue().take_conflicts()
    }
}

fn conflict(found: MountConflict) -> Uploaded {
    Uploaded::Conflict(found.path.clone(), EntryCondition::Conflict(found))
}
