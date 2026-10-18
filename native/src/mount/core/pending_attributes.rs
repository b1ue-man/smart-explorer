//! Times and Unix modes a POSIX frontend sets on a file whose content is not
//! published yet (DRV-M4, decision C2 of strand S2, plan 2026-10-17).
//!
//! `utimensat`, `futimens`, `chmod` and `fchmod` on a dirty entry are kept
//! here under the entry's spool name and handed to the backend's
//! `finish_stage` when that content is published, right before the stage
//! takes the user's file name. A target that keeps its own times
//! (`mtime_applied: false`) or no Unix modes is no error; that is the
//! contract of `BackendExtensions::finish_stage`. A change on a clean file
//! is success without effect: there is no publication it could ride on.
//!
//! Windows drives store nothing and never call `finish_stage`: Dokany
//! answers time and attribute changes with `ERROR_NOT_SUPPORTED`, and a
//! drive of the Windows dialect refuses `set_pending_attributes`. The values
//! live only in memory; after a host crash the content is still retried,
//! the requested times and modes are not.
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Mutex;

use crate::vfs::{StageDurability, StageFinish};

use super::engine::{lock, MountEngine, OpenHandleKind};
use super::frontend::PendingAttributes;
use super::path_dialect::PathDialect;
use super::types::{EntryCondition, HandleId};

/// Pending attributes by spool name. A spool name belongs to one entry for
/// its whole life, also across renames, and is never reused.
#[derive(Default)]
pub(super) struct PendingAttributeTable {
    by_spool: Mutex<HashMap<String, PendingAttributes>>,
}

impl PendingAttributeTable {
    /// A later value replaces an earlier one field by field.
    fn merge(&self, spool_name: &str, attributes: PendingAttributes) -> io::Result<()> {
        let mut table = lock(&self.by_spool)?;
        let stored = table.entry(spool_name.to_string()).or_default();
        if attributes.mtime_ms.is_some() {
            stored.mtime_ms = attributes.mtime_ms;
        }
        if attributes.mode.is_some() {
            stored.mode = attributes.mode;
        }
        Ok(())
    }

    pub(super) fn get(&self, spool_name: &str) -> Option<PendingAttributes> {
        self.by_spool.lock().ok()?.get(spool_name).copied()
    }

    /// After a publication, or when the entry left the namespace.
    pub(super) fn forget(&self, spool_name: &str) {
        if let Ok(mut table) = self.by_spool.lock() {
            table.remove(spool_name);
        }
    }
}

impl MountEngine {
    /// Stores times and mode for the next publication of the dirty entry
    /// behind `handle`. A clean or lazily opened file and a file whose name
    /// was deleted keep nothing (success without effect, decision C2).
    pub fn set_pending_attributes(
        &self,
        handle: HandleId,
        attributes: PendingAttributes,
    ) -> io::Result<()> {
        if self.projector.dialect() != PathDialect::Posix {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this drive does not store times or modes of pending files",
            ));
        }
        let opened = self.handle(handle)?;
        let OpenHandleKind::Materialized(entry) = &opened.kind else {
            return Ok(());
        };
        let state = lock(&entry.state)?;
        let pending = !state.delete_committed
            && state.delete_token.is_none()
            && state.condition == EntryCondition::Dirty;
        if !pending {
            return Ok(());
        }
        self.pending_attributes.merge(&state.spool_name, attributes)
    }

    /// Gives the uploaded `staged` copy of the entry with `spool_name` its
    /// pending times and mode; without pending values nothing is called.
    pub(super) fn finish_pending_stage(&self, spool_name: &str, staged: &str) -> io::Result<()> {
        let Some(attributes) = self.pending_attributes.get(spool_name) else {
            return Ok(());
        };
        let finished = crate::vfs::finish_stage(&*self.backend, staged, stage_finish(attributes))?;
        if attributes.mtime_ms.is_some() && !finished.mtime_applied {
            let _ = writeln!(
                io::stderr().lock(),
                "mount: this target keeps its own modification times; the requested time was not stored"
            );
        }
        Ok(())
    }
}

/// What a publication asks of `finish_stage`: the requested time, the mode
/// bits only (`0o777`), and no extra durability (a save keeps the flush rules
/// of earlier releases).
pub(super) fn stage_finish(attributes: PendingAttributes) -> StageFinish {
    StageFinish {
        mtime_ms: attributes.mtime_ms,
        mode: attributes.mode.map(|mode| mode & 0o777),
        durability: StageDurability::NotRequired,
    }
}

#[cfg(test)]
#[path = "pending_attributes_tests.rs"]
mod tests;
