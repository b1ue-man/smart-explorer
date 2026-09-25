//! Complete result membership is independent of the rows currently unfolded.
//! The membership itself (`result_rows`) lives in `crate::filter::tree`.
use crate::types::FileEntry;
use std::collections::HashSet;
use std::sync::Arc;

#[cfg(test)]
pub(super) use crate::filter::tree::relative_path;
pub(super) use crate::filter::tree::result_rows;

#[derive(Default)]
pub(super) struct RecursiveView {
    pub(super) rows: Vec<(usize, u32)>,
    pub(super) collapsed: HashSet<Arc<str>>,
}

impl RecursiveView {
    pub(super) fn displayed(&self, entries: &[FileEntry]) -> Vec<(usize, u32)> {
        let mut hidden_below = None;
        self.rows
            .iter()
            .copied()
            .filter(|&(index, depth)| {
                if hidden_below.is_some_and(|parent| depth > parent) {
                    return false;
                }
                hidden_below = None;
                let entry = &entries[index];
                if entry.is_dir && self.collapsed.contains(&entry.key()) {
                    hidden_below = Some(depth);
                }
                true
            })
            .collect()
    }
}
