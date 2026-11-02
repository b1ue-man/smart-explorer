use super::{identity_key, support::join};
use crate::vfs::VfsMeta;
use std::collections::{HashMap, LinkedList};
use std::mem::size_of;
use std::sync::Arc;

// At least one maximum-size old/new snapshot pair plus bookkeeping must fit.
// Conservatively charging shared Arcs is intentional; this is not allocation.
const MAX_PENDING_BYTES: usize = 3 * super::MAX_CACHED_BYTES;
const MAX_DRAIN_RECORDS: usize = 1_024;
const MAX_DRAIN_COMPARISONS: usize = 4_096;

/// Paths are absolute inside the authorized backend, not Windows drive paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataChange {
    Created { path: String, is_directory: bool },
    Deleted { path: String, is_directory: bool },
    Modified { path: String },
}

impl MetadataChange {
    pub fn path(&self) -> &str {
        match self {
            Self::Created { path, .. } | Self::Deleted { path, .. }
            | Self::Modified { path } => path,
        }
    }
}

#[derive(Clone)]
pub(super) struct SnapshotImage {
    pub entries: Arc<[VfsMeta]>,
    pub index: Arc<HashMap<String, usize>>,
    // Conservatively counts each retained image even when its Arcs are shared.
    pub bytes: usize,
}

pub(super) struct PendingDiff {
    path: String,
    previous: SnapshotImage,
    current: SnapshotImage,
    case_sensitive: bool,
    old_cursor: usize,
    new_cursor: usize,
    pending_created: Option<usize>,
    started: bool,
}

enum Scan {
    Change(MetadataChange),
    Finished,
    Pending,
}

impl PendingDiff {
    fn bytes(&self) -> usize {
        self.previous.bytes.saturating_add(self.current.bytes)
            .saturating_add(self.path.capacity()).saturating_add(size_of::<Self>())
            .saturating_add(2 * size_of::<usize>())
    }

    fn next(&mut self, comparisons: &mut usize) -> Scan {
        if *comparisons == 0 { return Scan::Pending; }
        if let Some(index) = self.pending_created.take() {
            *comparisons -= 1;
            let new = &self.current.entries[index];
            return Scan::Change(MetadataChange::Created {
                path: join(&self.path, &new.name), is_directory: new.is_dir,
            });
        }
        while let Some(old) = self.previous.entries.get(self.old_cursor) {
            if *comparisons == 0 { return Scan::Pending; }
            *comparisons -= 1;
            self.started = true;
            self.old_cursor += 1;
            let key = identity_key(self.case_sensitive, &old.name);
            let index = self.current.index.get(&key).copied();
            let new = index.and_then(|index| self.current.entries.get(index));
            let change = match new {
                None => Some(MetadataChange::Deleted {
                    path: join(&self.path, &old.name), is_directory: old.is_dir,
                }),
                Some(new) if old.name != new.name || old.is_dir != new.is_dir
                    || old.is_symlink != new.is_symlink => {
                    self.pending_created = index;
                    Some(MetadataChange::Deleted {
                        path: join(&self.path, &old.name), is_directory: old.is_dir,
                    })
                }
                Some(new) if !same(old, new) => Some(MetadataChange::Modified {
                    path: join(&self.path, &new.name),
                }),
                Some(_) => None,
            };
            if let Some(change) = change {
                self.started = true;
                return Scan::Change(change);
            }
        }
        while let Some(new) = self.current.entries.get(self.new_cursor) {
            if *comparisons == 0 { return Scan::Pending; }
            *comparisons -= 1;
            self.started = true;
            self.new_cursor += 1;
            if !self.previous.index.contains_key(&identity_key(self.case_sensitive, &new.name)) {
                self.started = true;
                return Scan::Change(MetadataChange::Created {
                    path: join(&self.path, &new.name), is_directory: new.is_dir,
                });
            }
        }
        Scan::Finished
    }
}

pub(super) struct PreparedDiff {
    pending: Option<PendingDiff>,
    replacement: bool,
}

#[derive(Default)]
pub(super) struct ChangeQueue {
    // Endpoint-only ownership avoids retaining a peak-capacity backing array
    // after a large burst has drained; node storage is charged in bytes().
    pending: LinkedList<PendingDiff>,
    bytes: usize,
    #[cfg(test)]
    test_byte_budget: Option<usize>,
}

impl ChangeQueue {
    #[cfg(test)]
    pub(super) fn set_test_byte_budget(&mut self, budget: Option<usize>) -> usize {
        self.test_byte_budget = budget;
        self.bytes
    }

    /// Pure admission, called under the snapshot mutex before any cache change.
    /// None means backpressure: the caller must retain the previous snapshot.
    pub(super) fn prepare(
        &self, path: &str, previous: SnapshotImage, current: SnapshotImage,
        case_sensitive: bool,
    ) -> Option<PreparedDiff> {
        if !different(&previous, &current, case_sensitive) {
            return Some(PreparedDiff { pending: None, replacement: false });
        }
        // Only the tail may coalesce: this preserves commit order across
        // directories and preserves progress of a partly compared snapshot.
        let replacement = self.pending.back().is_some_and(|pending| {
            pending.path == path && !pending.started
        });
        let previous = self.pending.back().filter(|_| replacement)
            .map(|pending| pending.previous.clone())
            .unwrap_or(previous);
        if replacement && !different(&previous, &current, case_sensitive) {
            return Some(PreparedDiff { pending: None, replacement });
        }
        let pending = PendingDiff {
            path: path.to_string(), previous, current, case_sensitive,
            old_cursor: 0, new_cursor: 0, pending_created: None, started: false,
        };
        let replaced_bytes = self.pending.back().filter(|_| replacement)
            .map_or(0, PendingDiff::bytes);
        let budget = MAX_PENDING_BYTES;
        #[cfg(test)]
        let budget = self.test_byte_budget.unwrap_or(budget);
        if self.bytes.saturating_sub(replaced_bytes).saturating_add(pending.bytes())
            > budget
        {
            return None;
        }
        Some(PreparedDiff { pending: Some(pending), replacement })
    }

    /// Must be called under the same snapshot mutex as prepare, and only after
    /// successful revision/capacity admission. No I/O or external locks occur.
    pub(super) fn commit(&mut self, prepared: PreparedDiff) -> Option<PendingDiff> {
        let retired = if prepared.replacement { self.pending.pop_back() } else { None };
        if let Some(previous) = retired.as_ref() {
            self.bytes = self.bytes.saturating_sub(previous.bytes());
        }
        if let Some(pending) = prepared.pending {
            self.bytes += pending.bytes();
            self.pending.push_back(pending);
        }
        retired
    }

    pub(super) fn drain(&mut self, limit: usize) -> (Vec<MetadataChange>, Vec<PendingDiff>) {
        let limit = limit.min(MAX_DRAIN_RECORDS);
        let mut comparisons = MAX_DRAIN_COMPARISONS;
        let mut drained = Vec::new();
        let mut retired = Vec::new();
        while drained.len() < limit {
            let Some(pending) = self.pending.front_mut() else { break; };
            match pending.next(&mut comparisons) {
                Scan::Change(change) => drained.push(change),
                Scan::Pending => break,
                Scan::Finished => {
                    if let Some(finished) = self.pending.pop_front() {
                        self.bytes = self.bytes.saturating_sub(finished.bytes());
                        retired.push(finished);
                    }
                }
            }
        }
        (drained, retired)
    }
}

fn different(previous: &SnapshotImage, current: &SnapshotImage, case_sensitive: bool) -> bool {
    previous.entries.len() != current.entries.len() || previous.entries.iter().any(|old| {
        current.index.get(&identity_key(case_sensitive, &old.name))
            .and_then(|index| current.entries.get(*index)).map_or(true, |new| !same(old, new))
    })
}

pub(super) fn same(left: &VfsMeta, right: &VfsMeta) -> bool {
    left.name == right.name && left.is_dir == right.is_dir && left.is_symlink == right.is_symlink
        && left.size == right.size && left.mtime_ms == right.mtime_ms
        && left.id == right.id && left.content_md5 == right.content_md5
        && left.btime_ms == right.btime_ms && left.hidden == right.hidden
        && left.system == right.system
}
