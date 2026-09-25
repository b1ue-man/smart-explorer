//! Clipboard selections of recursive views. The validated relative-path
//! snapshot lives in `crate::filter::tree::clipboard_snapshot`.
pub(in crate::app) use crate::filter::tree::clipboard_snapshot;
use crate::types::FileEntry;
use std::collections::HashSet;
use std::sync::Arc;

/// Whole-folder clipboard operations need only the outer selected roots.
pub(in crate::app) fn plain_selection_paths(
    entries: &[FileEntry],
    selected: &HashSet<Arc<str>>,
) -> Vec<String> {
    let directories: HashSet<_> = entries
        .iter()
        .filter(|entry| entry.is_dir && selected.contains(&entry.key()))
        .map(|entry| entry.path.trim_end_matches('/'))
        .collect();
    entries
        .iter()
        .filter(|entry| selected.contains(&entry.key()))
        .filter(|entry| {
            let mut parent = entry.parent.trim_end_matches('/');
            while !parent.is_empty() {
                if directories.contains(parent) {
                    return false;
                }
                let Some((next, _)) = parent.rsplit_once('/') else {
                    break;
                };
                parent = next;
            }
            true
        })
        .map(|entry| entry.path.replace('/', "\\"))
        .collect()
}
