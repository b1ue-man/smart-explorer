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
use std::collections::HashSet;
use std::io;

use super::sha256::Sha256;

#[path = "folder_key_tree.rs"]
mod tree;
pub(crate) use tree::{KeyEmit, KeyTree, Mode};

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

#[cfg(test)]
#[path = "folder_key_tests.rs"]
mod tests;
