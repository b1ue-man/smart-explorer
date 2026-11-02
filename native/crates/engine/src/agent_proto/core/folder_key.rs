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
//! - every subfolder that was walked: its name and its key. A folder the
//!   walk's filter leaves out (`walk_filter.rs`) is an entry of its parent's
//!   run and has no key: its parent's key holds no key for it, so a key of a
//!   walk that entered it never equals one of a walk that did not.
//!
//! Beside its key, a `Mode::KeysOnly` walk reports what each folder's subtree
//! holds (`KeyTally`: files, bytes, omissions, files of unknown time,
//! folder depth), so a caller that finds two folders equal by key accounts
//! for everything in them without listing either side.
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

use super::key_tally::{name_is_unusual, KeyTally};
use super::sha256::Sha256;

#[path = "folder_key_tree.rs"]
mod tree;
pub use tree::{KeyEmit, KeyTree, Mode};

/// The key of a folder.
pub type FolderKey = [u8; 32];
/// The prefix of a key a client sends as known (128 bits; the client looks up
/// the full key of every `same` mark and refuses one it does not hold).
pub type KnownKey = [u8; 16];
/// Known key prefixes one agent request may carry (4 MiB).
pub const MAX_KNOWN_KEYS: usize = 256 * 1024;
/// Runs held back while their folder may still turn out known.
pub const BUFFER_BYTES: usize = 32 * 1024 * 1024;

const DOMAIN_RUN: &[u8] = b"se-folder-run-v1\0";
const DOMAIN_KEY: &[u8] = b"se-folder-key-v1\0";

/// The kind of an entry, as the comparison derives it from the walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Link,
    Special,
}

impl EntryKind {
    /// The kind of an entry with these flags (a link wins over a special
    /// file, a special file over a folder).
    pub fn of(is_dir: bool, is_symlink: bool, special: bool) -> Self {
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
pub fn entry_part(
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
pub fn omission_part(name: &str, reason: u8) -> Vec<u8> {
    let mut part = Vec::with_capacity(name.len() + 6);
    put_str(&mut part, name);
    part.push(5);
    part.push(reason);
    part
}

/// The folder itself could not be read (to its end).
pub fn failure_part(reason: u8) -> Vec<u8> {
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
pub fn run_digest(mut parts: Vec<Vec<u8>>) -> [u8; 32] {
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
pub fn folder_key(run: &[u8; 32], mut children: Vec<(String, FolderKey)>) -> FolderKey {
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
pub fn known_prefix(key: &FolderKey) -> KnownKey {
    let mut prefix = [0u8; 16];
    prefix.copy_from_slice(&key[..16]);
    prefix
}

/// The known keys of a request: concatenated 16-byte prefixes.
pub fn parse_known(bytes: &[u8]) -> io::Result<HashSet<KnownKey>> {
    if !bytes.len().is_multiple_of(16) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "known folder keys are no whole 16-byte prefixes",
        ));
    }
    Ok(bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|chunk| {
            let mut prefix = [0u8; 16];
            prefix.copy_from_slice(chunk);
            prefix
        })
        .collect())
}

/// What an entry of a run is, for its key part and for the tally.
#[derive(Clone, Copy)]
pub struct Shape<'a> {
    pub name: &'a str,
    pub kind: EntryKind,
    pub size: u64,
    pub mtime_ms: i64,
}

/// What one thread collects for the folder it reads.
pub struct RunKeys<I> {
    pub rel: String,
    parts: Vec<Vec<u8>>,
    items: Vec<I>,
    bytes: usize,
    /// What the run itself holds (subfolders add theirs once complete).
    tally: KeyTally,
    /// Subfolder entries, and the children whose omission followed them.
    dirs: Vec<String>,
    unopened: HashSet<String>,
}

impl<I> RunKeys<I> {
    pub fn new(rel: &str) -> Self {
        Self {
            rel: rel.to_string(),
            parts: Vec::new(),
            items: Vec::new(),
            bytes: 0,
            tally: KeyTally::default(),
            dirs: Vec::new(),
            unopened: HashSet::new(),
        }
    }

    /// The name of `item_rel` in this folder, `None` for the folder itself.
    pub fn name_of<'r>(&self, item_rel: &'r str) -> io::Result<Option<&'r str>> {
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
    pub fn entry(&mut self, shape: Shape<'_>, part: Vec<u8>, item: I, cost: usize) {
        match shape.kind {
            EntryKind::File => {
                self.tally.files = self.tally.files.saturating_add(1);
                self.tally.bytes = self.tally.bytes.saturating_add(shape.size);
                if shape.mtime_ms == 0 {
                    self.tally.untimed = self.tally.untimed.saturating_add(1);
                }
            }
            EntryKind::Dir => self.dirs.push(shape.name.to_string()),
            EntryKind::Link | EntryKind::Special => {}
        }
        // A link, a special file and a name no transfer takes are reported by
        // the transfer's own walk: one omission each.
        if matches!(shape.kind, EntryKind::Link | EntryKind::Special) || name_is_unusual(shape.name)
        {
            self.omit_one();
        }
        self.push(part, item, cost);
    }

    fn omit_one(&mut self) {
        self.tally.omitted = self.tally.omitted.saturating_add(1);
    }

    /// An omission of the child `name`: a subfolder entry it follows is not
    /// walked (could not be opened, pseudo file system).
    pub fn omission(&mut self, name: &str, reason: u8, item: I, cost: usize) {
        self.unopened.insert(name.to_string());
        self.omit_one();
        self.push(omission_part(name, reason), item, cost);
    }

    /// The folder entry `name` (already added) is not walked: the walk's
    /// filter leaves its content out (`walk_filter.rs`). The run's part names
    /// it; no subfolder key follows for it.
    pub fn skip_dir(&mut self, name: &str) {
        self.unopened.insert(name.to_string());
    }

    /// The folder itself failed.
    pub fn failure(&mut self, reason: u8, item: I, cost: usize) {
        self.omit_one();
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
