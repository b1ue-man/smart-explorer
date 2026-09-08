//! Small revision-owned records charged to their retained parent snapshot.
use super::{refresh_order::KeyQueue, CachedDirectory};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct ChildKey {
    pub(super) parent: Arc<str>,
    pub(super) revision: u64,
    pub(super) index: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum ChildState { Ready, Selected, Covered, Cooling(Instant) }

struct Parent {
    key: Arc<str>,
    revision: u64,
    depth: u8,
    cursor: usize,
    children: HashMap<usize, ChildState>,
}

#[derive(Default)]
pub(super) struct PreloadRecords {
    parents: HashMap<String, Parent>,
    cursors: BTreeMap<u8, KeyQueue<Arc<str>>>,
    ready: BTreeMap<u8, KeyQueue<ChildKey>>,
    retries: BTreeSet<(Instant, ChildKey)>,
}

fn remove_from<K: Clone + Eq + std::hash::Hash>(
    queues: &mut BTreeMap<u8, KeyQueue<K>>, depth: u8, key: &K,
) {
    if let Some(queue) = queues.get_mut(&depth) {
        queue.remove(key);
        if queue.is_empty() { queues.remove(&depth); }
    }
}

impl PreloadRecords {
    pub(super) fn insert(&mut self, key: &str, cached: &CachedDirectory) {
        self.remove(key);
        let parent = Parent { key: Arc::from(key), revision: cached.revision,
            depth: cached.depth.saturating_add(1), cursor: 0, children: HashMap::new() };
        if !cached.entries.is_empty() {
            self.cursors.entry(parent.depth).or_default().push_back(Arc::clone(&parent.key));
        }
        self.parents.insert(key.to_string(), parent);
    }

    pub(super) fn remove(&mut self, key: &str) {
        let Some(parent) = self.parents.remove(key) else { return; };
        remove_from(&mut self.cursors, parent.depth, &parent.key);
        for (index, status) in parent.children {
            let child = ChildKey { parent: Arc::clone(&parent.key), revision: parent.revision, index };
            if status == ChildState::Ready {
                remove_from(&mut self.ready, parent.depth, &child);
            } else if let ChildState::Cooling(deadline) = status {
                self.retries.remove(&(deadline, child));
            }
        }
        if self.parents.capacity() > 32 && self.parents.capacity() / 4 > self.parents.len() {
            self.parents.shrink_to(self.parents.len().saturating_mul(2));
        }
    }

    pub(super) fn cursor(&self, maximum_depth: u8) -> Option<(Arc<str>, usize, u8)> {
        let (depth, queue) = self.cursors.first_key_value()?;
        if *depth >= maximum_depth { return None; }
        let key = queue.first()?;
        let parent = self.parents.get(key.as_ref())?;
        Some((key, parent.cursor, *depth))
    }

    pub(super) fn advance(&mut self, key: &str, next: usize, finished: bool) {
        let Some(parent) = self.parents.get_mut(key) else { return; };
        parent.cursor = next;
        remove_from(&mut self.cursors, parent.depth, &parent.key);
        if !finished {
            self.cursors.entry(parent.depth).or_default().push_back(Arc::clone(&parent.key));
        }
    }

    pub(super) fn discover(&mut self, key: &str, index: usize, eligible: bool) {
        let Some(parent) = self.parents.get_mut(key) else { return; };
        if parent.children.contains_key(&index) { return; }
        parent.children.insert(index, if eligible { ChildState::Ready } else { ChildState::Covered });
        if eligible {
            self.ready.entry(parent.depth).or_default().push_back(ChildKey {
                parent: Arc::clone(&parent.key), revision: parent.revision, index,
            });
        }
    }

    pub(super) fn ready(&self, maximum_depth: u8) -> Option<(ChildKey, u8)> {
        let (depth, queue) = self.ready.first_key_value()?;
        if *depth >= maximum_depth { return None; }
        Some((queue.first()?, *depth))
    }

    pub(super) fn select(&mut self, key: &ChildKey) {
        let Some(parent) = self.parents.get_mut(key.parent.as_ref()) else { return; };
        if parent.revision != key.revision { return; }
        remove_from(&mut self.ready, parent.depth, key);
        if let Some(status) = parent.children.get_mut(&key.index) { *status = ChildState::Selected; }
    }

    pub(super) fn selected(&self, key: &ChildKey) -> bool {
        self.parents.get(key.parent.as_ref()).is_some_and(|parent| {
            parent.revision == key.revision
                && parent.children.get(&key.index) == Some(&ChildState::Selected)
        })
    }

    pub(super) fn cover(&mut self, parent_key: &str, index: usize) {
        let Some(parent) = self.parents.get_mut(parent_key) else { return; };
        let Some(status) = parent.children.get_mut(&index) else { return; };
        if *status == ChildState::Ready {
            remove_from(&mut self.ready, parent.depth, &ChildKey {
                parent: Arc::clone(&parent.key), revision: parent.revision, index,
            });
        } else if let ChildState::Cooling(deadline) = *status {
            self.retries.remove(&(deadline, ChildKey {
                parent: Arc::clone(&parent.key), revision: parent.revision, index,
            }));
        }
        *status = ChildState::Covered;
    }

    pub(super) fn rearm(&mut self, parent_key: &str, index: usize) {
        let Some(parent) = self.parents.get_mut(parent_key) else { return; };
        let Some(status) = parent.children.get_mut(&index) else { return; };
        // A selected batch owns its retry until all started workers have joined.
        if *status != ChildState::Covered { return; }
        *status = ChildState::Ready;
        self.ready.entry(parent.depth).or_default().push_back(ChildKey {
            parent: Arc::clone(&parent.key), revision: parent.revision, index,
        });
    }

    pub(super) fn has_work(&self, maximum_depth: u8) -> bool {
        self.cursor(maximum_depth).is_some() || self.ready(maximum_depth).is_some()
    }

    pub(super) fn cool_down(&mut self, parent_key: &str, index: usize, deadline: Instant) {
        self.cover(parent_key, index);
        let Some(parent) = self.parents.get_mut(parent_key) else { return; };
        let Some(status) = parent.children.get_mut(&index) else { return; };
        *status = ChildState::Cooling(deadline);
        self.retries.insert((deadline, ChildKey {
            parent: Arc::clone(&parent.key), revision: parent.revision, index,
        }));
    }

    pub(super) fn retry_due(&mut self, now: Instant) -> Option<ChildKey> {
        let (deadline, key) = self.retries.first()?.clone();
        if deadline > now { return None; }
        self.retries.remove(&(deadline, key.clone()));
        if let Some(parent) = self.parents.get_mut(key.parent.as_ref()) {
            if parent.revision == key.revision {
                if let Some(status) = parent.children.get_mut(&key.index) {
                    if *status == ChildState::Cooling(deadline) { *status = ChildState::Covered; }
                }
            }
        }
        Some(key)
    }
}

pub(super) fn byte_charge(key: &str, entries: &[crate::vfs::VfsMeta]) -> usize {
    // Includes geometric hash-table capacity, linked queue keys, cursor and
    // refresh indexes. Child keys share only a small parent string, never entries.
    key.len().saturating_mul(24).saturating_add(2048)
        .saturating_add(entries.iter().filter(|entry| entry.is_dir && !entry.is_symlink)
            .count().saturating_mul(768))
}
