//! The active view's selection as a transfer source, shared by copy, drag,
//! the copy dialog and "Für andere Programme bereitstellen". A filtered
//! recursive view hands over its current matching files with their paths
//! below the view root (a snapshot from the tree in memory: sizes, times and
//! provider ids included, nothing is scanned again); every other view hands
//! over its outermost selected entries with the view's filter for folders.
use super::prelude::*;
use super::transfer_route::{parent_dir, TransferPlace, TransferSelection};
use super::*;
use crate::transfer::PairItem;

pub(in crate::app) struct ViewSelection {
    pub(in crate::app) selection: TransferSelection,
    /// Outermost selected entries, forward slashes.
    pub(in crate::app) roots: Vec<String>,
    /// The filter below selected folders when the selection is handed to
    /// another program as whole entries (virtual files, drag-out).
    pub(in crate::app) view_filter: Option<(FilterDef, String)>,
    /// The matching files of a filtered recursive view.
    pub(in crate::app) snapshot: Option<Vec<FileEntry>>,
    pub(in crate::app) has_dir: bool,
}

/// Why nothing can be handed over.
pub(in crate::app) enum SelectionIssue {
    /// Nothing (matching) is selected: a hint, not an error.
    Empty(String),
    Invalid(String),
}

impl App {
    /// `cut` hands over whole entries: moving never applies a filter.
    pub(in crate::app) fn view_selection(
        &self,
        cut: bool,
    ) -> Result<ViewSelection, SelectionIssue> {
        let place = self.current_place();
        let roots =
            super::recursive_clipboard::outer_selection_roots(&self.entries, &self.selection);
        if roots.is_empty() {
            return Err(SelectionIssue::Empty(
                "Nichts ausgewählt — bitte erst Dateien markieren".to_string(),
            ));
        }
        let has_dir = self
            .entries
            .iter()
            .any(|entry| entry.is_dir && self.selection.contains(&entry.key()));
        if self.recursive && self.filter_is_active() && !cut {
            let files = self.recursive_transfer_files();
            if files.is_empty() {
                return Err(SelectionIssue::Empty(
                    "Keine Dateien entsprechen dem aktiven Filter".to_string(),
                ));
            }
            let root = self.root_prefix();
            let pairs = snapshot_pairs(&files, &root).map_err(SelectionIssue::Invalid)?;
            return Ok(ViewSelection {
                selection: TransferSelection::pairs(place, pairs),
                roots,
                view_filter: Some((self.filter.clone(), root)),
                snapshot: Some(files),
                has_dir,
            });
        }
        let filter = (!cut && has_dir && self.filter_is_active())
            .then(|| (self.filter.clone(), self.root_prefix()));
        // Local selections keep the layout the file clipboard always gave
        // them (paths below the first entry's folder); remote entries land
        // under their names as downloads always did.
        let base = place.is_local().then(|| parent_dir(&roots[0]));
        Ok(ViewSelection {
            selection: TransferSelection::roots(place, roots.clone(), base)
                .with_filter(filter.clone()),
            roots,
            view_filter: filter,
            snapshot: None,
            has_dir,
        })
    }

    /// "Kopieren/Verschieben nach…": the snapshot of a filtered recursive
    /// view, else the outermost selected entries below the view root with
    /// the active filter (for moves too, as the dialog always did).
    pub(in crate::app) fn dialog_selection(&self) -> Result<TransferSelection, SelectionIssue> {
        let place = self.current_place();
        let root = self.root_prefix();
        if self.recursive && self.filter_is_active() {
            let files = self.recursive_transfer_files();
            if files.is_empty() {
                return Err(SelectionIssue::Empty(
                    "Keine Dateien entsprechen dem aktiven Filter".to_string(),
                ));
            }
            let pairs = snapshot_pairs(&files, &root).map_err(SelectionIssue::Invalid)?;
            return Ok(TransferSelection::pairs(place, pairs));
        }
        let roots =
            super::recursive_clipboard::outer_selection_roots(&self.entries, &self.selection);
        if roots.is_empty() {
            return Err(SelectionIssue::Empty(
                "Nichts ausgewählt — bitte erst Dateien markieren".to_string(),
            ));
        }
        let filter = self
            .filter_is_active()
            .then(|| (self.filter.clone(), root.clone()));
        Ok(TransferSelection::roots(place, roots, Some(root)).with_filter(filter))
    }
}

/// Paths from the OS (file clipboard, dropped files) as a local selection;
/// they keep the layout below the first path's folder, as before.
pub(in crate::app) fn os_paths_selection(paths: Vec<String>) -> Option<TransferSelection> {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"))
        .filter(|path| !path.is_empty())
        .collect();
    let base = parent_dir(paths.first()?);
    Some(TransferSelection::roots(
        TransferPlace::local(),
        paths,
        Some(base),
    ))
}

/// The snapshot's files with their destination paths below `root` and what
/// the view already knows about them, so the engine needs no stat per file.
fn snapshot_pairs(files: &[FileEntry], root: &str) -> Result<Vec<PairItem>, String> {
    files
        .iter()
        .map(|entry| {
            let rel = crate::filter::tree::relative_path(&entry.path, root)
                .ok_or_else(|| format!("{}: liegt außerhalb der Auswahlwurzel", entry.path))?;
            Ok(PairItem {
                source: entry.path.to_string(),
                rel: rel.to_string(),
                size: Some(entry.size),
                mtime_ms: entry.mtime_ms,
                id: entry.id.as_deref().map(str::to_string),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "transfer_selection_tests.rs"]
mod tests;
