//! Recursive result trees independent of any view: the complete membership of
//! a filtered recursive scan, the files a selection of it stands for, and the
//! relative-path snapshot a filtered copy transfers.
use super::CompiledFilter;
use crate::types::{FileEntry, FilterDef};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// One file of a filtered selection with its path relative to the snapshot
/// root, as a virtual clipboard or a relative-path upload consumes it.
#[derive(Clone, Debug)]
pub struct ClipboardVirtualFile {
    pub abs: String,
    pub rel: String,
    pub size: u64,
    pub mtime_ms: i64,
}

/// `path` relative to `root`, or `None` when it is `root` itself or outside.
pub fn relative_path<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    path.strip_prefix(root.trim_end_matches('/'))
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
}

/// Link by directory identity, not by an exact spelling of a root row. A
/// streamed match whose ancestors have not arrived yet stays visible at the
/// nearest known ancestor; the next update restores its full placement.
pub fn result_rows(
    entries: &[FileEntry],
    root: &str,
    filter: &FilterDef,
    mut compare: impl FnMut(usize, usize) -> std::cmp::Ordering,
) -> Vec<(usize, u32)> {
    let root = root.trim_end_matches('/');
    let directories: HashMap<_, _> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.is_dir && entry.depth > 0)
        .map(|(index, entry)| (entry.path.trim_end_matches('/'), index))
        .collect();
    let mut parents = vec![None; entries.len()];
    let mut order: Vec<_> = (0..entries.len()).collect();
    order.sort_unstable_by_key(|&index| entries[index].depth);
    let mut allowed = vec![false; entries.len()];
    let mut matching = vec![false; entries.len()];
    let compiled = CompiledFilter::compile(filter);
    let mut children = vec![Vec::new(); entries.len() + 1];
    let virtual_root = entries.len();
    for &index in &order {
        let entry = &entries[index];
        if entry.depth == 0 || relative_path(&entry.path, root).is_none() {
            continue;
        }
        let mut parent = entry.parent.trim_end_matches('/');
        while parent != root && relative_path(parent, root).is_some() {
            if let Some(&candidate) = directories.get(parent) {
                if entries[candidate].depth < entry.depth {
                    parents[index] = Some(candidate);
                    break;
                }
            }
            let Some((ancestor, _)) = parent.rsplit_once('/') else {
                break;
            };
            parent = ancestor;
        }
        let flags_allowed =
            (!entry.hidden || filter.include_hidden) && (!entry.system || filter.include_system);
        allowed[index] = flags_allowed && parents[index].is_none_or(|parent| allowed[parent]);
        matching[index] = allowed[index] && compiled.matches(entry, root);
        children[parents[index].unwrap_or(virtual_root)].push(index);
    }
    // Parents always have smaller depths; one reverse pass propagates results
    // even when the channel delivered entries in a different order.
    for &index in order.iter().rev() {
        if matching[index] {
            if let Some(parent) = parents[index] {
                matching[parent] = true;
            }
        }
    }
    for siblings in &mut children {
        siblings.retain(|&index| matching[index]);
        siblings.sort_unstable_by(|&left, &right| compare(left, right));
    }
    let mut stack: Vec<_> = children[virtual_root]
        .iter()
        .rev()
        .map(|&index| (index, 0))
        .collect();
    let mut rows = Vec::new();
    while let Some((index, depth)) = stack.pop() {
        let show = !entries[index].is_dir || filter.include_dirs;
        if show {
            rows.push((index, depth));
        }
        let next_depth = depth + u32::from(show);
        stack.extend(
            children[index]
                .iter()
                .rev()
                .map(|&child| (child, next_depth)),
        );
    }
    rows
}

/// A recursive transfer is a snapshot of the current matching files. Folder
/// rows select their matching descendants, regardless of their folded state.
pub fn selected_files(
    entries: &[FileEntry],
    rows: &[(usize, u32)],
    selected: &HashSet<Arc<str>>,
    filter: &FilterDef,
    root: &str,
) -> Vec<FileEntry> {
    let directories: HashSet<_> = rows
        .iter()
        .map(|&(index, _)| &entries[index])
        .filter(|entry| entry.is_dir && selected.contains(&entry.key()))
        .map(|entry| entry.path.trim_end_matches('/'))
        .collect();
    let compiled = CompiledFilter::compile(filter);
    let mut seen = HashSet::new();
    rows.iter()
        .filter_map(|&(index, _)| {
            let entry = &entries[index];
            if entry.is_dir || !compiled.matches(entry, root) || !seen.insert(entry.key()) {
                return None;
            }
            let mut parent = entry.parent.trim_end_matches('/');
            let mut included = selected.contains(&entry.key());
            while !included && relative_path(parent, root).is_some() {
                included = directories.contains(parent);
                let Some((ancestor, _)) = parent.rsplit_once('/') else {
                    break;
                };
                parent = ancestor;
            }
            included.then(|| entry.clone())
        })
        .collect()
}

/// Validated relative-path snapshot of regular files below `root`; refuses
/// directories, links, entries outside `root` and ambiguous relative targets.
pub fn clipboard_snapshot(
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
