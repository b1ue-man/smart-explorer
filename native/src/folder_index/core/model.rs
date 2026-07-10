use std::collections::HashSet;
use std::io;

pub(super) const MAX_INDEX_PATHS: usize = 1_000_000;
pub(super) const MAX_INDEX_PATH_TEXT_BYTES: usize = 128 * 1024 * 1024;
pub(super) const MAX_INDEX_DEPTH: usize = 512;
pub(super) const MAX_INDEX_FILE_BYTES: u64 = (MAX_INDEX_PATH_TEXT_BYTES + MAX_INDEX_PATHS) as u64;

#[derive(Clone)]
pub struct FolderIndex {
    /// Absolute folder paths with forward slashes, case-preserving.
    /// HashSet so live updates from the filesystem watcher (insert / remove)
    /// are O(1) even at 500k+ entries.
    pub(super) paths: HashSet<String>,
    pub(super) path_text_bytes: usize,
}

pub enum IndexMsg {
    Progress { count: u64, current: String },
    Complete(FolderIndex),
    Canceled,
    Failed(String),
}

impl FolderIndex {
    pub fn new() -> Self {
        Self {
            paths: HashSet::new(),
            path_text_bytes: 0,
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
        self.try_insert(path).unwrap_or(false)
    }

    /// Insert a path while preserving the persisted-index resource bounds.
    pub fn try_insert(&mut self, path: String) -> io::Result<bool> {
        validate_path(&path)?;
        if self.paths.contains(path.as_str()) {
            return Ok(false);
        }
        if self.paths.len() >= MAX_INDEX_PATHS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("folder index exceeds {MAX_INDEX_PATHS} paths"),
            ));
        }
        let next_bytes = self
            .path_text_bytes
            .checked_add(path.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path budget overflow"))?;
        if next_bytes > MAX_INDEX_PATH_TEXT_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("folder index path text exceeds {MAX_INDEX_PATH_TEXT_BYTES} bytes"),
            ));
        }
        self.paths.insert(path);
        self.path_text_bytes = next_bytes;
        Ok(true)
    }

    /// Remove a path. Returns true if removed.
    pub fn remove(&mut self, path: &str) -> bool {
        let Some(removed) = self.paths.take(path) else {
            return false;
        };
        self.path_text_bytes = self.path_text_bytes.saturating_sub(removed.len());
        true
    }

    /// Iterate all indexed paths. Order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.paths.iter()
    }
}

pub(super) fn validate_path(path: &str) -> io::Result<()> {
    if path.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "folder index path is empty",
        ));
    }
    if path.bytes().any(|byte| matches!(byte, b'\n' | b'\r')) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "folder index path contains a line break",
        ));
    }
    Ok(())
}

impl Default for FolderIndex {
    fn default() -> Self {
        Self::new()
    }
}
