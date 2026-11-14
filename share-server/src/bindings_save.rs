//! Saving the key bindings (R-SERVER-03, SEC-SERVER-BESITZER-60S Teil 3).
//!
//! A copy of the bindings is taken while the state lock is held; the copy is
//! written after the lock was released. [`SaveOrder`] serializes the writes and
//! orders them by generation: a copy older than the newest one on disk never
//! overwrites it.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::{Bindings, ACCESS_SAVE_INTERVAL_SECS, STATE_VERSION};
use crate::discovery_state::unix_seconds;
use crate::state::{lock_state, State};

/// The order of the copies on disk: the generation of the newest copy written,
/// and the lock that serializes the writes (held during each write).
#[derive(Default)]
pub(crate) struct SaveOrder {
    written: AtomicU64,
    write_lock: Mutex<()>,
}

/// A copy of the bindings as they were when its generation was taken.
pub(crate) struct Snapshot {
    generation: u64,
    bindings: Bindings,
}

impl SaveOrder {
    /// The generation of the newest copy on disk. It is read without the write
    /// lock, so the state lock never waits for a file write.
    pub(crate) fn written(&self) -> u64 {
        self.written.load(Ordering::SeqCst)
    }

    /// Writes `snapshot` unless a newer copy is already on disk. The generation
    /// of the newest copy is kept only after a successful write.
    pub(crate) fn write(&self, path: &Path, snapshot: &Snapshot) -> io::Result<()> {
        let _write = self
            .write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if snapshot.generation < self.written() {
            return Ok(());
        }
        snapshot.bindings.save(path)?;
        self.written.store(snapshot.generation, Ordering::SeqCst);
        Ok(())
    }
}

impl Bindings {
    /// Whether a save is due: an owner was added, or renewed access hashes wait
    /// and the last save is [`ACCESS_SAVE_INTERVAL_SECS`] ago (R-SERVER-03).
    pub(crate) fn save_due(&self, now: i64) -> bool {
        let access_due = self.access_dirty
            && self
                .saved_at
                .is_none_or(|at| now.saturating_sub(at) >= ACCESS_SAVE_INTERVAL_SECS);
        self.dirty || access_due
    }

    /// Takes the copy for a due save and clears the pending flags. Called while
    /// the state lock is held; a failed write calls [`Bindings::restore_pending`].
    #[cfg(test)]
    pub(crate) fn take_snapshot(&mut self, now: i64) -> Option<Snapshot> {
        if !self.save_due(now) {
            return None;
        }
        Some(self.snapshot(now))
    }

    /// Takes the copy whether or not a save is due. A new binding must be on disk
    /// before its registration is acknowledged, even when another copy already
    /// cleared the pending flags.
    pub(crate) fn snapshot_now(&mut self, now: i64) -> Snapshot {
        self.snapshot(now)
    }

    fn snapshot(&mut self, now: i64) -> Snapshot {
        self.dirty = false;
        self.access_dirty = false;
        self.saved_at = Some(now);
        self.generation = self.generation.saturating_add(1);
        let mut bindings = self.clone();
        bindings.version = STATE_VERSION;
        Snapshot {
            generation: self.generation,
            bindings,
        }
    }

    /// Keeps a failed save pending for the next call.
    pub(crate) fn restore_pending(&mut self) {
        self.dirty = true;
    }

    /// The generation a copy must reach on disk before a change it holds is
    /// acknowledged: the next copy while a change is pending, else the newest one.
    pub(crate) fn ack_generation(&self) -> u64 {
        if self.dirty {
            self.generation.saturating_add(1)
        } else {
            self.generation
        }
    }

    /// Saves in place for the tests of the synchronous path; a failed save stays
    /// pending.
    #[cfg(test)]
    pub(crate) fn save_pending(&mut self, path: &Path, now: i64) -> io::Result<()> {
        let Some(snapshot) = self.take_snapshot(now) else {
            return Ok(());
        };
        let result = snapshot.bindings.save(path);
        if result.is_err() {
            self.restore_pending();
        }
        result
    }
}

impl State {
    /// Takes, under the state lock, the copy that a direct publication needs before
    /// its entry becomes visible (R1). `None` without a state file, and when nothing
    /// is due and the newest copy is already on disk (R-SERVER-03).
    pub(crate) fn take_publication_copy(&mut self) -> Option<PendingCopy> {
        let path = self.binding_path.clone()?;
        let now = unix_seconds();
        if !self.bindings.save_due(now)
            && self.binding_saves.written() >= self.bindings.ack_generation()
        {
            return None;
        }
        let snapshot = self.bindings.snapshot_now(now);
        Some(PendingCopy {
            path,
            order: self.binding_saves.clone(),
            snapshot,
        })
    }
}

/// A copy of the bindings taken under the state lock, written after the lock was
/// released (R1).
pub(crate) struct PendingCopy {
    path: PathBuf,
    order: Arc<SaveOrder>,
    snapshot: Snapshot,
}

impl PendingCopy {
    /// Writes the copy outside the state lock. A failed write keeps the change pending
    /// for the next save; the caller then refuses the publication.
    pub(crate) fn write(self, state: &Arc<Mutex<State>>) -> io::Result<()> {
        let result = self.order.write(&self.path, &self.snapshot);
        if result.is_err() {
            lock_state(state).bindings.restore_pending();
        }
        result
    }
}

/// Saves a new binding after the state lock was released (registration). Only the
/// copy is taken under the lock; serialization and the file write run outside it.
/// The registration goes on only once this copy, or a newer one, is on disk.
pub(crate) fn persist_outside_lock(state: &Arc<Mutex<State>>) -> io::Result<()> {
    let (path, saves, snapshot) = {
        let mut state = lock_state(state);
        let Some(path) = state.binding_path.clone() else {
            return Ok(());
        };
        let snapshot = state.bindings.snapshot_now(unix_seconds());
        (path, state.binding_saves.clone(), snapshot)
    };
    let result = saves.write(&path, &snapshot);
    if result.is_err() {
        lock_state(state).bindings.restore_pending();
    }
    result
}
