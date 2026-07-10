use super::{AnalyticsBudget, Diagnostics, Progress, ScanOutcome, SizeNode};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::atomic::Ordering;

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
    let tree = if backend.parallelism() <= 1 {
        scan_serial(
            backend,
            root,
            name.into_boxed_str(),
            progress,
            &diagnostics,
            &budget,
            0,
            true,
        )
    } else {
        scan_parallel(backend, root, name, progress, &diagnostics, &budget)
    };
    diagnostics.finish(tree, progress.cancel.load(Ordering::Relaxed))
}

pub(super) struct ChildMeta {
    pub(super) name: String,
    pub(super) is_dir: bool,
    pub(super) size: u64,
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

fn collect_children(
    backend: &dyn crate::vfs::Backend,
    directory: &str,
    depth: u32,
    diagnostics: &Diagnostics,
    budget: &AnalyticsBudget,
) -> crate::vfs::VfsResult<Vec<ChildMeta>> {
    let mut children = Vec::new();
    let mut names = HashSet::new();
    for metadata in backend.list_dir(directory)? {
        crate::vfs::validate_child_name(&metadata.name)?;
        if !names.insert(metadata.name.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "backend returned duplicate child name in {directory}: {:?}",
                    metadata.name
                ),
            ));
        }
        if metadata.is_symlink {
            continue;
        }
        let path = child_path(directory, &metadata.name);
        if !budget.claim(
            Path::new(&path),
            depth.saturating_add(1),
            path.len().saturating_add(metadata.name.len()) as u64,
            diagnostics,
        ) {
            break;
        }
        children.push(ChildMeta {
            name: metadata.name,
            is_dir: metadata.is_dir,
            size: metadata.size,
        });
    }
    Ok(children)
}

fn scan_parallel(
    backend: &dyn crate::vfs::Backend,
    root: &str,
    name: String,
    progress: &Progress,
    diagnostics: &Diagnostics,
    budget: &AnalyticsBudget,
) -> SizeNode {
    let pool = match rayon::ThreadPoolBuilder::new()
        .num_threads(backend.parallelism().clamp(2, 16))
        .build()
    {
        Ok(pool) => pool,
        Err(_) => {
            return scan_serial(
                backend,
                root,
                name.into_boxed_str(),
                progress,
                diagnostics,
                budget,
                0,
                true,
            )
        }
    };
    let root = normalized(root);
    let mut listings = HashMap::new();
    let mut frontier = vec![(root.clone(), 0u32)];
    while !frontier.is_empty() && !progress.cancel.load(Ordering::Relaxed) && !budget.stopped() {
        let level = pool.install(|| {
            frontier
                .par_iter()
                .map(|(directory, depth)| {
                    (
                        directory.clone(),
                        *depth,
                        collect_children(backend, directory, *depth, diagnostics, budget),
                    )
                })
                .collect::<Vec<_>>()
        });
        let mut next = Vec::new();
        let (mut files, mut dirs, mut bytes) = (0u64, 0u64, 0u64);
        for (directory, depth, result) in level {
            let children = match result {
                Ok(children) => children,
                Err(error) => {
                    diagnostics.record(&directory, error.to_string(), directory == root);
                    continue;
                }
            };
            for child in &children {
                if child.is_dir {
                    let path = child_path(&directory, &child.name);
                    if !crate::agent_proto::is_pseudo_dir(&path) {
                        dirs = dirs.saturating_add(1);
                        next.push((path, depth.saturating_add(1)));
                    }
                } else {
                    files = files.saturating_add(1);
                    bytes = bytes.saturating_add(child.size);
                }
            }
            listings.insert(directory, children);
        }
        progress.files.fetch_add(files, Ordering::Relaxed);
        progress.dirs.fetch_add(dirs, Ordering::Relaxed);
        progress.bytes.fetch_add(bytes, Ordering::Relaxed);
        frontier = next;
    }
    build_from_listings(&root, name.into_boxed_str(), &listings)
}

pub(super) fn build_from_listings(
    path: &str,
    name: Box<str>,
    listings: &HashMap<String, Vec<ChildMeta>>,
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

#[allow(clippy::too_many_arguments)]
fn scan_serial(
    backend: &dyn crate::vfs::Backend,
    directory: &str,
    name: Box<str>,
    progress: &Progress,
    diagnostics: &Diagnostics,
    budget: &AnalyticsBudget,
    depth: u32,
    is_root: bool,
) -> SizeNode {
    if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
        return SizeNode {
            name,
            size: 0,
            is_dir: true,
            children: Vec::new(),
        };
    }
    let listed = match collect_children(backend, directory, depth, diagnostics, budget) {
        Ok(listed) => listed,
        Err(error) => {
            diagnostics.record(directory, error.to_string(), is_root);
            Vec::new()
        }
    };
    let mut children = Vec::with_capacity(listed.len());
    let mut size = 0u64;
    let (mut files, mut dirs, mut bytes) = (0u64, 0u64, 0u64);
    for child in listed {
        if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
            break;
        }
        if child.is_dir {
            let path = child_path(directory, &child.name);
            if crate::agent_proto::is_pseudo_dir(&path) {
                continue;
            }
            dirs = dirs.saturating_add(1);
            let node = scan_serial(
                backend,
                &path,
                child.name.into_boxed_str(),
                progress,
                diagnostics,
                budget,
                depth.saturating_add(1),
                false,
            );
            size = size.saturating_add(node.size);
            children.push(node);
        } else {
            files = files.saturating_add(1);
            bytes = bytes.saturating_add(child.size);
            size = size.saturating_add(child.size);
            children.push(SizeNode {
                name: child.name.into_boxed_str(),
                size: child.size,
                is_dir: false,
                children: Vec::new(),
            });
        }
    }
    progress.files.fetch_add(files, Ordering::Relaxed);
    progress.dirs.fetch_add(dirs, Ordering::Relaxed);
    progress.bytes.fetch_add(bytes, Ordering::Relaxed);
    SizeNode {
        name,
        size,
        is_dir: true,
        children,
    }
}
