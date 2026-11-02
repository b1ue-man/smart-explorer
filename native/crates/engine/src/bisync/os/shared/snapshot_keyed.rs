//! One remote snapshot side from a walk with folder keys (E10; agent and
//! service `walk-keys-v1`, Share `hash_walk_keys_v1`,
//! `agent_proto::folder_key`). The side forms the key of every folder where
//! the files are; the request names the keys this side and the other side
//! of the pair held after their last walks (`folder_key_cache.rs`), and the
//! host answers a folder with such a key by one mark instead of its subtree:
//! neither its listing nor its entries cross the connection. Its stored
//! items are replayed into the same check (`snapshot_host_stream::Stream`)
//! as sent ones, so the snapshot, its folders, filtered files, omissions and
//! early copies are exactly those of the walk without keys.
//!
//! Why equal keys mean equal input (E11, `docs/plaene/2026-10-31-sync-
//! abgleich-schnell/plan.md` E): a key is the SHA-256 of a canonical form of
//! every field the check takes from a folder's items plus the keys of its
//! subfolders (a Merkle tree); the check is a function of these fields
//! alone, since sent items are reduced to the same fields as stored ones
//! (`Part::item`). Unequal keys only mean "read it": the folder comes whole.
//!
//! After the walk the side's file is replaced by the keys of this walk: the
//! sent folders (each checked against the key the host named) and the
//! stored records of the folders answered by a mark. A file that cannot be
//! written only costs the next walk its marks.
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use super::folder_key_cache::{FolderCache, Part, Record};
use super::snapshot_dir::WalkContext;
use super::snapshot_host_stream::Stream;
use super::types::Tree;
use crate::agent_proto::FolderKey;
use crate::vfs::KeyedWalkItem;

/// The folder key files of one side of a pair.
#[derive(Clone, Debug)]
pub(super) struct KeyFiles {
    /// This side's file, replaced after a complete walk.
    pub(super) own: PathBuf,
    /// The other side's file, read only (its keys are offered as known).
    pub(super) peer: Option<PathBuf>,
}

impl KeyFiles {
    /// The files of `backend`/`root` with `peer` as the other side.
    pub(super) fn of(
        backend: &dyn crate::vfs::Backend,
        root: &str,
        peer: Option<(&dyn crate::vfs::Backend, &str)>,
        algorithm: Option<&str>,
    ) -> Self {
        let file = |backend: &dyn crate::vfs::Backend, root: &str| {
            super::folder_key_file::side_path(&backend.state_identity(), root, algorithm)
        };
        Self {
            own: file(backend, root),
            peer: peer.map(|(backend, root)| file(backend, root)),
        }
    }
}

/// What the walk with keys gave.
pub(super) struct KeyedTree {
    pub(super) tree: Tree,
    /// Folders answered by a mark (and replayed from the stored keys).
    pub(super) reused: u64,
    /// Folders the host sent.
    pub(super) sent: u64,
}

/// The tree of `ctx.root` from a walk with folder keys; `None` when the side
/// has none (nothing was recorded then).
pub(super) fn walk(
    ctx: &WalkContext<'_>,
    files: &KeyFiles,
    stream: impl FnOnce(
        &mut dyn FnMut(KeyedWalkItem) -> io::Result<()>,
        &[[u8; 16]],
    ) -> io::Result<bool>,
) -> io::Result<Option<KeyedTree>> {
    let own = super::folder_key_file::load(&files.own);
    let peer = files
        .peer
        .as_deref()
        .map(super::folder_key_file::load)
        .unwrap_or_default();
    let caches = [&own, &peer];
    let known = FolderCache::known(&caches);
    let mut driver = Driver {
        stream: Stream::new(ctx),
        caches: &caches,
        fresh: HashMap::new(),
        current: None,
        keys: HashMap::new(),
        same: Vec::new(),
        reused: 0,
        items: 0,
    };
    let ran = stream(
        &mut |item: KeyedWalkItem| driver.take(item),
        known.as_slice(),
    )?;
    if !ran {
        if driver.items > 0 {
            return Err(invalid("keyed hash walk refused after emitting entries"));
        }
        return Ok(None);
    }
    super::transfer_stream::check(ctx.cancel)?;
    let Driver {
        stream,
        fresh,
        keys,
        same,
        reused,
        ..
    } = driver;
    let sent = fresh.len() as u64;
    let tree = stream.finish_tree()?;
    match next_cache(fresh, &keys, &same, &caches) {
        Some(cache) => {
            if let Err(error) = super::folder_key_file::save(&files.own, &cache) {
                super::run_log::note(
                    "Scan",
                    &format!("Ordnerschlüssel nicht gespeichert: {error}"),
                );
            }
        }
        None => super::run_log::note(
            "Scan",
            "Ordnerschlüssel der Gegenseite unvollständig: nicht gespeichert",
        ),
    }
    Ok(Some(KeyedTree { tree, reused, sent }))
}

struct Driver<'s, 'c, 'a> {
    stream: Stream<'c, 'a>,
    caches: &'s [&'s FolderCache; 2],
    /// The runs the host sent, by folder.
    fresh: HashMap<String, Record>,
    current: Option<String>,
    /// The key of every folder (end and `same` marks).
    keys: HashMap<String, FolderKey>,
    same: Vec<FolderKey>,
    reused: u64,
    items: u64,
}

impl Driver<'_, '_, '_> {
    fn take(&mut self, item: KeyedWalkItem) -> io::Result<()> {
        self.items = self.items.saturating_add(1);
        match item {
            KeyedWalkItem::Run { rel } => {
                if self.fresh.insert(rel.clone(), Record::default()).is_some() {
                    return Err(invalid("keyed hash walk sent one folder twice"));
                }
                self.stream.run_mark(&rel)?;
                self.current = Some(rel);
                Ok(())
            }
            KeyedWalkItem::Item(item) => {
                let folder = self
                    .current
                    .as_deref()
                    .ok_or_else(|| invalid("keyed hash walk item outside a run"))?;
                let part = Part::of(folder, &item)?;
                let normalized = part.item(folder);
                if let Some(record) = self.fresh.get_mut(folder) {
                    record.parts.push(part);
                }
                self.stream.item(normalized)
            }
            KeyedWalkItem::Key { rel, key, same, .. } => {
                self.current = None;
                if self.keys.insert(rel.clone(), key).is_some() {
                    return Err(invalid("keyed hash walk named one folder key twice"));
                }
                if !same {
                    return Ok(());
                }
                self.same.push(key);
                let stream = &mut self.stream;
                let mut replayed = |item: KeyedWalkItem| match item {
                    KeyedWalkItem::Run { rel } => stream.run_mark(&rel),
                    KeyedWalkItem::Item(item) => stream.item(item),
                    KeyedWalkItem::Key { .. } => Ok(()),
                };
                self.reused += FolderCache::replay(self.caches, key, &rel, &mut replayed)?;
                Ok(())
            }
        }
    }
}

/// The keys of this walk, or `None` when one does not match its folder.
fn next_cache(
    fresh: HashMap<String, Record>,
    keys: &HashMap<String, FolderKey>,
    same: &[FolderKey],
    caches: &[&FolderCache; 2],
) -> Option<FolderCache> {
    let mut next = FolderCache::default();
    for (rel, mut record) in fresh {
        // A subfolder without a key was not walked (the host's filter left
        // it out); the key check below refuses a record that lost one.
        record.children = record.walked_children(&rel, keys);
        let key = *keys.get(&rel)?;
        if record.key() != key {
            return None;
        }
        next.insert(key, record);
    }
    for key in same {
        if !next.adopt(*key, caches) {
            return None;
        }
    }
    Some(next)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

#[cfg(all(test, unix))]
#[path = "snapshot_keyed_tests.rs"]
mod tests;
