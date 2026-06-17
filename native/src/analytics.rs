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
    scan_dir(root, name.into_boxed_str(), p)
}

fn scan_dir(dir: &Path, name: Box<str>, p: &Progress) -> SizeNode {
    if p.cancel.load(Ordering::Relaxed) {
        return SizeNode { name, size: 0, is_dir: true, children: Vec::new() };
    }
    let mut subdirs: Vec<(PathBuf, Box<str>)> = Vec::new();
    let mut files: Vec<SizeNode> = Vec::new();
    let mut own_files = 0u64;
    let mut own_bytes = 0u64;

    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            // Don't follow symlinks/reparse points — avoids cycles and
            // double-counting the same bytes.
            if ft.is_symlink() {
                continue;
            }
            let nm: Box<str> = ent.file_name().to_string_lossy().into_owned().into_boxed_str();
            if ft.is_dir() {
                subdirs.push((ent.path(), nm));
            } else if ft.is_file() {
                let sz = ent.metadata().map(|m| m.len()).unwrap_or(0);
                own_files += 1;
                own_bytes += sz;
                files.push(SizeNode { name: nm, size: sz, is_dir: false, children: Vec::new() });
            }
        }
    }

    p.files.fetch_add(own_files, Ordering::Relaxed);
    p.bytes.fetch_add(own_bytes, Ordering::Relaxed);
    p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);

    // Recurse in parallel. A serial fallback for tiny lists avoids rayon
    // overhead on leaf-heavy trees.
    let mut dir_nodes: Vec<SizeNode> = if subdirs.len() > 1 {
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
    SizeNode { name, size, is_dir: true, children }
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
}
