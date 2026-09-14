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
pub(super) use budget::MAX_RETAINED_FILES_PER_DIRECTORY;
use budget::{AnalyticsBudget, Retention};
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
pub fn scan(root: &Path, p: &Progress) -> ScanOutcome {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let root = crate::analytics::os::normalize_scan_root(root);
    let threads = local_scan_threads();
    let diagnostics = Diagnostics::default();
    let budget = AnalyticsBudget::default();
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
        budget: &budget,
        // This also makes a failed pool creation genuinely serial: recursive
        // work must not silently escape into Rayon's global pool.
        parallel: pool.is_some(),
    };
    let visit = || scan_dir(&traversal, &root, name.into_boxed_str(), 0, true);
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

fn scan_dir(
    traversal: &Traversal<'_>,
    dir: &Path,
    name: Box<str>,
    depth: u32,
    is_root: bool,
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
        scan_entries(traversal, dir, name, read_directory(dir), depth, is_root)
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
    let mut subdirs: Vec<(PathBuf, Box<str>, Retention)> = Vec::new();
    let mut files: Vec<(Box<str>, u64)> = Vec::new();
    let mut own_files = 0u64;
    let mut own_bytes = 0u64;
    let mut aggregated_bytes = 0u64;
    let mut aggregated_entries = 0u64;

    match entries {
        Ok(rd) => {
            for entry in rd {
                let ent = match entry {
                    Ok(ent) => ent,
                    Err(error) => {
                        diagnostics.record_io(
                            crate::analytics::os::display_path(dir),
                            &error,
                            false,
                        );
                        continue;
                    }
                };
                if p.cancel.load(Ordering::Relaxed) {
                    break;
                }
                if matches!(ent.kind, EntryKind::Link | EntryKind::Other) {
                    continue;
                }
                let nm: Box<str> = ent.name.to_string_lossy().into_owned().into_boxed_str();
                if ent.kind == EntryKind::Directory {
                    if ent.unreachable {
                        diagnostics.record(
                            format!(
                                "{}{}{}",
                                crate::analytics::os::display_path(dir),
                                std::path::MAIN_SEPARATOR,
                                nm
                            ),
                            "Ordnername ist nicht als Pfad darstellbar; Inhalt nicht erfasst",
                            false,
                        );
                        continue;
                    }
                    let path = dir.join(&ent.name);
                    if crate::agent_proto::is_pseudo_dir(&path.to_string_lossy()) {
                        continue; // /proc, /sys, … report bogus huge sizes
                    }
                    let retention =
                        budget.claim(&path, depth.saturating_add(1), nm.len() as u64, diagnostics);
                    subdirs.push((path, nm, retention));
                } else if ent.kind == EntryKind::File {
                    own_files += 1;
                    own_bytes = own_bytes.saturating_add(ent.size);
                    files.push((nm, ent.size));
                }
            }
        }
        Err(error) => {
            diagnostics.record_io(crate::analytics::os::display_path(dir), &error, is_root)
        }
    }

    p.files.fetch_add(own_files, Ordering::Relaxed);
    p.bytes.fetch_add(own_bytes, Ordering::Relaxed);
    p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);

    // Retain the largest files individually; fold the rest of a huge
    // directory into one aggregate node so totals stay exact.
    if files.len() > MAX_RETAINED_FILES_PER_DIRECTORY {
        files.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    }
    let mut file_nodes: Vec<SizeNode> =
        Vec::with_capacity(files.len().min(MAX_RETAINED_FILES_PER_DIRECTORY));
    for (index, (file_name, size)) in files.into_iter().enumerate() {
        let retained = index < MAX_RETAINED_FILES_PER_DIRECTORY
            && budget.claim(
                &dir.join(&*file_name),
                depth.saturating_add(1),
                file_name.len() as u64,
                diagnostics,
            ) == Retention::Keep;
        if retained {
            file_nodes.push(SizeNode {
                name: file_name,
                size,
                is_dir: false,
                children: Vec::new(),
            });
        } else {
            aggregated_bytes = aggregated_bytes.saturating_add(size);
            aggregated_entries += 1;
        }
    }
    if aggregated_entries > 0 {
        diagnostics.count_aggregated_files(aggregated_entries);
    }

    // Recurse in parallel. A serial fallback for tiny lists avoids rayon
    // overhead on leaf-heavy trees.
    let visit = |(path, name, retention): (PathBuf, Box<str>, Retention)| {
        (
            scan_dir(traversal, &path, name, depth.saturating_add(1), false),
            retention,
        )
    };
    let visited: Vec<(SizeNode, Retention)> = if p.cancel.load(Ordering::Relaxed) {
        Vec::new()
    } else if traversal.parallel && subdirs.len() > 1 {
        subdirs.into_par_iter().map(visit).collect()
    } else {
        subdirs.into_iter().map(visit).collect()
    };

    let mut size = own_bytes;
    let mut dir_nodes = Vec::with_capacity(visited.len());
    for (node, retention) in visited {
        size = size.saturating_add(node.size);
        match retention {
            Retention::Keep => dir_nodes.push(node),
            Retention::Aggregate => {
                aggregated_bytes = aggregated_bytes.saturating_add(node.size);
                aggregated_entries += 1;
            }
        }
    }
    let mut children = Vec::with_capacity(dir_nodes.len() + file_nodes.len() + 1);
    children.append(&mut dir_nodes);
    children.append(&mut file_nodes);
    if aggregated_entries > 0 {
        children.push(SizeNode {
            name: format!("… {aggregated_entries} weitere Eintraege").into_boxed_str(),
            size: aggregated_bytes,
            is_dir: false,
            children: Vec::new(),
        });
    }
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
