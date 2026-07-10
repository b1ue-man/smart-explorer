use crate::types::FileEntry;
use std::collections::HashSet;
use std::path::Path;

use super::super::platform;

const MAX_COPY_ENTRIES: usize = 1_000_000;
const MAX_COPY_TEXT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Default)]
pub(super) struct EntryAccumulator {
    entries: Vec<FileEntry>,
    text_bytes: usize,
}

impl EntryAccumulator {
    pub(super) fn push(&mut self, entry: FileEntry) -> Result<(), String> {
        let added = entry
            .path
            .len()
            .saturating_add(entry.parent.len())
            .saturating_add(entry.name.len())
            .saturating_add(entry.ext.len());
        let next_bytes = self.text_bytes.saturating_add(added);
        if self.entries.len() >= MAX_COPY_ENTRIES || next_bytes > MAX_COPY_TEXT_BYTES {
            return Err(limit_error());
        }
        self.text_bytes = next_bytes;
        self.entries.push(entry);
        Ok(())
    }

    pub(super) fn extend(
        &mut self,
        entries: impl IntoIterator<Item = FileEntry>,
    ) -> Result<(), String> {
        for entry in entries {
            self.push(entry)?;
        }
        Ok(())
    }

    pub(super) fn into_entries(self) -> Vec<FileEntry> {
        self.entries
    }
}

pub(super) fn dedupe_entries(mut entries: Vec<FileEntry>) -> Vec<FileEntry> {
    entries.sort_by_key(|entry| path_depth(Path::new(entry.path.as_ref())));
    let mut kept: Vec<FileEntry> = Vec::with_capacity(entries.len());
    let mut exact = HashSet::new();
    let mut directory_keys = HashSet::new();
    for entry in entries {
        let path = Path::new(entry.path.as_ref());
        let key = match platform::path_key(path) {
            Ok(key) => key,
            Err(_) => {
                kept.push(entry);
                continue;
            }
        };
        if !exact.insert(key)
            || path.ancestors().skip(1).any(|ancestor| {
                platform::path_key(ancestor)
                    .ok()
                    .is_some_and(|key| directory_keys.contains(&key))
            })
        {
            continue;
        }
        if entry.is_dir && !entry.is_symlink {
            if let Ok(key) = platform::path_key(path) {
                directory_keys.insert(key);
            }
        }
        kept.push(entry);
    }
    kept
}

pub(super) fn dedupe_paths(mut paths: Vec<String>) -> Vec<String> {
    paths.sort_by_key(|path| path_depth(Path::new(path)));
    let mut kept: Vec<String> = Vec::with_capacity(paths.len());
    let mut exact = HashSet::new();
    let mut directory_keys = HashSet::new();
    for path in paths {
        let candidate = Path::new(&path);
        let key = match platform::path_key(candidate) {
            Ok(key) => key,
            Err(_) => {
                kept.push(path);
                continue;
            }
        };
        if !exact.insert(key)
            || candidate.ancestors().skip(1).any(|ancestor| {
                platform::path_key(ancestor)
                    .ok()
                    .is_some_and(|key| directory_keys.contains(&key))
            })
        {
            continue;
        }
        if std::fs::symlink_metadata(candidate)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false)
        {
            if let Ok(key) = platform::path_key(candidate) {
                directory_keys.insert(key);
            }
        }
        kept.push(path);
    }
    kept
}

pub(super) fn validate_pair_budget(pairs: &[(String, String)]) -> Result<(), String> {
    let bytes = pairs.iter().try_fold(0usize, |total, (source, target)| {
        total.checked_add(source.len().saturating_add(target.len()))
    });
    if pairs.len() > MAX_COPY_ENTRIES || bytes.is_none_or(|bytes| bytes > MAX_COPY_TEXT_BYTES) {
        return Err(limit_error());
    }
    Ok(())
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

fn limit_error() -> String {
    format!(
        "Kopiervorgang überschreitet das Limit von {MAX_COPY_ENTRIES} Einträgen oder {} MiB Pfadtext.",
        MAX_COPY_TEXT_BYTES / (1024 * 1024)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn entry(path: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            path: Arc::from(path),
            parent: Arc::from(""),
            name: Arc::from(path.rsplit('/').next().unwrap_or(path)),
            ext: Arc::from(""),
            size: 0,
            mtime_ms: 0,
            btime_ms: 0,
            is_dir,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: 0,
            id: None,
        }
    }

    #[test]
    fn parent_selection_removes_duplicate_child_seed() {
        let entries = dedupe_entries(vec![
            entry("/root/folder/file", false),
            entry("/root/folder", true),
        ]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path.as_ref(), "/root/folder");
    }

    #[test]
    fn similarly_prefixed_sibling_is_not_deduplicated() {
        let entries = dedupe_entries(vec![entry("/root/a", true), entry("/root/ab/file", false)]);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn exact_duplicate_is_deduplicated() {
        let entries = dedupe_entries(vec![entry("/root/a", false), entry("/root/a", false)]);
        assert_eq!(entries.len(), 1);
    }
}
