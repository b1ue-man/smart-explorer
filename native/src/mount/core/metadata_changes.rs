use super::{identity_key, support::join};
use crate::vfs::VfsMeta;
use std::collections::{HashMap, VecDeque};
use std::mem::size_of;
use std::sync::Arc;

const MAX_PENDING_DIFFS: usize = 64;
const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
const MAX_DRAIN_RECORDS: usize = 1_024;

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

struct PendingDiff {
    path: String,
    previous: SnapshotImage,
    current: SnapshotImage,
    case_sensitive: bool,
    old_cursor: usize,
    new_cursor: usize,
    pending_created: Option<usize>,
    started: bool,
}

impl PendingDiff {
    fn bytes(&self) -> usize {
        self.previous.bytes.saturating_add(self.current.bytes)
            .saturating_add(self.path.len()).saturating_add(size_of::<Self>())
    }

    fn next(&mut self) -> Option<MetadataChange> {
        if let Some(index) = self.pending_created.take() {
            let new = &self.current.entries[index];
            return Some(MetadataChange::Created {
                path: join(&self.path, &new.name), is_directory: new.is_dir,
            });
        }
        while let Some(old) = self.previous.entries.get(self.old_cursor) {
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
            if change.is_some() {
                self.started = true;
                return change;
            }
        }
        while let Some(new) = self.current.entries.get(self.new_cursor) {
            self.new_cursor += 1;
            if !self.previous.index.contains_key(&identity_key(self.case_sensitive, &new.name)) {
                self.started = true;
                return Some(MetadataChange::Created {
                    path: join(&self.path, &new.name), is_directory: new.is_dir,
                });
            }
        }
        None
    }
}

pub(super) struct PreparedDiff {
    pending: Option<PendingDiff>,
    replacement: Option<usize>,
}

#[derive(Default)]
pub(super) struct ChangeQueue {
    pending: VecDeque<PendingDiff>,
    bytes: usize,
}

impl ChangeQueue {
    /// Pure admission, called under the snapshot mutex before any cache change.
    /// None means backpressure: the caller must retain the previous snapshot.
    pub(super) fn prepare(
        &self, path: &str, previous: SnapshotImage, current: SnapshotImage,
        case_sensitive: bool,
    ) -> Option<PreparedDiff> {
        if !different(&previous, &current, case_sensitive) {
            return Some(PreparedDiff { pending: None, replacement: None });
        }
        // Only the tail may coalesce: this preserves commit order across
        // directories as well as every already partly delivered snapshot.
        let replacement = self.pending.back().filter(|pending| {
            pending.path == path && !pending.started
        }).map(|_| self.pending.len() - 1);
        let previous = replacement.map(|index| self.pending[index].previous.clone())
            .unwrap_or(previous);
        if !different(&previous, &current, case_sensitive) {
            return Some(PreparedDiff { pending: None, replacement });
        }
        let pending = PendingDiff {
            path: path.to_string(), previous, current, case_sensitive,
            old_cursor: 0, new_cursor: 0, pending_created: None, started: false,
        };
        let replaced_bytes = replacement.map_or(0, |index| self.pending[index].bytes());
        if (replacement.is_none() && self.pending.len() >= MAX_PENDING_DIFFS)
            || self.bytes.saturating_sub(replaced_bytes).saturating_add(pending.bytes())
                > MAX_PENDING_BYTES
        {
            return None;
        }
        Some(PreparedDiff { pending: Some(pending), replacement })
    }

    /// Must be called under the same snapshot mutex as prepare, and only after
    /// successful revision/capacity admission. No I/O or external locks occur.
    pub(super) fn commit(&mut self, prepared: PreparedDiff) {
        if let Some(index) = prepared.replacement {
            if let Some(previous) = self.pending.remove(index) {
                self.bytes = self.bytes.saturating_sub(previous.bytes());
            }
        }
        if let Some(pending) = prepared.pending {
            self.bytes += pending.bytes();
            self.pending.push_back(pending);
        }
    }

    pub(super) fn drain(&mut self, limit: usize) -> Vec<MetadataChange> {
        let limit = limit.min(MAX_DRAIN_RECORDS);
        let mut drained = Vec::new();
        while drained.len() < limit {
            let Some(pending) = self.pending.front_mut() else { break; };
            if let Some(change) = pending.next() {
                drained.push(change);
            } else if let Some(finished) = self.pending.pop_front() {
                self.bytes = self.bytes.saturating_sub(finished.bytes());
            }
        }
        drained
    }
}

fn different(previous: &SnapshotImage, current: &SnapshotImage, case_sensitive: bool) -> bool {
    previous.entries.len() != current.entries.len() || previous.entries.iter().any(|old| {
        current.index.get(&identity_key(case_sensitive, &old.name))
            .and_then(|index| current.entries.get(*index)).map_or(true, |new| !same(old, new))
    })
}

fn same(left: &VfsMeta, right: &VfsMeta) -> bool {
    left.name == right.name && left.is_dir == right.is_dir && left.is_symlink == right.is_symlink
        && left.size == right.size && left.mtime_ms == right.mtime_ms
        && left.id == right.id && left.content_md5 == right.content_md5
        && left.btime_ms == right.btime_ms && left.hidden == right.hidden
        && left.system == right.system
}
