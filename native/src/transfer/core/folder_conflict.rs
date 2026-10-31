//! Conflicts of selected source folders, independent of file conflicts.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FolderConflict {
    /// Preserve the explicitly named destination (terminal compatibility).
    #[default]
    Merge,
    /// Exclusively reserve a sibling; never mix with an existing tree.
    KeepBoth,
    Skip,
    Fail,
}

/// Keys are exact source roots within the job's source namespace. Scaffold
/// parents and individual files never inherit another root's decision.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderConflictPolicy {
    pub default: FolderConflict,
    pub roots: BTreeMap<String, FolderConflict>,
    /// Files below a selected folder whose decision is `Merge` follow the
    /// file rule "neuere ersetzen ältere" (`Conflict::Update`) instead of the
    /// job's file policy. A file selected on its own keeps the job's policy.
    /// Absent in records written before the rule, which means false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub merge_update: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl FolderConflictPolicy {
    pub fn new(default: FolderConflict) -> Self {
        Self {
            default,
            roots: BTreeMap::new(),
            merge_update: false,
        }
    }

    /// The files of merged folders follow "neuere ersetzen ältere".
    pub fn with_merge_update(mut self) -> Self {
        self.merge_update = true;
        self
    }

    pub fn for_root(&self, source: &str) -> FolderConflict {
        self.roots.get(source).copied().unwrap_or(self.default)
    }
}

#[derive(Clone, Debug, Default)]
pub struct FolderConflictPreview {
    pub source: String,
    pub relative: String,
    pub keep_both: String,
    pub keep_both_supported: bool,
    pub existing: bool,
    pub new_files: u64,
    /// Same-named files at the target (the four counts below, together).
    pub colliding_files: u64,
    /// Of those: the source is newer (replaces under the file rule) ...
    pub newer_files: u64,
    /// ... the same file (skipped) ...
    pub equal_files: u64,
    /// ... the target is newer (kept) ...
    pub target_newer_files: u64,
    /// ... same time, other size (both kept).
    pub conflict_files: u64,
    pub samples: Vec<String>,
    pub omitted: u64,
    pub omitted_paths: Vec<String>,
    pub note: Option<String>,
    pub complete: bool,
    pub counted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FolderPreviewState {
    pub ready: bool,
    pub error: Option<String>,
    pub folders: Vec<FolderConflictPreview>,
}
