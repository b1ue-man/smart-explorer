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
use rayon::prelude::*;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[path = "analytics_backend.rs"]
mod backend;
#[path = "analytics_budget.rs"]
mod budget;
#[path = "analytics_outcome.rs"]
mod outcome;
pub use backend::scan_backend;
use budget::AnalyticsBudget;
use outcome::Diagnostics;
pub use outcome::{ScanIssue, ScanOutcome, ScanStatus};

/// One node of the size tree. `name` is this node's own segment, never the full
/// path; `size` is recursive (subtree total) for a directory and the file size
/// for a file. `children` is empty for files.
pub struct SizeNode {
    pub name: Box<str>,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<SizeNode>,
}

/// Shared live progress + cancellation for a running scan.
#[derive(Clone, Default)]
pub struct Progress {
    pub files: Arc<AtomicU64>,
    pub dirs: Arc<AtomicU64>,
    pub bytes: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
}

/// Scan `root` into a size tree, updating `p` live. Parallel traversal is used
/// only when the OS confirms that moving work preserves the caller's authority.
pub fn scan(root: &Path, p: &Progress) -> ScanOutcome {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let threads = local_scan_threads();
    let diagnostics = Diagnostics::default();
    let budget = AnalyticsBudget::default();
    let _ = budget.claim(root, 0, name.len() as u64, &diagnostics);
    let pool = if threads > 1 && parallel_scan_allowed() {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .ok()
    } else {
        None
    };
    let traversal = Traversal {
        progress: p,
        diagnostics: &diagnostics,
        budget: &budget,
        // This also makes a failed pool creation genuinely serial: recursive
        // work must not silently escape into Rayon's global pool.
        parallel: pool.is_some(),
    };
    let visit = || scan_dir(&traversal, root, name.into_boxed_str(), 0, true);
    let tree = match pool {
        Some(pool) => pool.install(visit),
        None => visit(),
    };
    diagnostics.finish(tree, p.cancel.load(Ordering::Relaxed))
}

fn local_scan_threads() -> usize {
    std::env::var("SMART_EXPLORER_ANALYTICS_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, 4)
}

struct Traversal<'a> {
    progress: &'a Progress,
    diagnostics: &'a Diagnostics,
    budget: &'a AnalyticsBudget,
    parallel: bool,
}

fn scan_dir(
    traversal: &Traversal<'_>,
    dir: &Path,
    name: Box<str>,
    depth: u32,
    is_root: bool,
) -> SizeNode {
    if traversal.progress.cancel.load(Ordering::Relaxed) || traversal.budget.stopped() {
        return SizeNode {
            name,
            size: 0,
            is_dir: true,
            children: Vec::new(),
        };
    }
    scan_entries(traversal, dir, name, read_directory(dir), depth, is_root)
}

fn scan_entries(
    traversal: &Traversal<'_>,
    dir: &Path,
    name: Box<str>,
    entries: io::Result<impl Iterator<Item = io::Result<LocalEntry>>>,
    depth: u32,
    is_root: bool,
) -> SizeNode {
    let p = traversal.progress;
    let diagnostics = traversal.diagnostics;
    let budget = traversal.budget;
    let mut subdirs: Vec<(PathBuf, Box<str>)> = Vec::new();
    let mut files: Vec<SizeNode> = Vec::new();
    let mut own_files = 0u64;
    let mut own_bytes = 0u64;

    match entries {
        Ok(rd) => {
            for entry in rd {
                let ent = match entry {
                    Ok(ent) => ent,
                    Err(error) => {
                        diagnostics.record_io(dir.to_string_lossy().into_owned(), &error, false);
                        continue;
                    }
                };
                if p.cancel.load(Ordering::Relaxed) {
                    break;
                }
                if matches!(ent.kind, EntryKind::Link | EntryKind::Other) {
                    continue;
                }
                let path = dir.join(&ent.name);
                let nm: Box<str> = ent.name.to_string_lossy().into_owned().into_boxed_str();
                if !budget.claim(&path, depth.saturating_add(1), nm.len() as u64, diagnostics) {
                    break;
                }
                if ent.kind == EntryKind::Directory {
                    let cp = path;
                    if crate::agent_proto::is_pseudo_dir(&cp.to_string_lossy()) {
                        continue; // /proc, /sys, … report bogus huge sizes
                    }
                    subdirs.push((cp, nm));
                } else if ent.kind == EntryKind::File {
                    let sz = ent.size;
                    own_files += 1;
                    own_bytes = own_bytes.saturating_add(sz);
                    files.push(SizeNode {
                        name: nm,
                        size: sz,
                        is_dir: false,
                        children: Vec::new(),
                    });
                }
            }
        }
        Err(error) => diagnostics.record_io(dir.to_string_lossy().into_owned(), &error, is_root),
    }

    p.files.fetch_add(own_files, Ordering::Relaxed);
    p.bytes.fetch_add(own_bytes, Ordering::Relaxed);
    p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);

    // Recurse in parallel. A serial fallback for tiny lists avoids rayon
    // overhead on leaf-heavy trees.
    let visit = |(path, name): (PathBuf, Box<str>)| {
        scan_dir(traversal, &path, name, depth.saturating_add(1), false)
    };
    let mut dir_nodes: Vec<SizeNode> = if p.cancel.load(Ordering::Relaxed) || budget.stopped() {
        Vec::new()
    } else if traversal.parallel && subdirs.len() > 1 {
        subdirs.into_par_iter().map(visit).collect()
    } else {
        subdirs.into_iter().map(visit).collect()
    };

    let mut size = own_bytes;
    for d in &dir_nodes {
        size = size.saturating_add(d.size);
    }
    let mut children = Vec::with_capacity(dir_nodes.len() + files.len());
    children.append(&mut dir_nodes);
    children.append(&mut files);
    SizeNode {
        name,
        size,
        is_dir: true,
        children,
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
use backend::{build_from_listings, ChildMeta};
#[cfg(test)]
#[path = "analytics_tests.rs"]
mod tests;
