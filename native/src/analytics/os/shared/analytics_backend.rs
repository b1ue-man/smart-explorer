//! Storage analysis through any VFS backend, for remotes without an own
//! analysis on the other side (SFTP/FTP/WebDAV/SMB/Drive, older Share peers):
//! one listing per folder, in parallel up to the backend's width with work
//! stealing (no barrier per level), each folder assembled as soon as its
//! subfolders are done, and the retention budget honoured like in the local
//! scan – detail beyond it folds into the parent's aggregate, sizes and
//! counts stay exact. One unusable entry never costs its whole folder.
use super::MAX_RETAINED_FILES_PER_DIRECTORY;
use super::{AnalyticsBudget, Diagnostics, Progress, Retention, ScanOutcome, SizeNode};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::Ordering;

/// More concurrent listings than this never sped up one remote connection.
const MAX_LISTERS: usize = 16;

/// Scan a remote tree through any VFS backend with bounded retained state.
pub fn scan_backend(
    backend: &dyn crate::vfs::Backend,
    root: &str,
    progress: &Progress,
) -> ScanOutcome {
    let name = root
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(root)
        .to_string();
    let diagnostics = Diagnostics::default();
    let budget = AnalyticsBudget::default();
    let _ = budget.claim(Path::new(root), 0, name.len() as u64, &diagnostics);
    let threads = backend.parallelism().clamp(1, MAX_LISTERS);
    let pool = (threads > 1)
        .then(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .stack_size(super::SCAN_THREAD_STACK_BYTES)
                .build()
                .ok()
        })
        .flatten();
    let walker = Walker {
        backend,
        progress,
        diagnostics: &diagnostics,
        budget: &budget,
        // A failed pool stays serial instead of using Rayon's global pool.
        parallel: pool.is_some(),
    };
    let root = normalized(root);
    let visit = || walker.dir(&root, name.into_boxed_str(), 0, true);
    let tree = match pool {
        Some(pool) => pool.install(visit),
        None => visit(),
    };
    diagnostics.finish(tree, progress.cancel.load(Ordering::Relaxed))
}

fn normalized(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn child_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    }
}

fn empty_dir(name: Box<str>) -> SizeNode {
    SizeNode {
        name,
        size: 0,
        is_dir: true,
        children: Vec::new(),
    }
}

/// One folder's listing: subfolders to visit with their retention, files
/// with their sizes, and what was counted without an own node.
#[derive(Default)]
struct Listing {
    subdirs: Vec<(String, Box<str>, Retention)>,
    files: Vec<(Box<str>, u64)>,
    own_bytes: u64,
    aggregated_bytes: u64,
    aggregated_entries: u64,
}

struct Walker<'a> {
    backend: &'a dyn crate::vfs::Backend,
    progress: &'a Progress,
    diagnostics: &'a Diagnostics,
    budget: &'a AnalyticsBudget,
    parallel: bool,
}

impl Walker<'_> {
    fn dir(&self, path: &str, name: Box<str>, depth: u32, is_root: bool) -> SizeNode {
        if self.progress.cancel.load(Ordering::Relaxed) {
            return empty_dir(name);
        }
        if !self.budget.depth_allowed(depth) {
            self.diagnostics.record(
                path,
                format!(
                    "Verzeichnistiefe ueber {} wird nicht weiter erfasst",
                    self.budget.max_depth()
                ),
                is_root,
            );
            return empty_dir(name);
        }
        let mut listing = match self.list(path, depth) {
            Ok(listing) => listing,
            Err(error) => {
                self.diagnostics.record_io(path, &error, is_root);
                return empty_dir(name);
            }
        };
        let file_nodes = self.retain_files(path, depth, &mut listing);
        let subdirs = std::mem::take(&mut listing.subdirs);
        let visit = |(child, child_name, retention): (String, Box<str>, Retention)| {
            (
                self.dir(&child, child_name, depth.saturating_add(1), false),
                retention,
            )
        };
        let visited: Vec<(SizeNode, Retention)> = if self.progress.cancel.load(Ordering::Relaxed) {
            Vec::new()
        } else if self.parallel && subdirs.len() > 1 {
            subdirs.into_par_iter().map(visit).collect()
        } else {
            subdirs.into_iter().map(visit).collect()
        };
        let mut size = listing.own_bytes;
        let mut children = Vec::with_capacity(visited.len() + file_nodes.len() + 1);
        for (node, retention) in visited {
            size = size.saturating_add(node.size);
            match retention {
                Retention::Keep => children.push(node),
                Retention::Aggregate => {
                    listing.aggregated_bytes = listing.aggregated_bytes.saturating_add(node.size);
                    listing.aggregated_entries += 1;
                }
            }
        }
        children.extend(file_nodes);
        if listing.aggregated_entries > 0 {
            children.push(SizeNode {
                name: crate::analytics::aggregate_name(listing.aggregated_entries).into_boxed_str(),
                size: listing.aggregated_bytes,
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

    /// Lists `path`. Links, special files and the app trash are left out; an
    /// entry whose name cannot form a path is counted (a file) or reported (a
    /// folder, whose content stays unknown) instead of failing the folder.
    fn list(&self, path: &str, depth: u32) -> crate::vfs::VfsResult<Listing> {
        self.progress.enter_directory(path);
        let mut listing = Listing::default();
        let (mut files, mut dirs) = (0u64, 0u64);
        let mut dir_names = HashSet::new();
        for metadata in self.backend.list_dir(path)? {
            if self.progress.cancel.load(Ordering::Relaxed) {
                break;
            }
            if metadata.is_symlink
                || metadata.special
                || crate::apptrash::excluded_name(&metadata.name)
            {
                continue;
            }
            let usable = crate::vfs::validate_child_name(&metadata.name).is_ok();
            if !metadata.is_dir {
                files = files.saturating_add(1);
                listing.own_bytes = listing.own_bytes.saturating_add(metadata.size);
                if usable {
                    listing
                        .files
                        .push((metadata.name.into_boxed_str(), metadata.size));
                } else {
                    listing.aggregated_bytes =
                        listing.aggregated_bytes.saturating_add(metadata.size);
                    listing.aggregated_entries += 1;
                }
                continue;
            }
            let child = child_path(path, &metadata.name);
            if !usable {
                self.diagnostics.record(
                    child,
                    "Ordnername ist nicht als Pfad darstellbar; Inhalt nicht erfasst",
                    false,
                );
                continue;
            }
            if !dir_names.insert(metadata.name.clone()) {
                self.diagnostics.record(
                    child,
                    "Ordnername kommt mehrfach vor; nur einmal erfasst",
                    false,
                );
                continue;
            }
            if crate::agent_proto::is_pseudo_dir(&child) {
                continue; // /proc, /sys, … of an SFTP server report bogus sizes
            }
            dirs = dirs.saturating_add(1);
            let retention = self.budget.claim(
                Path::new(&child),
                depth.saturating_add(1),
                metadata.name.len() as u64,
                self.diagnostics,
            );
            listing
                .subdirs
                .push((child, metadata.name.into_boxed_str(), retention));
        }
        self.progress.files.fetch_add(files, Ordering::Relaxed);
        self.progress.dirs.fetch_add(dirs, Ordering::Relaxed);
        self.progress
            .bytes
            .fetch_add(listing.own_bytes, Ordering::Relaxed);
        Ok(listing)
    }

    /// Keeps the largest files of a folder as own nodes while the budget
    /// allows; the rest joins the folder's aggregate with its exact size.
    fn retain_files(&self, path: &str, depth: u32, listing: &mut Listing) -> Vec<SizeNode> {
        let mut files = std::mem::take(&mut listing.files);
        if files.len() > MAX_RETAINED_FILES_PER_DIRECTORY {
            files.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        }
        let mut nodes = Vec::with_capacity(files.len().min(MAX_RETAINED_FILES_PER_DIRECTORY));
        let mut folded = 0u64;
        for (index, (name, size)) in files.into_iter().enumerate() {
            let retained = index < MAX_RETAINED_FILES_PER_DIRECTORY
                && self.budget.claim(
                    Path::new(&child_path(path, &name)),
                    depth.saturating_add(1),
                    name.len() as u64,
                    self.diagnostics,
                ) == Retention::Keep;
            if retained {
                nodes.push(SizeNode {
                    name,
                    size,
                    is_dir: false,
                    children: Vec::new(),
                });
            } else {
                listing.aggregated_bytes = listing.aggregated_bytes.saturating_add(size);
                listing.aggregated_entries += 1;
                folded += 1;
            }
        }
        if folded > 0 {
            self.diagnostics.count_aggregated_files(folded);
        }
        nodes
    }
}

/// Tree assembly from complete listings (the former level-by-level walk);
/// kept for its test until that test moves to the walker above.
#[cfg(test)]
pub(super) struct ChildMeta {
    pub(super) name: String,
    pub(super) is_dir: bool,
    pub(super) size: u64,
}

#[cfg(test)]
pub(super) fn build_from_listings(
    path: &str,
    name: Box<str>,
    listings: &std::collections::HashMap<String, Vec<ChildMeta>>,
) -> SizeNode {
    let mut children = Vec::new();
    let mut size = 0u64;
    if let Some(listed) = listings.get(path) {
        for child in listed.iter().filter(|child| child.is_dir) {
            let node = build_from_listings(
                &child_path(path, &child.name),
                child.name.clone().into_boxed_str(),
                listings,
            );
            size = size.saturating_add(node.size);
            children.push(node);
        }
        for child in listed.iter().filter(|child| !child.is_dir) {
            size = size.saturating_add(child.size);
            children.push(SizeNode {
                name: child.name.clone().into_boxed_str(),
                size: child.size,
                is_dir: false,
                children: Vec::new(),
            });
        }
    }
    SizeNode {
        name,
        size,
        is_dir: true,
        children,
    }
}

#[cfg(test)]
#[path = "analytics_backend_tests.rs"]
mod tests;
