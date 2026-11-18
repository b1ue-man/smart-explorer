//! The size tree from the NTFS master file table of a local, elevated
//! analysis: one sequential read of the MFT instead of a listing per folder.
//! The visitor applies the walk's rules (exclusions, retention budget,
//! largest files, aggregates, depth limit, progress) to the MFT walk; the
//! NTFS system files of a volume root become one node of their own
//! (`SYSTEM_FILES_NAME`, not removable). Only `scan()` of a local path on
//! Windows tries it; Share, peer and confined scans never do, and every
//! refusal or read failure leaves the ordinary directory walk.
#![cfg_attr(any(not(windows), test, feature = "test-support"), allow(dead_code))]
use super::assembly::Assembly;
use super::{AnalyticsBudget, Diagnostics, Progress, Retention, SizeNode};
use crate::local_access::mft::{MftDiagnostics, MftEntry, MftKind, MftTotals, MftVisitor};
use std::path::{Path, PathBuf};

pub use crate::local_access::mft::SYSTEM_FILES_NAME;

/// Note of the analysis way that ran.
pub(super) const MFT_WAY: &str = "Analyseweg: schneller NTFS-Weg (MFT)";

struct Folder {
    name: Box<str>,
    path: PathBuf,
    depth: u32,
    retention: Retention,
    assembly: Assembly,
}

/// Builds the size tree while `MftIndex::walk` visits the entries.
pub(super) struct TreeVisitor<'a> {
    pub(super) root: &'a Path,
    pub(super) progress: &'a Progress,
    pub(super) diagnostics: &'a Diagnostics,
    pub(super) budget: &'a AnalyticsBudget,
    pub(super) excluded: &'a [PathBuf],
    stack: Vec<Folder>,
    done: Option<(SizeNode, Retention)>,
    /// The next walk is the volume root's system-files section.
    system_files: bool,
}

impl<'a> TreeVisitor<'a> {
    pub(super) fn new(
        root: &'a Path,
        progress: &'a Progress,
        diagnostics: &'a Diagnostics,
        budget: &'a AnalyticsBudget,
        excluded: &'a [PathBuf],
    ) -> Self {
        Self {
            root,
            progress,
            diagnostics,
            budget,
            excluded,
            stack: Vec::new(),
            done: None,
            system_files: false,
        }
    }

    /// The next walk covers the NTFS system files of the volume root.
    pub(super) fn begin_system_files(&mut self) {
        self.system_files = true;
    }

    /// The tree of the last completed walk and the retention of its root.
    pub(super) fn take(&mut self) -> Option<(SizeNode, Retention)> {
        self.done.take()
    }

    fn path_of(&self, entry: &MftEntry<'_>) -> PathBuf {
        let mut path = self.root.to_path_buf();
        path.extend(entry.parents);
        path.push(entry.name);
        path
    }
}

impl MftVisitor for TreeVisitor<'_> {
    fn keep(&mut self, entry: &MftEntry<'_>) -> bool {
        if matches!(entry.kind, MftKind::Link | MftKind::Special) {
            return false;
        }
        if crate::apptrash::excluded_name(entry.name)
            || crate::vfs::is_engine_name(entry.name)
            || crate::vfs::is_staging_name(entry.name)
        {
            return false;
        }
        if entry.kind != MftKind::Directory {
            // A file with an unrepresentable name is still counted.
            return true;
        }
        let path = self.path_of(entry);
        if !entry.name_exact {
            self.diagnostics.record(
                crate::analytics::os::display_path(&path),
                "Ordnername ist nicht als Pfad darstellbar; Inhalt nicht erfasst",
                false,
            );
            return false;
        }
        let depth = entry.parents.len() as u32 + 1;
        if !self.budget.depth_allowed(depth) {
            self.diagnostics.record(
                crate::analytics::os::display_path(&path),
                format!(
                    "Verzeichnistiefe ueber {} wird nicht weiter erfasst",
                    self.budget.max_depth()
                ),
                false,
            );
            return false;
        }
        !self.excluded.iter().any(|hidden| hidden == &path)
            && !crate::agent_proto::is_pseudo_dir(&path.to_string_lossy())
    }

    fn enter_directory(&mut self, entry: &MftEntry<'_>) {
        let (path, depth, retention) = match self.stack.last() {
            // The walk root: the scan root's claim was made before the scan.
            None if self.system_files => {
                let path = self.root.join(SYSTEM_FILES_NAME);
                let retention =
                    self.budget
                        .claim(&path, 1, entry.name.len() as u64, self.diagnostics);
                (path, 1, retention)
            }
            None => (self.root.to_path_buf(), 0, Retention::Keep),
            Some(parent) => {
                let path = parent.path.join(entry.name);
                let depth = parent.depth + 1;
                let retention =
                    self.budget
                        .claim(&path, depth, entry.name.len() as u64, self.diagnostics);
                self.progress
                    .dirs
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                (path, depth, retention)
            }
        };
        if Progress::directory_due() {
            self.progress
                .enter_directory(&crate::analytics::os::display_path(&path));
        }
        self.stack.push(Folder {
            name: entry.name.into(),
            path,
            depth,
            retention,
            assembly: Assembly::default(),
        });
    }

    fn leaf(&mut self, entry: &MftEntry<'_>) {
        if let Some(folder) = self.stack.last_mut() {
            folder
                .assembly
                .file(entry.name, entry.size, entry.allocated);
            folder.assembly.report(self.progress, false);
        }
    }

    fn leave_directory(&mut self, _totals: &MftTotals) {
        let Some(mut folder) = self.stack.pop() else {
            return;
        };
        folder.assembly.report(self.progress, true);
        let node = folder.assembly.finish(
            folder.name,
            &folder.path,
            folder.depth,
            self.budget,
            self.diagnostics,
        );
        self.diagnostics.finish_directory(&folder.path, node.size);
        match self.stack.last_mut() {
            Some(parent) => parent.assembly.child(node, folder.retention),
            None => self.done = Some((node, folder.retention)),
        }
    }
}

/// The MFT's findings: with `partial` they are issues (sizes are lower
/// bounds), otherwise notes.
pub(super) fn report(diagnostics: &Diagnostics, root: &Path, found: &MftDiagnostics) {
    let place = crate::analytics::os::display_path(root);
    for text in &found.notes {
        if found.partial {
            diagnostics.record(place.clone(), text.clone(), false);
        } else {
            diagnostics.note(text.clone());
        }
    }
    if found.omitted_notes > 0 {
        diagnostics.note(format!(
            "{} weitere MFT-Hinweise nicht einzeln aufgeführt",
            found.omitted_notes
        ));
    }
}

/// Adds the system-files node to the root, keeping both sums exact.
pub(super) fn attach_system_files(tree: &mut SizeNode, system: SizeNode, retention: Retention) {
    tree.size = tree.size.saturating_add(system.size);
    tree.allocated = tree.allocated.saturating_add(system.allocated);
    match retention {
        Retention::Keep => tree.children.push(system),
        Retention::Aggregate => tree.children.push(SizeNode {
            name: crate::analytics::aggregate_name(1).into_boxed_str(),
            size: system.size,
            allocated: system.allocated,
            is_dir: false,
            children: Vec::new(),
        }),
    }
}

#[cfg(windows)]
#[path = "../windows/mft_scan.rs"]
mod windows;
#[cfg(all(windows, not(any(test, feature = "test-support"))))]
pub(super) use windows::scan_local;

#[cfg(test)]
#[path = "analytics_mft_tests.rs"]
mod tests;
