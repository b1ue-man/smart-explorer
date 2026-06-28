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

use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

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

/// Scan `root` into a size tree, updating `p` live. Runs the subdirectories in
/// parallel (rayon work-stealing handles the nested recursion).
pub fn scan(root: &Path, p: &Progress) -> SizeNode {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let threads = local_scan_threads();
    if threads <= 1 {
        scan_dir(root, name.into_boxed_str(), p)
    } else {
        match rayon::ThreadPoolBuilder::new().num_threads(threads).build() {
            Ok(pool) => pool.install(|| scan_dir(root, name.into_boxed_str(), p)),
            Err(_) => scan_dir(root, name.into_boxed_str(), p),
        }
    }
}

fn local_scan_threads() -> usize {
    std::env::var("SMART_EXPLORER_ANALYTICS_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, 4)
}

fn scan_dir(dir: &Path, name: Box<str>, p: &Progress) -> SizeNode {
    if p.cancel.load(Ordering::Relaxed) {
        return SizeNode {
            name,
            size: 0,
            is_dir: true,
            children: Vec::new(),
        };
    }
    let mut subdirs: Vec<(PathBuf, Box<str>)> = Vec::new();
    let mut files: Vec<SizeNode> = Vec::new();
    let mut own_files = 0u64;
    let mut own_bytes = 0u64;

    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            if p.cancel.load(Ordering::Relaxed) {
                break;
            }
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            // Don't follow symlinks/reparse points — avoids cycles and
            // double-counting the same bytes.
            if ft.is_symlink() {
                continue;
            }
            let nm: Box<str> = ent
                .file_name()
                .to_string_lossy()
                .into_owned()
                .into_boxed_str();
            if ft.is_dir() {
                let cp = ent.path();
                if crate::agent_proto::is_pseudo_dir(&cp.to_string_lossy()) {
                    continue; // /proc, /sys, … report bogus huge sizes
                }
                subdirs.push((cp, nm));
            } else if ft.is_file() {
                let sz = ent.metadata().map(|m| m.len()).unwrap_or(0);
                own_files += 1;
                own_bytes += sz;
                files.push(SizeNode {
                    name: nm,
                    size: sz,
                    is_dir: false,
                    children: Vec::new(),
                });
            }
        }
    }

    p.files.fetch_add(own_files, Ordering::Relaxed);
    p.bytes.fetch_add(own_bytes, Ordering::Relaxed);
    p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);

    // Recurse in parallel. A serial fallback for tiny lists avoids rayon
    // overhead on leaf-heavy trees.
    let mut dir_nodes: Vec<SizeNode> = if p.cancel.load(Ordering::Relaxed) {
        Vec::new()
    } else if subdirs.len() > 1 {
        subdirs
            .into_par_iter()
            .map(|(path, nm)| scan_dir(&path, nm, p))
            .collect()
    } else {
        subdirs
            .into_iter()
            .map(|(path, nm)| scan_dir(&path, nm, p))
            .collect()
    };

    let mut size = own_bytes;
    for d in &dir_nodes {
        size += d.size;
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

/// Scan a REMOTE tree through the VFS backend (SFTP/FTP/WebDAV/Drive) into the
/// same compact `SizeNode` output as the local `scan`. Backends that report
/// `parallelism() > 1` (WebDAV, Google Drive) list each tree level concurrently
/// — the dominant latency lever for HTTP backends; serial otherwise. Each
/// directory is one `list_dir` round-trip. Cancellation is checked per level/dir.
pub fn scan_backend(be: &dyn crate::vfs::Backend, root: &str, p: &Progress) -> SizeNode {
    let name = root
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(root)
        .to_string();
    if be.parallelism() <= 1 {
        scan_backend_dir(be, root, name.into_boxed_str(), p)
    } else {
        scan_backend_parallel(be, root, name, p)
    }
}

/// Backend-neutral child entry collected during the parallel walk.
struct ChildMeta {
    name: String,
    is_dir: bool,
    size: u64,
}

/// Normalise a backend dir path so the listing key matches the path rebuilt
/// during tree assembly (trailing slash stripped; root stays "/").
fn norm_dir(s: &str) -> String {
    let t = s.trim_end_matches('/');
    if t.is_empty() {
        "/".to_string()
    } else {
        t.to_string()
    }
}

fn child_path(parent: &str, name: &str) -> String {
    format!("{}/{}", parent.trim_end_matches('/'), name)
}

fn scan_backend_parallel(
    be: &dyn crate::vfs::Backend,
    root: &str,
    name: String,
    p: &Progress,
) -> SizeNode {
    let par = be.parallelism().clamp(2, 16);
    let pool = match rayon::ThreadPoolBuilder::new().num_threads(par).build() {
        Ok(pool) => pool,
        Err(_) => return scan_backend_dir(be, root, name.into_boxed_str(), p),
    };

    let root_norm = norm_dir(root);
    let mut listings: std::collections::HashMap<String, Vec<ChildMeta>> =
        std::collections::HashMap::new();
    let mut frontier: Vec<String> = vec![root_norm.clone()];

    // Breadth-first: list every directory at the current depth in parallel, then
    // descend to the next depth. Bounds concurrency to the pool size.
    while !frontier.is_empty() {
        if p.cancel.load(Ordering::Relaxed) {
            break;
        }
        let level: Vec<(String, Vec<ChildMeta>)> = pool.install(|| {
            frontier
                .par_iter()
                .map(|dir| {
                    if p.cancel.load(Ordering::Relaxed) {
                        return (dir.clone(), Vec::new());
                    }
                    let mut kids = Vec::new();
                    if let Ok(entries) = be.list_dir(dir) {
                        for m in entries {
                            if m.is_symlink {
                                continue;
                            }
                            kids.push(ChildMeta {
                                name: m.name,
                                is_dir: m.is_dir,
                                size: m.size,
                            });
                        }
                    }
                    (dir.clone(), kids)
                })
                .collect()
        });

        let mut next = Vec::new();
        let (mut lvl_files, mut lvl_dirs, mut lvl_bytes) = (0u64, 0u64, 0u64);
        for (dir, kids) in level {
            for c in &kids {
                if c.is_dir {
                    let cp = child_path(&dir, &c.name);
                    if crate::agent_proto::is_pseudo_dir(&cp) {
                        continue; // /proc, /sys, … bogus sizes
                    }
                    lvl_dirs += 1;
                    next.push(cp);
                } else {
                    lvl_files += 1;
                    lvl_bytes += c.size;
                }
            }
            listings.insert(dir, kids);
        }
        p.files.fetch_add(lvl_files, Ordering::Relaxed);
        p.dirs.fetch_add(lvl_dirs, Ordering::Relaxed);
        p.bytes.fetch_add(lvl_bytes, Ordering::Relaxed);
        frontier = next;
    }

    build_from_listings(&root_norm, name.into_boxed_str(), &listings)
}

/// Assemble the `SizeNode` tree from the collected per-directory listings.
fn build_from_listings(
    path: &str,
    name: Box<str>,
    listings: &std::collections::HashMap<String, Vec<ChildMeta>>,
) -> SizeNode {
    let mut children = Vec::new();
    let mut size = 0u64;
    if let Some(kids) = listings.get(path) {
        for c in kids.iter().filter(|c| c.is_dir) {
            let node = build_from_listings(
                &child_path(path, &c.name),
                c.name.clone().into_boxed_str(),
                listings,
            );
            size += node.size;
            children.push(node);
        }
        for c in kids.iter().filter(|c| !c.is_dir) {
            size += c.size;
            children.push(SizeNode {
                name: c.name.clone().into_boxed_str(),
                size: c.size,
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

fn scan_backend_dir(
    be: &dyn crate::vfs::Backend,
    dir: &str,
    name: Box<str>,
    p: &Progress,
) -> SizeNode {
    if p.cancel.load(Ordering::Relaxed) {
        return SizeNode {
            name,
            size: 0,
            is_dir: true,
            children: Vec::new(),
        };
    }
    let mut subdirs: Vec<(String, Box<str>)> = Vec::new();
    let mut files: Vec<SizeNode> = Vec::new();
    let mut own_files = 0u64;
    let mut own_bytes = 0u64;

    if let Ok(entries) = be.list_dir(dir) {
        let base = dir.trim_end_matches('/');
        for m in entries {
            if m.is_symlink {
                continue; // don't follow — avoids cycles + double counting
            }
            if m.is_dir {
                let child = format!("{}/{}", base, m.name);
                if crate::agent_proto::is_pseudo_dir(&child) {
                    continue; // /proc, /sys, … bogus sizes
                }
                subdirs.push((child, m.name.into_boxed_str()));
            } else {
                own_files += 1;
                own_bytes += m.size;
                files.push(SizeNode {
                    name: m.name.into_boxed_str(),
                    size: m.size,
                    is_dir: false,
                    children: Vec::new(),
                });
            }
        }
    }

    p.files.fetch_add(own_files, Ordering::Relaxed);
    p.bytes.fetch_add(own_bytes, Ordering::Relaxed);
    p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);

    let mut size = own_bytes;
    let mut dir_nodes: Vec<SizeNode> = Vec::with_capacity(subdirs.len());
    for (path, nm) in subdirs {
        if p.cancel.load(Ordering::Relaxed) {
            break;
        }
        let node = scan_backend_dir(be, &path, nm, p);
        size += node.size;
        dir_nodes.push(node);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_sizes_and_counts() {
        let base = std::env::temp_dir().join(format!("se_an_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.txt"), vec![0u8; 100]).unwrap();
        std::fs::write(base.join("sub/b.bin"), vec![0u8; 250]).unwrap();
        std::fs::write(base.join("sub/c.bin"), vec![0u8; 150]).unwrap();

        let p = Progress::default();
        let root = scan(&base, &p);
        assert!(root.is_dir);
        assert_eq!(root.size, 500);
        assert_eq!(p.files.load(Ordering::Relaxed), 3);
        let sub = root.children.iter().find(|c| &*c.name == "sub").unwrap();
        assert_eq!(sub.size, 400);
        assert_eq!(sub.children.len(), 2);
        // Leaf nodes carry no children (no wasted allocation).
        let a = root.children.iter().find(|c| &*c.name == "a.txt").unwrap();
        assert_eq!(a.size, 100);
        assert!(!a.is_dir && a.children.is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn scan_backend_via_local_backend() {
        // Exercise the VFS-backend walk against the real LocalBackend so the
        // remote code path is covered without a network.
        let base = std::env::temp_dir().join(format!("se_anbe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.txt"), vec![0u8; 100]).unwrap();
        std::fs::write(base.join("sub/b.bin"), vec![0u8; 250]).unwrap();

        let root = base.to_string_lossy().replace('\\', "/");
        let be = crate::vfs::LocalBackend::new("/");
        let p = Progress::default();
        let node = scan_backend(&be, &root, &p);
        assert!(node.is_dir);
        assert_eq!(node.size, 350);
        assert_eq!(p.files.load(Ordering::Relaxed), 2);
        let sub = node.children.iter().find(|c| &*c.name == "sub").unwrap();
        assert_eq!(sub.size, 250);
        assert_eq!(sub.children.len(), 1);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn parallel_tree_assembly() {
        // build_from_listings reconstructs the tree from per-dir listings (the
        // parallel walk's output) — verify sizes + nesting deterministically.
        let mut l: std::collections::HashMap<String, Vec<ChildMeta>> =
            std::collections::HashMap::new();
        l.insert(
            "/r".into(),
            vec![
                ChildMeta {
                    name: "sub".into(),
                    is_dir: true,
                    size: 0,
                },
                ChildMeta {
                    name: "a.txt".into(),
                    is_dir: false,
                    size: 100,
                },
            ],
        );
        l.insert(
            "/r/sub".into(),
            vec![
                ChildMeta {
                    name: "b.bin".into(),
                    is_dir: false,
                    size: 250,
                },
                ChildMeta {
                    name: "c.bin".into(),
                    is_dir: false,
                    size: 150,
                },
            ],
        );
        let node = build_from_listings("/r", "r".into(), &l);
        assert_eq!(node.size, 500);
        // Directories sort before files in the assembled children.
        assert!(node.children[0].is_dir);
        let sub = node.children.iter().find(|c| &*c.name == "sub").unwrap();
        assert_eq!(sub.size, 400);
        assert_eq!(sub.children.len(), 2);
    }
}
