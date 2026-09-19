//! A recursive transfer is a snapshot of the current matching files. Folder
//! rows select their matching descendants, regardless of their folded state.
use super::prelude::*;
use super::*;

pub(super) fn selected_files(
    entries: &[FileEntry],
    rows: &[(usize, u32)],
    selected: &HashSet<Arc<str>>,
    filter: &FilterDef,
    root: &str,
) -> Vec<FileEntry> {
    let directories: HashSet<_> = rows.iter()
        .map(|&(index, _)| &entries[index])
        .filter(|entry| entry.is_dir && selected.contains(&entry.key()))
        .map(|entry| entry.path.trim_end_matches('/'))
        .collect();
    let compiled = CompiledFilter::compile(filter);
    let mut seen = HashSet::new();
    rows.iter().filter_map(|&(index, _)| {
        let entry = &entries[index];
        if entry.is_dir || !compiled.matches(entry, root) || !seen.insert(entry.key()) {
            return None;
        }
        let mut parent = entry.parent.trim_end_matches('/');
        let mut included = selected.contains(&entry.key());
        while !included && super::recursive_tree::relative_path(parent, root).is_some() {
            included = directories.contains(parent);
            let Some((ancestor, _)) = parent.rsplit_once('/') else { break };
            parent = ancestor;
        }
        included.then(|| entry.clone())
    }).collect()
}

impl App {
    pub(in crate::app) fn recursive_transfer_files(&self) -> Vec<FileEntry> {
        selected_files(&self.entries, &self.tree.rows, &self.selection, &self.filter, &self.root_prefix())
    }
}
