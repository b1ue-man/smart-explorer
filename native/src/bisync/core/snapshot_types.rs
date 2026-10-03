//! One side's complete observation for a planning pass (V3): the files that
//! take part, the files the job's filters left out, every folder and the
//! protected omissions. The walk (`snapshot*.rs`) fills it; the planner
//! decides per pair what a filtered or omitted entry means.
use std::collections::BTreeSet;

use super::omissions::SyncOmissions;
use super::types::Tree;

/// Folders of one side: relative paths in that side's own spelling, without
/// the root itself and without omitted folders.
pub type DirSet = BTreeSet<String>;

#[derive(Clone, Debug, Default)]
pub struct SideSnapshot {
    /// Regular files that passed the job's filters (rel -> signature).
    pub tree: Tree,
    /// Existing regular files the job's filters left out on this side
    /// (hidden, ignore pattern, size, age), with their signature and hash 0.
    /// The planner excludes such a rel on both sides (two-way) or follows the
    /// source (one-way), and never reads it as a deletion (Y36/Y68).
    pub filtered: Tree,
    /// Every folder below the root, empty ones included (FS5).
    pub dirs: DirSet,
    /// Protected omissions of this side (filtered folders are recorded here
    /// as `OmissionKind::Filtered`, not entered).
    pub omissions: SyncOmissions,
}

impl SideSnapshot {
    /// An empty observation whose omissions fold letter case as the pair does.
    pub fn new(fold_case: bool) -> Self {
        Self {
            omissions: SyncOmissions::new(fold_case),
            ..Self::default()
        }
    }

    /// Nothing of the user's was found below the root: no file, no filtered
    /// file, no folder and no protected user entry; the engine's own entries
    /// (replica marker, versions) do not count (the "side is empty" check of
    /// FS3).
    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
            && self.filtered.is_empty()
            && self.dirs.is_empty()
            && self.omissions.is_empty()
    }

    /// Files (taking part or filtered) plus folders.
    pub fn entry_count(&self) -> u64 {
        (self.tree.len() + self.filtered.len() + self.dirs.len()) as u64
    }
}
