//! Inode numbers of the FUSE host (DRV-M4).
//!
//! The kernel names files by inode number; the engine names them by
//! callback path (`/a/b`, POSIX dialect). Every inode the kernel learns
//! through a lookup or a create counts its lookups; `forget` drops them and
//! a node without lookups disappears. Numbers grow from 2 (1 is the root)
//! and are never reused within one mount, so a late `forget` can never hit
//! a newer file. A rename moves the node and every descendant to the new
//! path; an unlinked or replaced name keeps its node (open handles still
//! use it) until the kernel forgets it, but no longer resolves by path.
//!
//! Directory listings report a known inode, or a stable pseudo number with
//! the high bit set for a name the kernel has not looked up yet: listings
//! do not count lookups, so they must not create nodes nobody forgets.
use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, MutexGuard};

pub(crate) const ROOT_INO: u64 = 1;
const ROOT_PATH: &str = "/";
/// Pseudo numbers of listed names never collide with counted inodes.
const PSEUDO_BIT: u64 = 1 << 63;

pub(crate) struct InodeTable {
    inner: Mutex<Inner>,
}

struct Inner {
    next: u64,
    nodes: HashMap<u64, Node>,
    by_path: HashMap<String, u64>,
}

struct Node {
    /// The linked callback path; `None` after unlink or a replacing rename.
    path: Option<String>,
    lookups: u64,
}

impl InodeTable {
    pub(crate) fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            ROOT_INO,
            Node {
                path: Some(ROOT_PATH.to_string()),
                lookups: 1,
            },
        );
        let mut by_path = HashMap::new();
        by_path.insert(ROOT_PATH.to_string(), ROOT_INO);
        Self {
            inner: Mutex::new(Inner {
                next: ROOT_INO + 1,
                nodes,
                by_path,
            }),
        }
    }

    /// The linked path of `ino`.
    pub(crate) fn path(&self, ino: u64) -> io::Result<String> {
        self.lock()?
            .nodes
            .get(&ino)
            .and_then(|node| node.path.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown or unlinked inode"))
    }

    /// The path of `name` in the directory `parent`.
    pub(crate) fn child_path(&self, parent: u64, name: &str) -> io::Result<String> {
        Ok(join(&self.path(parent)?, name))
    }

    /// The inode the kernel receives for `path` in a lookup or create reply;
    /// counts one lookup.
    pub(crate) fn remember(&self, path: &str) -> io::Result<u64> {
        let mut inner = self.lock()?;
        if let Some(ino) = inner.by_path.get(path).copied() {
            if let Some(node) = inner.nodes.get_mut(&ino) {
                node.lookups = node.lookups.saturating_add(1);
                return Ok(ino);
            }
        }
        let ino = inner.next;
        inner.next = ino
            .checked_add(1)
            .filter(|next| next & PSEUDO_BIT == 0)
            .ok_or_else(|| io::Error::other("inode numbers exhausted"))?;
        inner.nodes.insert(
            ino,
            Node {
                path: Some(path.to_string()),
                lookups: 1,
            },
        );
        inner.by_path.insert(path.to_string(), ino);
        Ok(ino)
    }

    /// The inode of a linked path, without counting a lookup.
    pub(crate) fn known(&self, path: &str) -> Option<u64> {
        self.lock().ok()?.by_path.get(path).copied()
    }

    /// The number a directory listing reports for `path`.
    pub(crate) fn listing_ino(&self, path: &str) -> u64 {
        self.known(path).unwrap_or_else(|| pseudo_ino(path))
    }

    pub(crate) fn forget(&self, ino: u64, count: u64) {
        if ino == ROOT_INO {
            return;
        }
        let Ok(mut inner) = self.lock() else {
            return;
        };
        let Some(node) = inner.nodes.get_mut(&ino) else {
            return;
        };
        node.lookups = node.lookups.saturating_sub(count);
        if node.lookups > 0 {
            return;
        }
        let path = node.path.take();
        inner.nodes.remove(&ino);
        if let Some(path) = path {
            if inner.by_path.get(&path) == Some(&ino) {
                inner.by_path.remove(&path);
            }
        }
    }

    /// The name `path` is gone; its node lives on for open handles.
    pub(crate) fn unlinked(&self, path: &str) {
        if let Ok(mut inner) = self.lock() {
            if let Some(ino) = inner.by_path.remove(path) {
                if let Some(node) = inner.nodes.get_mut(&ino) {
                    node.path = None;
                }
            }
        }
    }

    /// `from` now lives at `to`, with every descendant; a node that was
    /// linked at `to` (or below it) was replaced and is unlinked.
    pub(crate) fn renamed(&self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let Ok(mut inner) = self.lock() else {
            return;
        };
        let replaced: Vec<String> = inner
            .by_path
            .keys()
            .filter(|path| within(path, to))
            .cloned()
            .collect();
        for path in replaced {
            if let Some(ino) = inner.by_path.remove(&path) {
                if let Some(node) = inner.nodes.get_mut(&ino) {
                    node.path = None;
                }
            }
        }
        let moved: Vec<(String, u64)> = inner
            .by_path
            .iter()
            .filter(|(path, _)| within(path, from))
            .map(|(path, ino)| (path.clone(), *ino))
            .collect();
        for (old, ino) in moved {
            inner.by_path.remove(&old);
            let new = format!("{to}{}", &old[from.len()..]);
            if let Some(node) = inner.nodes.get_mut(&ino) {
                node.path = Some(new.clone());
            }
            inner.by_path.insert(new, ino);
        }
    }

    /// Counted inodes, the root included.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().map(|inner| inner.nodes.len()).unwrap_or(0)
    }

    fn lock(&self) -> io::Result<MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("inode table lock is poisoned"))
    }
}

/// `name` in the directory `parent` (both POSIX callback paths).
pub(crate) fn join(parent: &str, name: &str) -> String {
    if parent == ROOT_PATH {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// The parent and the last name of a callback path other than the root.
pub(crate) fn split(path: &str) -> Option<(&str, &str)> {
    let (parent, name) = path.rsplit_once('/')?;
    if name.is_empty() {
        return None;
    }
    Some((if parent.is_empty() { ROOT_PATH } else { parent }, name))
}

/// `path` is `root` or lies below it.
fn within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// FNV-1a of the path with the high bit set.
fn pseudo_ino(path: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    PSEUDO_BIT | (hash & !PSEUDO_BIT)
}

#[cfg(test)]
#[path = "fuse_inodes_tests.rs"]
mod tests;
