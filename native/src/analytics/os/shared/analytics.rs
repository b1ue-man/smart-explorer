//! Lightweight, low-memory recursive size scanner for the storage-analytics
//! view (WizTree-style "where is my space").
//!
//! The main scanner (`scanner.rs`) loads rich per-file metadata (mtime, btime,
//! attributes, extension, backend id, …) into `Arc<str>`-heavy `FileEntry`s —
//! great for the explorer, but it burns RAM and time on million-file trees.
//!
//! Here every node stores ONLY its own NAME (one path segment, not the full
//! path), its size, whether it's a directory, and its children. Full paths are
//! reconstructed by descending from the root (the drill position carries the
//! prefix), so the tree stays compact: roughly `name + ~48 bytes` per node.

use crate::analytics::os::{parallel_scan_allowed, read_directory, EntryKind, LocalEntry};
use crate::analytics::Progress;
use crate::apptrash::ProtectedAreas;
use rayon::prelude::*;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

#[path = "analytics_backend.rs"]
mod backend;
#[path = "analytics_budget.rs"]
mod budget;
#[path = "analytics_outcome.rs"]
mod outcome;
pub use backend::scan_backend;
pub(super) use budget::MAX_RETAINED_FILES_PER_DIRECTORY;
use budget::{AnalyticsBudget, Retention};
use outcome::Diagnostics;
#[path = "analytics_walk.rs"]
mod walk;
pub use outcome::{ScanIssue, ScanOutcome, ScanStatus};
#[cfg(test)]
use walk::scan_entries;
use walk::scan_entries_with;

/// One node of the size tree. `name` is this node's own segment, never the full
/// path; `size` is recursive (subtree total) for a directory and the file size
/// for a file. `children` is empty for files.
pub struct SizeNode {
    pub name: Box<str>,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<SizeNode>,
}

/// Stack reserved for every scan thread: recursion depth is bounded by
/// `MAX_ANALYTICS_DEPTH`, and the reservation is virtual until touched.
pub const SCAN_THREAD_STACK_BYTES: usize = 64 * 1024 * 1024;

/// Scan `root` into a size tree, updating `p` live. Parallel traversal is used
/// only when the OS confirms that moving work preserves the caller's authority.
///
/// Nothing short of cancellation ends the scan early: unreadable entries,
/// unrepresentable names, exhausted retention limits and even a panic inside
/// one directory are recorded and the traversal continues with exact sizes
/// for everything that could be read.
///
/// Other apps' private areas on Android (`Android/data`, `Android/obb`) are
/// walked as far as they are readable; what Android hides there is counted
/// in `ScanOutcome::protected`, never as an issue.
pub fn scan(root: &Path, p: &Progress) -> ScanOutcome {
    scan_with_guard(root, p, None)
}

/// A check every directory passes before it is listed (Share confinement).
pub(crate) type ScanGuard<'a> = &'a (dyn Fn(&Path) -> io::Result<()> + Sync);

pub(crate) fn scan_with_guard(
    root: &Path,
    p: &Progress,
    guard: Option<ScanGuard<'_>>,
) -> ScanOutcome {
    scan_in(root, p, guard, None, None)
}

/// How many tree nodes scans may retain together: the scans of all exports a
/// host merges into one result share one (a peer's `/`), and a receiver may
/// ask for fewer nodes than the host default (its memory).
pub(crate) struct ScanBudget(AnalyticsBudget);

impl ScanBudget {
    /// The host default, or at most `nodes` retained nodes.
    pub(crate) fn with_node_limit(nodes: Option<u64>) -> Self {
        Self(match nodes {
            Some(nodes) => AnalyticsBudget::with_node_limit(nodes),
            None => AnalyticsBudget::default(),
        })
    }
}

/// `scan_with_guard` drawing on a budget shared with other scans.
pub(crate) fn scan_with(
    root: &Path,
    p: &Progress,
    guard: Option<ScanGuard<'_>>,
    budget: &ScanBudget,
) -> ScanOutcome {
    scan_in(root, p, guard, None, Some(&budget.0))
}

/// `protected` replaces the areas derived from the registered volumes.
fn scan_in(
    root: &Path,
    p: &Progress,
    guard: Option<ScanGuard<'_>>,
    protected: Option<ProtectedAreas>,
    shared: Option<&AnalyticsBudget>,
) -> ScanOutcome {
    scan_in_with(root, p, guard, protected, shared, false, &[], None)
}

pub(crate) fn scan_confined(
    root: &Path,
    p: &Progress,
    budget: &ScanBudget,
    excluded: &[PathBuf],
) -> ScanOutcome {
    scan_in_with(root, p, None, None, Some(&budget.0), true, excluded, None)
}

pub(crate) fn scan_confined_with(
    root: &Path,
    p: &Progress,
    budget: &ScanBudget,
    excluded: &[PathBuf],
    handle: crate::local_access::DirectoryHandle,
) -> ScanOutcome {
    scan_in_with(
        root,
        p,
        None,
        None,
        Some(&budget.0),
        true,
        excluded,
        Some(handle),
    )
}

fn scan_in_with(
    root: &Path,
    p: &Progress,
    guard: Option<ScanGuard<'_>>,
    protected: Option<ProtectedAreas>,
    shared: Option<&AnalyticsBudget>,
    confined: bool,
    excluded: &[PathBuf],
    provided: Option<crate::local_access::DirectoryHandle>,
) -> ScanOutcome {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let root = crate::analytics::os::normalize_scan_root(root);
    let threads = local_scan_threads();
    let protected = protected.unwrap_or_else(|| ProtectedAreas::for_walk(&root));
    let diagnostics = Diagnostics::with_protected(protected);
    let own;
    let budget = match shared {
        Some(shared) => shared,
        None => {
            own = AnalyticsBudget::for_progress(p);
            &own
        }
    };
    let _ = budget.claim(&root, 0, name.len() as u64, &diagnostics);
    let pool = if threads > 1 && parallel_scan_allowed() {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .stack_size(SCAN_THREAD_STACK_BYTES)
            .build()
            .ok()
    } else {
        None
    };
    let traversal = Traversal {
        progress: p,
        diagnostics: &diagnostics,
        budget,
        // This also makes a failed pool creation genuinely serial: recursive
        // work must not silently escape into Rayon's global pool.
        parallel: pool.is_some(),
        guard,
    };
    let handle = confined.then(|| {
        provided
            .map(Ok)
            .unwrap_or_else(|| crate::local_access::DirectoryHandle::open_root(&root))
    });
    let visit = || match &handle {
        Some(Err(error)) => {
            diagnostics.dir_failed(&root, error, true);
            empty_dir(name.into_boxed_str())
        }
        Some(Ok(handle)) => scan_dir_with(
            &traversal,
            &root,
            name.into_boxed_str(),
            0,
            true,
            Some(handle),
            excluded,
        ),
        None => scan_dir_with(
            &traversal,
            &root,
            name.into_boxed_str(),
            0,
            true,
            None,
            excluded,
        ),
    };
    let tree = match pool {
        Some(pool) => pool.install(visit),
        None => visit(),
    };
    diagnostics.finish(tree, p.cancel.load(Ordering::Relaxed))
}

/// Worker threads of a local scan: `SMART_EXPLORER_ANALYTICS_THREADS`, else
/// the platform default (Android: one per core up to 4; desktop: 2).
pub(crate) fn local_scan_threads() -> usize {
    std::env::var("SMART_EXPLORER_ANALYTICS_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(crate::analytics::os::default_scan_threads)
        .clamp(1, 4)
}

struct Traversal<'a> {
    progress: &'a Progress,
    diagnostics: &'a Diagnostics,
    budget: &'a AnalyticsBudget,
    parallel: bool,
    guard: Option<ScanGuard<'a>>,
}

fn empty_dir(name: Box<str>) -> SizeNode {
    SizeNode {
        name,
        size: 0,
        is_dir: true,
        children: Vec::new(),
    }
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unbekannter interner Fehler".to_string()
    }
}

fn scan_dir_with(
    traversal: &Traversal<'_>,
    dir: &Path,
    name: Box<str>,
    depth: u32,
    is_root: bool,
    handle: Option<&crate::local_access::DirectoryHandle>,
    excluded: &[PathBuf],
) -> SizeNode {
    if traversal.progress.cancel.load(Ordering::Relaxed) {
        return empty_dir(name);
    }
    if !traversal.budget.depth_allowed(depth) {
        traversal.diagnostics.record(
            crate::analytics::os::display_path(dir),
            format!(
                "Verzeichnistiefe ueber {} wird nicht weiter erfasst",
                traversal.budget.max_depth()
            ),
            is_root,
        );
        return empty_dir(name);
    }
    // One directory's failure — even an unexpected panic in the platform
    // enumerator — must never take the rest of the scan down with it.
    let fallback_name = name.clone();
    let visit = std::panic::AssertUnwindSafe(|| {
        traversal
            .progress
            .enter_directory(&crate::analytics::os::display_path(dir));
        traversal.diagnostics.entered(dir, is_root);
        if let Some(guard) = traversal.guard {
            if let Err(error) = guard(dir) {
                traversal.diagnostics.dir_failed(dir, &error, is_root);
                return empty_dir(name);
            }
        }
        match handle {
            Some(handle) => scan_entries_with(
                traversal,
                dir,
                name,
                handle.read_directory(),
                depth,
                is_root,
                Some(handle),
                excluded,
            ),
            None => scan_entries_with(
                traversal,
                dir,
                name,
                read_directory(dir),
                depth,
                is_root,
                None,
                excluded,
            ),
        }
    });
    match std::panic::catch_unwind(visit) {
        Ok(node) => node,
        Err(payload) => {
            traversal.diagnostics.record(
                crate::analytics::os::display_path(dir),
                format!("interner Fehler beim Lesen: {}", panic_text(payload)),
                is_root,
            );
            empty_dir(fallback_name)
        }
    }
}

/// Convert a tree computed server-side by the SSH agent (`agent_proto::WireNode`)
/// into the analytics `SizeNode`. Same shape — names only, paths rebuilt on
/// descent — so this is a straight ownership-transferring recursion.
pub fn from_wire(w: crate::agent_proto::WireNode) -> SizeNode {
    SizeNode {
        name: w.name.into_boxed_str(),
        size: w.size,
        is_dir: w.is_dir,
        children: w.children.into_iter().map(from_wire).collect(),
    }
}

#[cfg(test)]
#[path = "analytics_protected_tests.rs"]
mod protected_tests;
#[cfg(test)]
#[path = "analytics_tests.rs"]
mod tests;
