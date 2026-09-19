use crate::app::recursive_tree::relative_path;
use crate::app::shared_platform_helpers::ClipboardVirtualFile;
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

pub(in crate::app) fn clipboard_snapshot(
    entries: Vec<FileEntry>,
    root: &str,
) -> Result<Vec<ClipboardVirtualFile>, String> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .map(|entry| {
            if entry.is_dir || entry.is_symlink {
                return Err(format!("{}: keine reguläre Datei", entry.path));
            }
            let relative = relative_path(&entry.path, root)
                .ok_or_else(|| format!("{}: liegt außerhalb der Auswahlwurzel", entry.path))?;
            for component in relative.split('/') {
                crate::vfs::validate_child_name(component).map_err(|error| error.to_string())?;
            }
            if !seen.insert(relative.to_string()) {
                return Err(format!("Mehrdeutiger Zielpfad in der Auswahl: {relative}"));
            }
            Ok(ClipboardVirtualFile {
                abs: entry.path.to_string(),
                rel: relative.to_string(),
                size: entry.size,
                mtime_ms: entry.mtime_ms,
            })
        })
        .collect()
}
