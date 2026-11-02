//! A recursive transfer is a snapshot of the current matching files. Folder
//! rows select their matching descendants, regardless of their folded state
//! (`crate::filter::tree::selected_files`).
use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn recursive_transfer_files(&self) -> Vec<FileEntry> {
        crate::filter::tree::selected_files(
            &self.entries,
            &self.tree.rows,
            &self.selection,
            &self.filter,
            &self.root_prefix(),
        )
    }
}
