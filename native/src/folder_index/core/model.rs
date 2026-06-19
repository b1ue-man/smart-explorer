use std::collections::HashSet;
use std::time::SystemTime;

pub struct FolderIndex {
    /// Absolute folder paths with forward slashes, case-preserving.
    /// HashSet so live updates from the filesystem watcher (insert / remove)
    /// are O(1) even at 500k+ entries.
    pub(super) paths: HashSet<String>,
    /// Modified-time of the saved index file, if loaded from disk.
    pub built_at: Option<SystemTime>,
}

pub enum IndexMsg {
    Progress { count: u64, current: String },
    Done(FolderIndex),
    Error(String),
}

impl FolderIndex {
    pub fn new() -> Self {
        Self {
            paths: HashSet::new(),
            built_at: None,
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

    /// True if the index contains exactly this path (no prefix matching).
    pub fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }

    /// Iterate all indexed paths. Order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.paths.iter()
    }
}
