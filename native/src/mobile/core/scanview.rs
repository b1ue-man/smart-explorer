//! Tree rows of a recursive scan (`scan.view`): the desktop result tree,
//! folded by the collapsed folders and cut into a window.
use super::args::SortSpec;
use crate::types::{FileEntry, FilterDef};

/// Every matching row in tree order with its display depth, plus whether a
/// row has visible children.
pub(crate) struct TreeRows {
    pub rows: Vec<(usize, u32)>,
    pub has_children: Vec<bool>,
}

pub(crate) fn tree_rows(
    entries: &[FileEntry],
    root: &str,
    filter: &FilterDef,
    sort: SortSpec,
) -> TreeRows {
    let rows = crate::filter::tree::result_rows(entries, root, filter, |left, right| {
        crate::format::compare_entries(
            &entries[left],
            &entries[right],
            sort.key,
            sort.dir,
            sort.dirs_first,
        )
    });
    let has_children = rows
        .iter()
        .enumerate()
        .map(|(position, &(_, depth))| {
            rows.get(position + 1)
                .is_some_and(|&(_, next_depth)| next_depth > depth)
        })
        .collect();
    TreeRows { rows, has_children }
}

/// Positions in `tree.rows` that stay visible when the folders for which
/// `is_collapsed` holds are folded.
pub(crate) fn visible_rows(
    tree: &TreeRows,
    entries: &[FileEntry],
    is_collapsed: impl Fn(&FileEntry) -> bool,
) -> Vec<usize> {
    let mut visible = Vec::with_capacity(tree.rows.len());
    let mut folded_at: Option<u32> = None;
    for (position, &(index, depth)) in tree.rows.iter().enumerate() {
        if let Some(fold_depth) = folded_at {
            if depth > fold_depth {
                continue;
            }
            folded_at = None;
        }
        visible.push(position);
        let entry = &entries[index];
        if entry.is_dir && tree.has_children[position] && is_collapsed(entry) {
            folded_at = Some(depth);
        }
    }
    visible
}

/// The `[offset, offset + limit)` slice of `items`, clamped.
pub(crate) fn window<T>(items: &[T], offset: usize, limit: usize) -> &[T] {
    let start = offset.min(items.len());
    let end = start.saturating_add(limit).min(items.len());
    &items[start..end]
}
