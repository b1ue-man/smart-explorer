//! The folder keys of a local side in the pair's key phases
//! (`snapshot_pair_keys.rs`), formed by the side's one snapshot walk: every
//! folder it lists gives its run (`folder_key_cache::Part`s of the listing,
//! the digests the walk read for its own check), and once the walk is
//! complete the keys follow bottom-up. The tree is listed once and every
//! file is read at most once (W5 of the second review, G6 of the third).
//!
//! A key covers what the listing holds, so equal keys mean equal folders
//! for the comparison wherever they were formed. To be equal to the key the
//! walk next to the data forms on the other side (`agent_proto::walk_keyed`,
//! Share `hash_walk_keys_v1`), the run of a folder holds the parts that walk
//! forms: every listed entry with its kind, name, hidden flag, and size,
//! time and digest of a regular file; every listing omission; a subfolder
//! the walk entered by its key. A subfolder this walk does not enter (the
//! job's filter, a protected folder) has no key here. Where the other side's
//! walk does enter it (its filter, `WalkFilter::host_spec`, keeps it), its
//! subtree is read by that walk's keyed walk in this process
//! (`LocalKeyed::below`), so its key and those of the folders above it are
//! the other side's ones for equal content. Not read that way: pseudo file
//! systems (that walk leaves them out too) and this application's own
//! folders (`snapshot_policy::own_path`, the digest cache among them). In
//! the same way a file this walk's filter leaves out but the other side's
//! keeps is hashed here (the checksum compare mode), as that walk hashes it.
//!
//! A key formed from fewer or other parts than the other side's never
//! equals its key: the folder is then fetched, never taken as equal. The
//! snapshot itself comes from the walk alone, unchanged by any of this.
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

use super::folder_key_cache::{FolderCache, Part, Record};
use super::omissions::OmissionKind;
use super::snapshot_dir::{literal_child, Listed};
use super::snapshot_pair_keys::SideTree;
use crate::agent_proto::{EntryKind, FolderKey, HostFilter, KeyedWireItem, WalkFilterSpec};
use crate::vfs::{
    Backend, HashWalkEntry, HashWalkItem, HashWalkMeta, OmissionReason, VfsMeta, VfsOmission,
};

/// What the walk next to the data says of a pseudo file system it lists.
const PSEUDO_DETAIL: &str = "Pseudo-Dateisystem: Inhalt wird nicht durchlaufen";

/// The folder keys of one local side while its snapshot walk runs.
pub(super) struct LocalKeys {
    /// The root of the side (as the walk names it).
    root: String,
    /// Digests are part of the keys (the checksum compare mode).
    md5: bool,
    /// The filter the other side's walk applies; `None` without one or when
    /// it cannot be compiled (that walk then runs unfiltered too).
    filter: Option<HostFilter>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The parts of every folder the walk listed, by path below the root.
    folders: HashMap<String, Vec<Part>>,
    /// Digests the walk read, by path below the root, until their folder
    /// is recorded.
    digests: HashMap<String, String>,
    /// Keys and records of subtrees read by the other side's walk here.
    keys: HashMap<String, FolderKey>,
    records: FolderCache,
    /// The folders read by that walk (tests: the tree is listed once).
    #[cfg(test)]
    below: Vec<String>,
}

impl LocalKeys {
    /// The keys of the local side `root`; `spec` is the filter the other
    /// side's walk applies (`WalkFilter::host_spec`).
    pub(super) fn new(root: &str, md5: bool, spec: Option<Arc<WalkFilterSpec>>) -> Self {
        Self {
            root: root.to_string(),
            md5,
            filter: spec.and_then(|spec| spec.compile().ok()),
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The walk read the file `rel` (MD5, 32 lowercase hex digits).
    pub(super) fn digest(&self, rel: &str, hex: String) {
        if self.md5 {
            self.lock().digests.insert(rel.to_string(), hex);
        }
    }

    /// The parts of one listing, before the walk checks it (digests follow
    /// in `listed`).
    pub(super) fn parts(entries: &[VfsMeta], omitted: &[VfsOmission]) -> Vec<Part> {
        let mut parts = Vec::with_capacity(entries.len() + omitted.len());
        for entry in entries {
            let kind = EntryKind::of(entry.is_dir, entry.is_symlink, entry.special);
            let file = kind == EntryKind::File;
            parts.push(Part::Entry {
                name: entry.name.clone(),
                kind,
                size: if file { entry.size } else { 0 },
                mtime_ms: if file { entry.mtime_ms } else { 0 },
                digest: None,
                hidden: entry.hidden,
            });
        }
        for omission in omitted {
            parts.push(Part::Omitted {
                name: omission.rel.clone(),
                reason: omission.reason,
                detail: omission.detail.clone(),
            });
        }
        parts
    }

    /// The folder `dir` (`dir_rel` below the root) was listed and checked:
    /// `listed` names the subfolders the walk enters. A subfolder it does
    /// not enter but the other side's walk does is read here by that walk.
    pub(super) fn listed(
        &self,
        (be, dir, dir_rel): (&dyn Backend, &str, &str),
        mut parts: Vec<Part>,
        listed: &Listed,
        cancel: &AtomicBool,
    ) {
        let entered: HashSet<&str> = listed
            .dirs
            .iter()
            .map(|(_, rel)| rel.rsplit_once('/').map_or(rel.as_str(), |(_, name)| name))
            .collect();
        let mut below = Vec::new();
        let mut pseudo = Vec::new();
        let mut unread = Vec::new();
        {
            let mut state = self.lock();
            for (index, part) in parts.iter_mut().enumerate() {
                let Part::Entry {
                    name,
                    kind,
                    size,
                    mtime_ms,
                    digest,
                    hidden,
                } = part
                else {
                    continue;
                };
                let rel = literal_child(dir_rel, name.as_str());
                match kind {
                    EntryKind::File if self.md5 => {
                        *digest = state.digests.remove(&rel);
                        // Not read by the walk although the other side's
                        // walk hashes it (a pattern only this side applies,
                        // a failed read): it is hashed here.
                        let kept = self
                            .filter
                            .as_ref()
                            .is_none_or(|f| !f.leaves_out_file(&rel, *hidden, *size, *mtime_ms));
                        if digest.is_none() && kept {
                            unread.push((index, name.clone()));
                        }
                    }
                    EntryKind::Dir if !entered.contains(name.as_str()) => {
                        if self
                            .filter
                            .as_ref()
                            .is_some_and(|f| f.leaves_out_dir(&rel, *hidden))
                        {
                            continue;
                        }
                        let Ok(path) = crate::vfs::literal_child_path(dir, name.as_str()) else {
                            continue;
                        };
                        if crate::agent_proto::is_pseudo_dir(&path) {
                            pseudo.push(name.clone());
                        } else if !super::snapshot_policy::own_path(be, &path) {
                            below.push(rel);
                        }
                    }
                    _ => {}
                }
            }
        }
        for (index, name) in unread {
            if let Some(Part::Entry { digest, .. }) = parts.get_mut(index) {
                *digest = self.read_digest(be, dir, &name, cancel);
            }
        }
        parts.extend(pseudo.into_iter().map(|name| Part::Omitted {
            name,
            reason: OmissionReason::Unreadable,
            detail: PSEUDO_DETAIL.to_string(),
        }));
        for rel in below {
            self.read_below(&rel, cancel);
        }
        self.lock().folders.insert(dir_rel.to_string(), parts);
    }

    /// The walk entered `dir_rel` but could not list it: its run is that
    /// failure, as the other side's walk sends it. A failure of another
    /// kind leaves the folder without a key.
    pub(super) fn failed(&self, dir_rel: &str, kind: OmissionKind, detail: String) {
        let reason = match kind {
            OmissionKind::Link => OmissionReason::Link,
            OmissionKind::Special => OmissionReason::Special,
            OmissionKind::Unreadable => OmissionReason::Unreadable,
            OmissionKind::Vanished => OmissionReason::Vanished,
            _ => return,
        };
        let parts = vec![Part::Failed { reason, detail }];
        self.lock().folders.insert(dir_rel.to_string(), parts);
    }

    /// The MD5 of the file `name` in `dir` the walk did not read (its filter
    /// left it out); `None` when it cannot be read (the folder is fetched).
    fn read_digest(
        &self,
        be: &dyn Backend,
        dir: &str,
        name: &str,
        cancel: &AtomicBool,
    ) -> Option<String> {
        let path = crate::vfs::literal_child_path(dir, name).ok()?;
        let reads = crate::agent_proto::LocalReads {
            fresh: true,
            run: None,
        };
        super::snapshot_hash::hash_file_cached_hex(be, &path, cancel, reads).ok()
    }

    /// The subtree `rel` as the other side's walk reads it, here; left
    /// without keys when that walk fails (its folder is then fetched).
    fn read_below(&self, rel: &str, cancel: &AtomicBool) {
        #[cfg(test)]
        self.lock().below.push(rel.to_string());
        let algorithm = if self.md5 {
            crate::agent_proto::digest::MD5
        } else {
            crate::agent_proto::digest::NONE
        };
        let reads = crate::agent_proto::LocalReads {
            fresh: self.md5,
            run: None,
        };
        let root = (self.root.as_str(), rel);
        let scope = (reads, self.filter.as_ref());
        let Ok(collected) = crate::agent_proto::LocalKeyed::below(root, algorithm, scope, cancel)
        else {
            return;
        };
        let keys: HashMap<String, FolderKey> = collected.keys.into_iter().collect();
        let mut runs = HashMap::new();
        for (folder, items) in collected.runs {
            let parts = items
                .into_iter()
                .map(|item| Part::of(&folder, &from_wire(item)))
                .collect::<io::Result<Vec<Part>>>();
            let Ok(parts) = parts else {
                return;
            };
            runs.insert(folder, parts);
        }
        let records = super::snapshot_pair_side::records(runs, &keys);
        let mut state = self.lock();
        state.keys.extend(keys);
        state.records.absorb(records);
    }

    /// The folders read by the other side's walk here, in this run.
    #[cfg(test)]
    pub(super) fn read_below_folders(&self) -> Vec<String> {
        let mut below = self.lock().below.clone();
        below.sort();
        below
    }

    /// The keys and runs of the whole side, once its walk completed.
    pub(super) fn finish(self) -> SideTree {
        let State {
            folders,
            mut keys,
            mut records,
            ..
        } = self
            .state
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut order: Vec<(String, Vec<Part>)> = folders.into_iter().collect();
        // Deepest first: a folder's key needs those of its subfolders.
        order.sort_by_key(|(rel, _)| std::cmp::Reverse(depth(rel)));
        for (rel, parts) in order {
            let mut record = Record {
                parts,
                children: Vec::new(),
            };
            record.children = record.walked_children(&rel, &keys);
            let key = record.key();
            keys.insert(rel, key);
            records.insert(key, record);
        }
        SideTree {
            keys,
            records: Arc::new(records),
        }
    }
}

fn depth(rel: &str) -> usize {
    if rel.is_empty() {
        0
    } else {
        1 + rel.matches('/').count()
    }
}

fn from_wire(item: KeyedWireItem) -> HashWalkItem {
    match item {
        KeyedWireItem::Entry(entry) => HashWalkItem::Entry(HashWalkEntry {
            rel: entry.rel,
            is_dir: entry.is_dir,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            digest: entry.digest,
            meta: entry.meta.map(|meta| HashWalkMeta {
                hidden: meta.hidden,
                system: meta.system,
                btime_ms: meta.btime_ms,
                is_symlink: meta.is_symlink,
                special: meta.special,
            }),
        }),
        KeyedWireItem::Omitted(omission) => {
            HashWalkItem::Omitted(crate::agent::ext_wire::omission_from_wire(omission))
        }
    }
}

#[cfg(test)]
#[path = "abgleich_task_local_keys_tests.rs"]
mod tests;
