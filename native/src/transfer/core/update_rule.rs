//! The file rule "newer replaces older" for same-named files (`Conflict::Update`):
//! the pure decision from size and modification time of both sides, the
//! tolerance window and the German summary every surface shows.
//!
//! The rule, for a file that exists on both sides:
//! - equal size and times at most the window apart: equal, skipped;
//! - the source is newer by more than the window: it replaces the target;
//! - the target is newer by more than the window: the target stays ("Ziel
//!   neuer");
//! - times within the window but different sizes: a conflict, both are kept;
//! - a time that one side does not know (0): nothing orders the two files and
//!   equal sizes do not prove equal contents, so it is a conflict and both
//!   are kept (nothing is skipped, nothing is replaced).
//!
//! A replacement is bound to the target it was decided against: right before
//! it is published the target's size and time are compared with that stamp
//! again (`Replacing::holds`), and a target that changed meanwhile keeps both.
//!
//! A file that is missing at the target is inserted; that case needs no
//! comparison and is decided by the engine (`engine/update.rs`).
use super::types::TransferProgress;
use crate::vfs::MtimePrecision;

/// The tolerance when a side does not know how exactly it stores times.
pub(crate) const UNKNOWN_WINDOW_MS: u64 = 2_000;

/// Size and modification time (Unix milliseconds) of one file. A time of 0
/// means "unknown" (backends that cannot report it leave the field empty).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub size: u64,
    pub mtime_ms: i64,
}

/// How a source file and its same-named target relate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    /// Same size, times within the window: skipped.
    Equal,
    /// The source is newer by more than the window: replaces the target.
    SourceNewer,
    /// The target is newer by more than the window: kept.
    TargetNewer,
    /// Same time within the window and another size, or a time that one side
    /// does not know: both are kept.
    Conflict,
}

/// The window: the coarser time resolution of the two sides, a side that
/// does not know its resolution counting as two seconds (FAT).
pub(crate) fn window_ms(source: MtimePrecision, target: MtimePrecision) -> u64 {
    let step = |precision: MtimePrecision| precision.step_ms().unwrap_or(UNKNOWN_WINDOW_MS);
    step(source).max(step(target))
}

/// Compares a source file with its same-named target.
pub(crate) fn classify(source: Stamp, target: Stamp, window_ms: u64) -> Case {
    let comparable = source.mtime_ms != 0 && target.mtime_ms != 0;
    if !comparable {
        // No time to order them by, and equal sizes do not make equal files:
        // a changed source of the same size must still reach the target, or
        // be kept beside it. Keeping both destroys nothing.
        return Case::Conflict;
    }
    let delta = i128::from(source.mtime_ms) - i128::from(target.mtime_ms);
    if delta.unsigned_abs() <= u128::from(window_ms) {
        same_or_conflict(source, target)
    } else if delta > 0 {
        Case::SourceNewer
    } else {
        Case::TargetNewer
    }
}

/// Whether a target still holds what the decision saw: the same size and a
/// time within the window of the stamp. A time that is unknown on either side
/// proves nothing and does not hold.
pub(crate) fn unchanged(decided: Stamp, now: Stamp, window_ms: u64) -> bool {
    if decided.size != now.size || decided.mtime_ms == 0 || now.mtime_ms == 0 {
        return false;
    }
    let delta = i128::from(decided.mtime_ms) - i128::from(now.mtime_ms);
    delta.unsigned_abs() <= u128::from(window_ms)
}

/// The existing target a file replaces, as the rule saw it when it decided:
/// the provider id of duplicate-capable providers (`None` elsewhere), its
/// size and time, and the window the times were compared with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Replacing {
    pub id: Option<String>,
    pub stamp: Stamp,
    pub window_ms: u64,
}

impl Replacing {
    /// Whether `now` (the target looked at right before publishing) is still
    /// the file that was decided against.
    pub(crate) fn holds(&self, now: Stamp) -> bool {
        unchanged(self.stamp, now, self.window_ms)
    }
}

fn same_or_conflict(source: Stamp, target: Stamp) -> Case {
    if source.size == target.size {
        Case::Equal
    } else {
        Case::Conflict
    }
}

/// What a finished or running transfer decided by this rule, as one line for
/// the transfer window, the terminal and Android; `None` when nothing was
/// decided by it. Names no file.
pub fn summary(progress: &TransferProgress) -> Option<String> {
    let mut parts = Vec::new();
    if progress.replaced > 0 {
        parts.push(format!("{} neuere ersetzt", progress.replaced));
    }
    if progress.skipped_equal > 0 {
        parts.push(format!("{} gleiche übersprungen", progress.skipped_equal));
    }
    if progress.kept_newer > 0 {
        parts.push(format!("{} Ziel neuer behalten", progress.kept_newer));
    }
    if progress.kept_both > 0 {
        parts.push(format!(
            "{} als Konflikt doppelt behalten",
            progress.kept_both
        ));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

#[cfg(test)]
#[path = "update_rule_tests.rs"]
mod tests;
