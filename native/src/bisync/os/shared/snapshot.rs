use crate::transfer::flow_for;
use crate::vfs::Backend;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use super::snapshot_dir::WalkContext;
pub use super::snapshot_hash::HashMode;
pub(super) use super::snapshot_hash::{hash_mode, md5_hex_to_u64, md5_to_u64};
use super::snapshot_walk::walk_tree;
use super::sync_flows::next_job;
use super::sync_overload::Progress;
use super::types::{Baseline, Tree};

pub(super) const MAX_WALK_NODES: u64 = 1_000_000;
pub(super) const MAX_WALK_TEXT_BYTES: u64 = 128 * 1024 * 1024;

/// What to skip while walking: hidden files, ignore globs (matched on the
/// relative path), and size/age bounds (Group G). A bound of 0 means "no limit".
pub struct WalkFilter<'a> {
    pub include_hidden: bool,
    pub ignore: &'a globset::GlobSet,
    /// Only include files with `min_size <= size <= max_size` (bytes; 0 = off).
    pub min_size: u64,
    pub max_size: u64,
    /// Only include files modified within `[after_mtime_ms, before_mtime_ms]`
    /// (unix ms; 0 = off on that side).
    pub after_mtime_ms: i64,
    pub before_mtime_ms: i64,
}

impl<'a> WalkFilter<'a> {
    pub(super) fn ignored(&self, relative: &str, directory_like: bool) -> bool {
        self.ignore.is_match(relative)
            || (directory_like && self.ignore.is_match(format!("{relative}/")))
    }

    /// A filter with no size/age bounds (the common case).
    pub fn basic(include_hidden: bool, ignore: &'a globset::GlobSet) -> Self {
        WalkFilter {
            include_hidden,
            ignore,
            min_size: 0,
            max_size: 0,
            after_mtime_ms: 0,
            before_mtime_ms: 0,
        }
    }

    /// Does a file of this size/mtime pass the size & age bounds?
    pub(super) fn size_age_ok(&self, size: u64, mtime_ms: i64) -> bool {
        if self.min_size > 0 && size < self.min_size {
            return false;
        }
        if self.max_size > 0 && size > self.max_size {
            return false;
        }
        if self.after_mtime_ms > 0 && mtime_ms < self.after_mtime_ms {
            return false;
        }
        if self.before_mtime_ms > 0 && mtime_ms > self.before_mtime_ms {
            return false;
        }
        true
    }
}

/// An empty filter (include everything) — handy for tests / "no settings".
pub fn empty_globset() -> globset::GlobSet {
    globset::GlobSetBuilder::new().build().unwrap()
}

/// One side's last-known tree (rel → Sig) reconstructed from the saved baseline,
/// used by `walk_files` to reuse stored hashes for files whose size+mtime are
/// unchanged (so a large local tree isn't re-hashed on every run).
pub(super) fn prev_side(base: &Baseline, side_a: bool) -> Tree {
    base.iter()
        .filter_map(|(rel, (a, b))| (if side_a { *a } else { *b }).map(|s| (rel.clone(), s)))
        .collect()
}

/// Folders are listed concurrently: a remote side as many at once as its
/// connection's flow allows (listings may use the flow's reserved slot), a
/// local side on `parallelism()` threads (all cores) as before.
///
/// `hash` chooses the content-hash strategy (see `HashMode`). `prev` is the
/// previous run's tree for THIS side (from the saved baseline): when a file's
/// size+mtime are unchanged from `prev` we reuse its stored hash instead of
/// re-reading the file — so re-hashing a large local tree every sync is avoided.
pub fn walk_files(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
) -> io::Result<Tree> {
    walk_files_impl(be, root, cancel, filter, hash, prev, false, None, None)
}

/// Mirror destinations on ID-addressed providers may contain pre-existing
/// duplicate regular-file names. The caller must preflight and apply an exact
/// dedupe plan before any path-based writes; this walk only selects the same
/// deterministic newest ID for planning.
pub(super) fn walk_files_with_duplicate_files(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
) -> io::Result<Tree> {
    walk_files_impl(be, root, cancel, filter, hash, prev, true, None, None)
}

pub(super) struct Snapshot {
    pub tree: Tree,
    pub omissions: super::omissions::SyncOmissions,
    pub duplicates: super::snapshot_duplicates::DuplicateGroups,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn walk_snapshot(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
    allow_duplicate_files: bool,
    fold_case: bool,
) -> io::Result<Snapshot> {
    let omissions = Mutex::new(super::omissions::SyncOmissions::new(fold_case));
    let duplicates = Mutex::new(super::snapshot_duplicates::DuplicateGroups::new());
    let tree = walk_files_impl(
        be,
        root,
        cancel,
        filter,
        hash,
        prev,
        allow_duplicate_files,
        Some(&omissions),
        Some(&duplicates),
    )?;
    Ok(Snapshot {
        tree,
        omissions: omissions.into_inner().unwrap_or_else(|e| e.into_inner()),
        duplicates: duplicates.into_inner().unwrap_or_else(|e| e.into_inner()),
    })
}

#[allow(clippy::too_many_arguments)]
fn walk_files_impl(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
    allow_duplicate_files: bool,
    omissions: Option<&Mutex<super::omissions::SyncOmissions>>,
    duplicates: Option<&Mutex<super::snapshot_duplicates::DuplicateGroups>>,
) -> io::Result<Tree> {
    let canceled = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "synchronization tree walk canceled",
        )
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    // Fast path: when the backend can produce the signature SERVER-SIDE (the SSH
    // agent's WalkHashed), get the whole tree — including content MD5 for Full —
    // in one pass without downloading a single file. Falls through to the per-dir
    // walk if it didn't run.
    if be.supports_walk_hashed() && !be.has_duplicate_file_names() {
        if let Some(tree) =
            super::snapshot_agent::walk_hashed_via_agent(be, root, cancel, filter, hash)?
        {
            return Ok(tree);
        }
    }

    let flow = (!be.is_local()).then(|| flow_for(be, root));
    let context = WalkContext {
        be,
        root,
        cancel,
        filter,
        hash,
        prev,
        allow_duplicate_files,
        omissions,
        duplicates,
        nodes: AtomicU64::new(1),
        text_bytes: AtomicU64::new(root.len() as u64),
        reads: flow.clone().map(|flow| (flow, next_job())),
        progress: Progress::default(),
    };
    walk_tree(&context, flow, be.parallelism().max(1))
}

#[cfg(test)]
#[path = "snapshot_walk_tests.rs"]
mod walk_tests;
