//! The folder keys a sync side observed (`agent_proto::folder_key`), stored
//! per side and addressed by content: a key names one folder's items and
//! the keys of its walked subfolders, so whatever is stored under a key is
//! valid for every folder with that key, at any path, on any side, at any
//! time. Nothing here can go stale; a changed folder simply has another key.
//!
//! A walk with keys (`snapshot_keyed.rs`) offers the stored keys of both
//! sides of the pair as known; the host answers a folder with a known key
//! by one mark, and the stored items of that key are replayed here as if
//! they had been sent. Only the fields the key covers are stored (and the
//! texts of omissions, for messages); the walk's other fields are normalized
//! the same way for sent and replayed items, so both reach the snapshot
//! identically.
//!
//! The file of a side (`folder-keys/<side>.sefk` in the sync data folder)
//! holds the keys of that side's last complete keyed walk. Every record is
//! checked against its key when it is loaded; a damaged record is left out.
//! Versions before this file ignore it; it never takes part in a baseline
//! (file format: `folder_key_file.rs`).
use std::collections::{HashMap, HashSet};
use std::io;

use crate::agent_proto::{entry_part, failure_part, folder_key, omission_part, run_digest};
use crate::agent_proto::{EntryKind, FolderKey, KnownKey};
use crate::vfs::{
    HashWalkEntry, HashWalkItem, HashWalkMeta, KeyedWalkItem, OmissionReason, VfsOmission,
};

/// One item of a folder's run, reduced to what its key covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Entry {
        name: String,
        kind: EntryKind,
        size: u64,
        mtime_ms: i64,
        digest: Option<String>,
        hidden: bool,
    },
    Omitted {
        name: String,
        reason: OmissionReason,
        detail: String,
    },
    Failed {
        reason: OmissionReason,
        detail: String,
    },
}

fn child(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

impl Part {
    /// The part of `item`, an item of the run of `folder`.
    pub(super) fn of(folder: &str, item: &HashWalkItem) -> io::Result<Part> {
        let name_of = |rel: &str| -> io::Result<Option<String>> {
            if rel == folder {
                return Ok(None);
            }
            let name = if folder.is_empty() {
                Some(rel)
            } else {
                rel.strip_prefix(folder)
                    .and_then(|rest| rest.strip_prefix('/'))
            };
            match name {
                Some(name) if !name.is_empty() && !name.contains('/') => Ok(Some(name.into())),
                _ => Err(invalid("keyed walk item outside its folder's run")),
            }
        };
        Ok(match item {
            HashWalkItem::Entry(entry) => {
                let meta = entry
                    .meta
                    .ok_or_else(|| invalid("keyed walk entry without its attributes"))?;
                let name = name_of(&entry.rel)?
                    .ok_or_else(|| invalid("keyed walk entry names its own folder"))?;
                let kind = EntryKind::of(entry.is_dir, meta.is_symlink, meta.special);
                let file = kind == EntryKind::File;
                Part::Entry {
                    name,
                    kind,
                    size: if file { entry.size } else { 0 },
                    mtime_ms: if file { entry.mtime_ms } else { 0 },
                    digest: if file { entry.digest.clone() } else { None },
                    hidden: meta.hidden,
                }
            }
            HashWalkItem::Omitted(omission) => match name_of(&omission.rel)? {
                Some(name) => Part::Omitted {
                    name,
                    reason: omission.reason,
                    detail: omission.detail.clone(),
                },
                None => Part::Failed {
                    reason: omission.reason,
                    detail: omission.detail.clone(),
                },
            },
        })
    }

    /// The item the snapshot gets for this part of the run of `folder`.
    pub(super) fn item(&self, folder: &str) -> HashWalkItem {
        match self {
            Part::Entry {
                name,
                kind,
                size,
                mtime_ms,
                digest,
                hidden,
            } => HashWalkItem::Entry(HashWalkEntry {
                rel: child(folder, name),
                is_dir: *kind == EntryKind::Dir,
                size: *size,
                mtime_ms: *mtime_ms,
                digest: digest.clone(),
                meta: Some(HashWalkMeta {
                    hidden: *hidden,
                    system: false,
                    btime_ms: 0,
                    is_symlink: *kind == EntryKind::Link,
                    special: *kind == EntryKind::Special,
                }),
            }),
            Part::Omitted {
                name,
                reason,
                detail,
            } => HashWalkItem::Omitted(VfsOmission {
                rel: child(folder, name),
                reason: *reason,
                detail: detail.clone(),
            }),
            Part::Failed { reason, detail } => HashWalkItem::Omitted(VfsOmission {
                rel: folder.to_string(),
                reason: *reason,
                detail: detail.clone(),
            }),
        }
    }

    fn key_part(&self) -> Vec<u8> {
        match self {
            Part::Entry {
                name,
                kind,
                size,
                mtime_ms,
                digest,
                hidden,
            } => entry_part(name, *kind, *size, *mtime_ms, digest.as_deref(), *hidden),
            Part::Omitted { name, reason, .. } => omission_part(name, reason.code()),
            Part::Failed { reason, .. } => failure_part(reason.code()),
        }
    }
}

/// One folder: its own items in the order they came and its walked
/// subfolders with their keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Record {
    pub(super) parts: Vec<Part>,
    pub(super) children: Vec<(String, FolderKey)>,
}

impl Record {
    pub(super) fn key(&self) -> FolderKey {
        let run = run_digest(self.parts.iter().map(Part::key_part).collect());
        folder_key(&run, self.children.clone())
    }

    /// The subfolders the walk may have entered: folder entries without an
    /// omission of the same name in this run. A folder the walk's filter
    /// left out is among them but has no key (`agent_proto::walk_filter`).
    pub(super) fn walked(&self) -> Vec<String> {
        let unopened: HashSet<&str> = self
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Omitted { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Entry {
                    name,
                    kind: EntryKind::Dir,
                    ..
                } if !unopened.contains(name.as_str()) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }
}

impl Record {
    /// The subfolders of this run of the folder `rel` that `keys` names a
    /// key for, with that key: the folders the walk entered. A folder its
    /// filter left out has no key; a missing key of an entered folder makes
    /// `key()` differ from the walk's key, so such a record is never taken.
    pub(super) fn walked_children(
        &self,
        rel: &str,
        keys: &HashMap<String, FolderKey>,
    ) -> Vec<(String, FolderKey)> {
        self.walked()
            .into_iter()
            .filter_map(|name| {
                let key = *keys.get(&child(rel, &name))?;
                Some((name, key))
            })
            .collect()
    }
}

/// The stored folders of one side, by key.
#[derive(Debug, Default)]
pub(super) struct FolderCache {
    records: HashMap<FolderKey, Record>,
}

impl FolderCache {
    pub(super) fn len(&self) -> usize {
        self.records.len()
    }

    pub(super) fn get(&self, key: &FolderKey) -> Option<&Record> {
        self.records.get(key)
    }

    pub(super) fn insert(&mut self, key: FolderKey, record: Record) {
        self.records.insert(key, record);
    }

    pub(super) fn records(&self) -> impl Iterator<Item = (&FolderKey, &Record)> {
        self.records.iter()
    }

    /// Takes every record of `other` in.
    pub(super) fn absorb(&mut self, other: FolderCache) {
        self.records.extend(other.records);
    }

    /// Copies `key` with everything below it from `from`; false when a
    /// record is missing there.
    pub(super) fn adopt(&mut self, key: FolderKey, from: &[&FolderCache]) -> bool {
        let mut pending = vec![key];
        while let Some(key) = pending.pop() {
            if self.records.contains_key(&key) {
                continue;
            }
            let Some(record) = from.iter().find_map(|cache| cache.get(&key)) else {
                return false;
            };
            pending.extend(record.children.iter().map(|(_, key)| *key));
            self.records.insert(key, record.clone());
        }
        true
    }

    /// Every stored key once, the keys covering the most folders first (a
    /// host takes as many as its request can carry).
    pub(super) fn known(caches: &[&FolderCache]) -> Vec<KnownKey> {
        let mut size: HashMap<FolderKey, u64> = HashMap::new();
        for cache in caches {
            for key in cache.records.keys() {
                subtree_size(*key, caches, &mut size);
            }
        }
        let mut keys: Vec<(u64, FolderKey)> = size
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(key, count)| (count, key))
            .collect();
        keys.sort_unstable_by(|left, right| right.cmp(left));
        let mut seen = HashSet::new();
        keys.into_iter()
            .map(|(_, key)| crate::agent_proto::known_prefix(&key))
            .filter(|prefix| seen.insert(*prefix))
            .collect()
    }

    /// Every key whose whole subtree `caches` hold.
    pub(super) fn complete_keys(caches: &[&FolderCache]) -> HashSet<FolderKey> {
        let mut size: HashMap<FolderKey, u64> = HashMap::new();
        for cache in caches {
            for key in cache.records.keys() {
                subtree_size(*key, caches, &mut size);
            }
        }
        size.into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(key, _)| key)
            .collect()
    }

    /// Whether `caches` hold the whole subtree of `key`.
    pub(super) fn holds(caches: &[&FolderCache], key: FolderKey) -> bool {
        subtree_size(key, caches, &mut HashMap::new()) > 0
    }

    /// The runs of `key` and everything below it, at `rel`, top down: a
    /// run mark for every folder, then each of its items. Returns the number
    /// of folders.
    pub(super) fn replay(
        caches: &[&FolderCache],
        key: FolderKey,
        rel: &str,
        take: &mut dyn FnMut(KeyedWalkItem) -> io::Result<()>,
    ) -> io::Result<u64> {
        let mut pending = vec![(key, rel.to_string())];
        let mut folders = 0u64;
        while let Some((key, rel)) = pending.pop() {
            let record = caches
                .iter()
                .find_map(|cache| cache.get(&key))
                .ok_or_else(|| invalid("host named a folder key this side does not hold"))?;
            folders += 1;
            take(KeyedWalkItem::Run { rel: rel.clone() })?;
            for part in &record.parts {
                take(KeyedWalkItem::Item(part.item(&rel)))?;
            }
            for (name, key) in record.children.iter().rev() {
                pending.push((*key, child(&rel, name)));
            }
        }
        Ok(folders)
    }
}

/// The folders below and including `key`; 0 when a record of that subtree
/// is missing (such a key is never offered: its replay would fail).
fn subtree_size(
    key: FolderKey,
    caches: &[&FolderCache],
    size: &mut HashMap<FolderKey, u64>,
) -> u64 {
    // Iterative post-order: children first, then the folder.
    let mut stack = vec![(key, false)];
    while let Some((key, expanded)) = stack.pop() {
        if size.contains_key(&key) {
            continue;
        }
        let Some(record) = caches.iter().find_map(|cache| cache.get(&key)) else {
            size.insert(key, 0);
            continue;
        };
        if expanded {
            let mut below = 1u64;
            for (_, child) in &record.children {
                match size.get(child).copied().unwrap_or(0) {
                    0 => {
                        below = 0;
                        break;
                    }
                    count => below = below.saturating_add(count),
                }
            }
            size.insert(key, below);
        } else {
            stack.push((key, true));
            for (_, child) in &record.children {
                if !size.contains_key(child) {
                    stack.push((*child, false));
                }
            }
        }
    }
    size.get(&key).copied().unwrap_or(0)
}
