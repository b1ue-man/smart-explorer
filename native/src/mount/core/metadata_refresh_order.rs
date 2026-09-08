//! Indexed, duplicate-free ordering. No queue owns a snapshot or listing Arc.
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

struct Links<K> { previous: Option<K>, next: Option<K> }

pub(super) struct KeyQueue<K> {
    links: HashMap<K, Links<K>>,
    first: Option<K>,
    last: Option<K>,
}

impl<K> Default for KeyQueue<K> {
    fn default() -> Self {
        Self { links: HashMap::new(), first: None, last: None }
    }
}

impl<K: Clone + Eq + Hash> KeyQueue<K> {
    pub(super) fn first(&self) -> Option<K> { self.first.clone() }
    pub(super) fn contains(&self, key: &K) -> bool { self.links.contains_key(key) }
    pub(super) fn is_empty(&self) -> bool { self.links.is_empty() }

    pub(super) fn remove(&mut self, key: &K) {
        let Some(links) = self.links.remove(key) else { return; };
        if let Some(previous) = &links.previous {
            if let Some(node) = self.links.get_mut(previous) { node.next = links.next.clone(); }
        } else { self.first = links.next.clone(); }
        if let Some(next) = &links.next {
            if let Some(node) = self.links.get_mut(next) { node.previous = links.previous; }
        } else { self.last = links.previous; }
        // Bound retained hash-table capacity after a large directory is retired.
        // Geometric shrinking amortizes rebuilding over the removed records.
        if self.links.capacity() > 32 && self.links.capacity() / 4 > self.links.len() {
            self.links.shrink_to(self.links.len().saturating_mul(2));
        }
    }

    pub(super) fn push_back(&mut self, key: K) {
        self.remove(&key);
        if let Some(last) = &self.last {
            if let Some(node) = self.links.get_mut(last) { node.next = Some(key.clone()); }
        } else { self.first = Some(key.clone()); }
        self.links.insert(key.clone(), Links { previous: self.last.take(), next: None });
        self.last = Some(key);
    }

    fn push_front(&mut self, key: K) {
        self.remove(&key);
        if let Some(first) = &self.first {
            if let Some(node) = self.links.get_mut(first) { node.previous = Some(key.clone()); }
        } else { self.last = Some(key.clone()); }
        self.links.insert(key.clone(), Links { previous: None, next: self.first.take() });
        self.first = Some(key);
    }

    fn append_unselected(&self, selected: &mut Vec<String>, seen: &mut HashSet<String>, limit: usize)
    where K: AsRef<str> {
        let mut cursor = self.first();
        while selected.len() < limit {
            let Some(key) = cursor else { break; };
            cursor = self.links.get(&key).and_then(|links| links.next.clone());
            if seen.insert(key.as_ref().to_string()) { selected.push(key.as_ref().to_string()); }
        }
    }
}

#[derive(Default)]
pub(super) struct RefreshOrder {
    age: KeyQueue<String>,
    recent: KeyQueue<String>,
    pending: KeyQueue<String>,
}

impl RefreshOrder {
    pub(super) fn install(&mut self, key: &str, accessed: bool, pending: bool) {
        let key = key.to_string();
        self.age.push_back(key.clone());
        if accessed && !self.recent.contains(&key) { self.recent.push_front(key.clone()); }
        if pending {
            if !self.pending.contains(&key) { self.pending.push_front(key); }
        } else { self.pending.remove(&key); }
    }

    pub(super) fn touch(&mut self, key: &str) {
        let key = key.to_string();
        if !self.age.contains(&key) { return; }
        self.recent.push_front(key.clone());
        self.pending.push_front(key);
    }

    pub(super) fn remove(&mut self, key: &str) {
        let key = key.to_string();
        self.age.remove(&key);
        self.recent.remove(&key);
        self.pending.remove(&key);
    }

    pub(super) fn select(&mut self, root: Option<String>, limit: usize) -> Vec<String> {
        let mut selected = Vec::with_capacity(limit);
        let mut seen = HashSet::with_capacity(limit);
        if let Some(root) = root { seen.insert(root.clone()); selected.push(root); }
        let cold_limit = (selected.len() + 1).min(limit);
        self.age.append_unselected(&mut selected, &mut seen, cold_limit);
        let recent_limit = (selected.len() + 1).min(limit);
        self.recent.append_unselected(&mut selected, &mut seen, recent_limit);
        self.pending.append_unselected(&mut selected, &mut seen, limit);
        self.recent.append_unselected(&mut selected, &mut seen, limit);
        self.age.append_unselected(&mut selected, &mut seen, limit);
        for key in &selected {
            if self.age.contains(key) { self.age.push_back(key.clone()); }
        }
        selected
    }
}
