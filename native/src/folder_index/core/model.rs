use std::collections::HashSet;

pub struct FolderIndex {
    /// Absolute folder paths with forward slashes, case-preserving.
    /// HashSet so live updates from the filesystem watcher (insert / remove)
    /// are O(1) even at 500k+ entries.
    pub(super) paths: HashSet<String>,
}

pub enum IndexMsg {
    Progress { count: u64, current: String },
    Done(FolderIndex),
}

impl FolderIndex {
    pub fn new() -> Self {
        Self {
            paths: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Insert a path. Returns true if new.
    pub fn insert(&mut self, path: String) -> bool {
        self.paths.insert(path)
    }

    /// Remove a path. Returns true if removed.
    pub fn remove(&mut self, path: &str) -> bool {
        self.paths.remove(path)
    }

    /// Iterate all indexed paths. Order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.paths.iter()
    }
}
