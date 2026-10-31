//! Folder keys (`walk-keys-v1`, Share `hash_walk_keys_v1`): the SHA-256 of a
//! canonical form of everything a sync comparison takes from one folder of a
//! walk next to the data, its subfolders' keys included (a Merkle tree, as
//! Git tree objects name a folder by the hash of its entries and their
//! hashes). Equal keys mean equal folders for the comparison; unequal keys
//! only mean "look inside".
//!
//! What a key covers per item of a folder's run (the fields
//! `bisync::snapshot_host_stream` passes on to the snapshot, nothing else):
//! - regular file: name, size, modification time (ms), digest, hidden;
//! - folder, link, special file: kind, name, hidden;
//! - omission of a child: name and reason code (`ops_types::omission`);
//! - the folder's own failure (could not be read to its end): reason code;
//! - every subfolder that was walked: its name and its key.
//!
//! Folder sizes and times, creation times, the system flag and omission texts
//! are left out: the comparison does not use them, and they differ between
//! two copies of the same tree. Items are sorted by their encoding, so the
//! listing order of a file system does not matter. Names are compared
//! exactly (no case folding): equal keys then mean equal spellings too.
//!
//! `KeyTree` follows the folders of one parallel walk (`ordered_walk`): a
//! folder's run ends before any folder below it starts, a folder is complete
//! when its run ended and all its walked subfolders are complete. With known
//! keys (from the client's earlier walks) a complete folder whose key the
//! client knows is not sent: one `same` mark replaces its subtree. Every
//! other folder's run goes out with an end mark carrying its key once its
//! subtree is complete. Runs wait in memory only while a known key is
//! possible and within `BUFFER_BYTES`; beyond that the oldest waiting chain
//! is sent (only less saving, never another result).
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::{Mutex, MutexGuard};

use super::sha256::Sha256;

/// The key of a folder.
pub(crate) type FolderKey = [u8; 32];
/// The prefix of a key a client sends as known (128 bits; the client looks up
/// the full key of every `same` mark and refuses one it does not hold).
pub(crate) type KnownKey = [u8; 16];
/// Known key prefixes one agent request may carry (4 MiB).
pub(crate) const MAX_KNOWN_KEYS: usize = 256 * 1024;
/// Runs held back while their folder may still turn out known.
pub(crate) const BUFFER_BYTES: usize = 32 * 1024 * 1024;

const DOMAIN_RUN: &[u8] = b"se-folder-run-v1\0";
const DOMAIN_KEY: &[u8] = b"se-folder-key-v1\0";

/// The kind of an entry, as the comparison derives it from the walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryKind {
    File,
    Dir,
    Link,
    Special,
}

impl EntryKind {
    /// The kind of an entry with these flags (a link wins over a special
    /// file, a special file over a folder).
    pub(crate) fn of(is_dir: bool, is_symlink: bool, special: bool) -> Self {
        if is_symlink {
            EntryKind::Link
        } else if special {
            EntryKind::Special
        } else if is_dir {
            EntryKind::Dir
        } else {
            EntryKind::File
        }
    }
}

/// One item of a folder's run in its canonical encoding.
pub(crate) fn entry_part(
    name: &str,
    kind: EntryKind,
    size: u64,
    mtime_ms: i64,
    digest: Option<&str>,
    hidden: bool,
) -> Vec<u8> {
    let mut part = Vec::with_capacity(name.len() + 40);
    put_str(&mut part, name);
    match kind {
        EntryKind::File => {
            part.push(1);
            part.extend_from_slice(&size.to_be_bytes());
            part.extend_from_slice(&mtime_ms.to_be_bytes());
            match digest {
                Some(digest) => {
                    part.push(1);
                    put_str(&mut part, digest);
                }
                None => part.push(0),
            }
        }
        EntryKind::Dir => part.push(2),
        EntryKind::Link => part.push(3),
        EntryKind::Special => part.push(4),
    }
    part.push(u8::from(hidden));
    part
}

/// An omission of the child `name` (reason code of `ops_types::omission`).
pub(crate) fn omission_part(name: &str, reason: u8) -> Vec<u8> {
    let mut part = Vec::with_capacity(name.len() + 6);
    put_str(&mut part, name);
    part.push(5);
    part.push(reason);
    part
}

/// The folder itself could not be read (to its end).
pub(crate) fn failure_part(reason: u8) -> Vec<u8> {
    // An empty name sorts first and no child has one.
    let mut part = Vec::with_capacity(6);
    put_str(&mut part, "");
    part.push(6);
    part.push(reason);
    part
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// The digest of a folder's own items (`parts` in any order).
pub(crate) fn run_digest(mut parts: Vec<Vec<u8>>) -> [u8; 32] {
    parts.sort_unstable();
    let mut sha = Sha256::new();
    sha.update(DOMAIN_RUN);
    sha.update(&(parts.len() as u64).to_be_bytes());
    for part in &parts {
        sha.update(&(part.len() as u32).to_be_bytes());
        sha.update(part);
    }
    sha.finish()
}

/// The key of a folder from its run digest and its walked subfolders.
pub(crate) fn folder_key(run: &[u8; 32], mut children: Vec<(String, FolderKey)>) -> FolderKey {
    children.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut sha = Sha256::new();
    sha.update(DOMAIN_KEY);
    sha.update(run);
    sha.update(&(children.len() as u64).to_be_bytes());
    for (name, key) in &children {
        let mut name_part = Vec::with_capacity(name.len() + 4);
        put_str(&mut name_part, name);
        sha.update(&name_part);
        sha.update(key);
    }
    sha.finish()
}

/// The known prefix of `key`.
pub(crate) fn known_prefix(key: &FolderKey) -> KnownKey {
    let mut prefix = [0u8; 16];
    prefix.copy_from_slice(&key[..16]);
    prefix
}

/// The known keys of a request: concatenated 16-byte prefixes.
pub(crate) fn parse_known(bytes: &[u8]) -> io::Result<HashSet<KnownKey>> {
    if bytes.len() % 16 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "known folder keys are no whole 16-byte prefixes",
        ));
    }
    Ok(bytes
        .chunks_exact(16)
        .map(|chunk| {
            let mut prefix = [0u8; 16];
            prefix.copy_from_slice(chunk);
            prefix
        })
        .collect())
}

/// What one thread collects for the folder it reads.
pub(crate) struct RunKeys<I> {
    pub(crate) rel: String,
    parts: Vec<Vec<u8>>,
    items: Vec<I>,
    bytes: usize,
    /// Subfolder entries, and the children whose omission followed them.
    dirs: Vec<String>,
    unopened: HashSet<String>,
}

impl<I> RunKeys<I> {
    pub(crate) fn new(rel: &str) -> Self {
        Self {
            rel: rel.to_string(),
            parts: Vec::new(),
            items: Vec::new(),
            bytes: 0,
            dirs: Vec::new(),
            unopened: HashSet::new(),
        }
    }

    /// The name of `item_rel` in this folder, `None` for the folder itself.
    pub(crate) fn name_of<'r>(&self, item_rel: &'r str) -> io::Result<Option<&'r str>> {
        if item_rel == self.rel {
            return Ok(None);
        }
        let name = if self.rel.is_empty() {
            Some(item_rel)
        } else {
            item_rel
                .strip_prefix(self.rel.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
        };
        match name {
            Some(name) if !name.is_empty() && !name.contains('/') => Ok(Some(name)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "walk item outside its folder's run",
            )),
        }
    }

    /// An entry of this folder (`part` from `entry_part`).
    pub(crate) fn entry(
        &mut self,
        name: &str,
        kind: EntryKind,
        part: Vec<u8>,
        item: I,
        cost: usize,
    ) {
        if kind == EntryKind::Dir {
            self.dirs.push(name.to_string());
        }
        self.push(part, item, cost);
    }

    /// An omission of the child `name`: a subfolder entry it follows is not
    /// walked (could not be opened, pseudo file system).
    pub(crate) fn omission(&mut self, name: &str, reason: u8, item: I, cost: usize) {
        self.unopened.insert(name.to_string());
        self.push(omission_part(name, reason), item, cost);
    }

    /// The folder itself failed.
    pub(crate) fn failure(&mut self, reason: u8, item: I, cost: usize) {
        self.push(failure_part(reason), item, cost);
    }

    fn push(&mut self, part: Vec<u8>, item: I, cost: usize) {
        self.parts.push(part);
        self.items.push(item);
        self.bytes = self.bytes.saturating_add(cost);
    }
}

/// Where a `KeyTree` sends what is decided.
pub(crate) trait KeyEmit<I> {
    /// The complete run of the folder `rel`, in the order it was produced
    /// (sent after a run mark naming `rel`, so the client knows where every
    /// run begins, an empty one included).
    fn run(&mut self, rel: &str, items: Vec<I>) -> io::Result<()>;
    /// `same = false`: the end mark of a folder sent with everything below
    /// it; `same = true`: the folder's subtree is the one the client knows
    /// under `key` and is not sent.
    fn key(&mut self, rel: &str, key: &FolderKey, same: bool) -> io::Result<()>;
}

struct Node<I> {
    pending: usize,
    run: [u8; 32],
    children: Vec<(String, FolderKey)>,
    items: Option<Vec<I>>,
    bytes: usize,
    same_children: Vec<(String, FolderKey)>,
}

struct State<I> {
    nodes: HashMap<String, Node<I>>,
    /// Folders whose run was held back, oldest first (spill order); an
    /// entry whose run went out since is skipped when it comes up.
    held: VecDeque<String>,
    buffered: usize,
    root_done: bool,
}

/// The folders of one keyed walk (see the module head).
pub(crate) struct KeyTree<I> {
    root: String,
    known: HashSet<KnownKey>,
    state: Mutex<State<I>>,
}

impl<I> KeyTree<I> {
    /// A walk whose first folder is `root` (its rel, usually empty).
    pub(crate) fn new(root: &str, known: HashSet<KnownKey>) -> Self {
        Self {
            root: root.to_string(),
            known,
            state: Mutex::new(State {
                nodes: HashMap::new(),
                held: VecDeque::new(),
                buffered: 0,
                root_done: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<I>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether the root folder and everything below it is decided.
    pub(crate) fn complete(&self) -> bool {
        self.lock().root_done
    }

    fn parent_of<'r>(&self, rel: &'r str) -> Option<&'r str> {
        if rel == self.root {
            return None;
        }
        Some(rel.rsplit_once('/').map_or("", |(parent, _)| parent))
    }

    /// One folder's run ended (`ordered_walk` calls this before any folder
    /// below it starts). Emission happens under the tree's lock, so runs and
    /// marks reach the stream in the order decided here.
    pub(crate) fn finish_run(&self, run: RunKeys<I>, emit: &mut dyn KeyEmit<I>) -> io::Result<()> {
        let RunKeys {
            rel,
            parts,
            items,
            bytes,
            dirs,
            unopened,
        } = run;
        let walked = dirs
            .into_iter()
            .filter(|name| !unopened.contains(name))
            .count();
        let node = Node {
            pending: walked,
            run: run_digest(parts),
            children: Vec::new(),
            items: Some(items),
            bytes,
            same_children: Vec::new(),
        };
        let mut state = self.lock();
        if state.nodes.insert(rel.clone(), node).is_some() {
            return Err(invalid("walk read one folder twice"));
        }
        if self.known.is_empty() {
            self.emit_chain(&mut state, &rel, emit)?;
        } else {
            state.buffered = state.buffered.saturating_add(bytes);
            state.held.push_back(rel.clone());
            while state.buffered > BUFFER_BYTES {
                let Some(oldest) = state.held.pop_front() else {
                    break;
                };
                self.emit_chain(&mut state, &oldest, emit)?;
            }
        }
        if walked == 0 {
            self.completed(&mut state, rel, emit)?;
        }
        Ok(())
    }

    /// Sends the runs of `rel` and of every ancestor not sent yet, top down,
    /// each followed by the `same` marks of its known children.
    fn emit_chain(
        &self,
        state: &mut State<I>,
        rel: &str,
        emit: &mut dyn KeyEmit<I>,
    ) -> io::Result<()> {
        let mut chain = Vec::new();
        let mut current = Some(rel.to_string());
        while let Some(folder) = current {
            match state.nodes.get(&folder) {
                Some(node) if node.items.is_some() => {}
                _ => break,
            }
            current = self.parent_of(&folder).map(str::to_string);
            chain.push(folder);
        }
        for folder in chain.into_iter().rev() {
            let Some(node) = state.nodes.get_mut(&folder) else {
                continue;
            };
            let Some(items) = node.items.take() else {
                continue;
            };
            let bytes = node.bytes;
            let same = std::mem::take(&mut node.same_children);
            state.buffered = state.buffered.saturating_sub(bytes);
            emit.run(&folder, items)?;
            for (name, key) in same {
                emit.key(&child_rel(&folder, &name), &key, true)?;
            }
        }
        Ok(())
    }

    /// `rel` and everything below it are read: its key is final.
    fn completed(
        &self,
        state: &mut State<I>,
        rel: String,
        emit: &mut dyn KeyEmit<I>,
    ) -> io::Result<()> {
        let mut rel = rel;
        loop {
            let node = state
                .nodes
                .get_mut(&rel)
                .ok_or_else(|| invalid("walk completed an unknown folder"))?;
            let key = folder_key(&node.run, std::mem::take(&mut node.children));
            let unsent = node.items.is_some();
            let same = unsent && self.known.contains(&known_prefix(&key));
            let parent = self.parent_of(&rel).map(str::to_string);
            if same {
                let node = state
                    .nodes
                    .remove(&rel)
                    .ok_or_else(|| invalid("walk lost a folder"))?;
                state.buffered = state.buffered.saturating_sub(node.bytes);
                let parent_waits = parent
                    .as_ref()
                    .and_then(|parent| state.nodes.get(parent))
                    .is_some_and(|parent| parent.items.is_some());
                if parent_waits {
                    let name = name_in(&rel).to_string();
                    if let Some(parent) = parent.as_ref().and_then(|p| state.nodes.get_mut(p)) {
                        parent.same_children.push((name, key));
                    }
                } else {
                    emit.key(&rel, &key, true)?;
                }
            } else {
                self.emit_chain(state, &rel, emit)?;
                emit.key(&rel, &key, false)?;
                state.nodes.remove(&rel);
            }
            let Some(parent) = parent else {
                state.root_done = true;
                return Ok(());
            };
            let node = state
                .nodes
                .get_mut(&parent)
                .ok_or_else(|| invalid("walk completed a folder before its parent"))?;
            node.children.push((name_in(&rel).to_string(), key));
            node.pending = node
                .pending
                .checked_sub(1)
                .ok_or_else(|| invalid("walk completed more subfolders than it listed"))?;
            if node.pending > 0 {
                return Ok(());
            }
            rel = parent;
        }
    }
}

fn child_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn name_in(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

#[cfg(test)]
#[path = "folder_key_tests.rs"]
mod tests;
