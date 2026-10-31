//! The terminal's choice of what happens to same-named files: `--conflict`.
use crate::transfer::TransferProgress;

/// `--conflict`. Without it an existing destination file is an error, or is
/// replaced with `--force`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(super) enum FileRule {
    /// Missing files are inserted, a newer file replaces the older one by
    /// modification time, equal files are skipped, a newer destination file
    /// stays, and files of the same time but another size are both kept.
    Update,
}

/// What the rule "update" decided in one run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RuleCounts {
    pub(super) replaced: u64,
    pub(super) skipped_equal: u64,
    pub(super) kept_newer: u64,
    pub(super) kept_both: u64,
}

impl RuleCounts {
    pub(super) fn of(progress: &TransferProgress) -> Self {
        Self {
            replaced: progress.replaced,
            skipped_equal: progress.skipped_equal,
            kept_newer: progress.kept_newer,
            kept_both: progress.kept_both,
        }
    }

    /// The counts as one line; `None` when the rule decided nothing.
    pub(super) fn line(self) -> Option<String> {
        let mut progress = TransferProgress::new(crate::transfer::TransferKind::Local, "", 0, 0);
        progress.replaced = self.replaced;
        progress.skipped_equal = self.skipped_equal;
        progress.kept_newer = self.kept_newer;
        progress.kept_both = self.kept_both;
        crate::transfer::update_summary(&progress)
    }
}
