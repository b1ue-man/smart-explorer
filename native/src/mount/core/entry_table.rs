//! Attached file identities and their exact-parent overlay index.
use super::engine::{parent_path, Entry};
use std::collections::HashMap;
use std::sync::Arc;

/// Keys are already normalized with the engine's case semantics. Both views
/// change under the engine's one table mutex; mutable access cannot bypass the
/// parent index. Index operations never acquire an Entry's state mutex.
#[derive(Default)]
pub(super) struct EntryTable {
    paths: HashMap<String, IndexedEntry>,
    parents: HashMap<String, Vec<String>>,
}

struct IndexedEntry {
    entry: Arc<Entry>,
    parent_slot: usize,
}

impl EntryTable {
    pub fn new() -> Self { Self::default() }

    pub fn get(&self, key: &str) -> Option<&Arc<Entry>> {
        self.paths.get(key).map(|indexed| &indexed.entry)
    }

    #[cfg(test)]
    pub fn contains_key(&self, key: &str) -> bool { self.paths.contains_key(key) }

    pub fn insert(&mut self, key: String, entry: Arc<Entry>) -> Option<Arc<Entry>> {
        if let Some(indexed) = self.paths.get_mut(&key) {
            // Replacing one identity does not add another child or change its
            // slot. The key alone determines the already-indexed parent.
            return Some(std::mem::replace(&mut indexed.entry, entry));
        }
        let children = self.parents.entry(parent_path(&key).to_string()).or_default();
        let parent_slot = children.len();
        children.push(key.clone());
        self.paths.insert(key, IndexedEntry { entry, parent_slot });
        None
    }

    pub fn remove(&mut self, key: &str) -> Option<Arc<Entry>> {
        let removed = self.paths.remove(key)?;
        let parent = parent_path(key);
        if let Some(children) = self.parents.get_mut(parent) {
            // Every live path owns exactly one valid slot. Swap removal keeps
            // traversal dense, even after most siblings have been closed.
            children.swap_remove(removed.parent_slot);
            if let Some(moved_key) = children.get(removed.parent_slot) {
                if let Some(moved) = self.paths.get_mut(moved_key) {
                    moved.parent_slot = removed.parent_slot;
                }
            }
            if children.is_empty() { self.parents.remove(parent); }
        }
        Some(removed.entry)
    }

    pub fn extend(&mut self, entries: impl IntoIterator<Item = (String, Arc<Entry>)>) {
        for (key, entry) in entries { self.insert(key, entry); }
    }

    pub fn children(&self, parent: &str) -> impl Iterator<Item = &Arc<Entry>> {
        // Inspect exactly the dense vector's live keys, never the capacity of
        // a sparsely populated sibling map. Index mutation is private above.
        self.parents.get(parent).into_iter().flatten().filter_map(|key| self.get(key))
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<Entry>> {
        self.paths.values().map(|indexed| &indexed.entry)
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool { self.paths.is_empty() }
}
