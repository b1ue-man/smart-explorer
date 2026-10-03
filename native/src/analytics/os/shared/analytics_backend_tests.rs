use super::*;
use crate::analytics::ScanStatus;
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io::{self, Read, Write};

/// A remote tree served from listings, optionally walked in parallel.
struct Listings {
    dirs: HashMap<String, Vec<VfsMeta>>,
    parallel: usize,
}

fn dir(name: &str) -> VfsMeta {
    VfsMeta {
        name: name.into(),
        is_dir: true,
        ..VfsMeta::default()
    }
}

fn file(name: &str, size: u64) -> VfsMeta {
    VfsMeta {
        name: name.into(),
        size,
        ..VfsMeta::default()
    }
}

impl Backend for Listings {
    fn parallelism(&self) -> usize {
        self.parallel
    }
    fn scheme(&self) -> Scheme {
        Scheme::Webdav
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.dirs
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_string()))
    }
    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Err(io::Error::other("unused"))
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Err(io::Error::other("unused"))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::other("unused"))
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
}

/// `/` with `width` folders of `files` files each, two levels deep.
fn wide_tree(width: usize, files: usize, parallel: usize) -> Listings {
    let mut dirs = HashMap::new();
    let mut root = Vec::new();
    for index in 0..width {
        let name = format!("d{index}");
        root.push(dir(&name));
        let sub = format!("/{name}");
        dirs.insert(
            sub.clone(),
            (0..files)
                .map(|file_index| file(&format!("f{file_index}"), 10))
                .chain([dir("inner")])
                .collect(),
        );
        dirs.insert(format!("{sub}/inner"), vec![file("deep.bin", 5)]);
    }
    root.push(file("top.bin", 1));
    dirs.insert("/".into(), root);
    Listings { dirs, parallel }
}

#[test]
fn review_task_backend_walk_is_exact_serial_and_parallel() {
    for parallel in [1, 4] {
        let backend = wide_tree(20, 3, parallel);
        let progress = Progress::default();
        let outcome = scan_backend(&backend, "/", &progress);
        assert_eq!(outcome.status, ScanStatus::Complete);
        let tree = outcome.tree.expect("tree");
        // 20 × (3 × 10 + 5) + 1
        assert_eq!(tree.size, 701);
        assert_eq!(progress.files.load(Ordering::Relaxed), 20 * 4 + 1);
        assert_eq!(progress.dirs.load(Ordering::Relaxed), 40);
        let first = tree.children.iter().find(|child| &*child.name == "d0");
        assert_eq!(first.map(|node| node.size), Some(35));
    }
}

#[test]
fn review_task_backend_walk_honours_the_retention_budget() {
    let backend = wide_tree(10, 3, 1);
    let progress = Progress::default();
    let diagnostics = Diagnostics::default();
    // The root and three more nodes; everything else folds into aggregates.
    let budget = AnalyticsBudget::with_limits(4, 1 << 20, 2048);
    let _ = budget.claim(Path::new("/"), 0, 1, &diagnostics);
    let walker = Walker {
        backend: &backend,
        progress: &progress,
        diagnostics: &diagnostics,
        budget: &budget,
        parallel: false,
    };
    let tree = walker.dir("/", "root".into(), 0, true);
    let outcome = diagnostics.finish(tree, false);
    assert_eq!(outcome.status, ScanStatus::Complete);
    assert!(outcome.aggregated_files > 0);
    assert_eq!(outcome.notes.len(), 1, "{:?}", outcome.notes);
    let tree = outcome.tree.expect("tree");
    assert_eq!(tree.size, 351, "sizes stay exact while detail folds");
    fn count(node: &SizeNode) -> usize {
        1 + node.children.iter().map(count).sum::<usize>()
    }
    // The root, three kept nodes and the aggregates that hold the rest.
    assert!(count(&tree) < 20, "{} nodes kept", count(&tree));
}

#[test]
fn review_task_listing_fallback_honours_the_offered_node_budget() {
    let backend = wide_tree(10, 3, 1);
    let progress = Progress::default();
    progress.set_node_budget(2);
    let outcome = scan_backend(&backend, "/", &progress);
    assert_eq!(outcome.status, ScanStatus::Complete);
    let tree = outcome.tree.unwrap();
    assert_eq!(tree.size, 351);
    fn count(node: &SizeNode) -> usize { 1 + node.children.iter().map(count).sum::<usize>() }
    assert!(count(&tree) <= 2);
    assert!(!outcome.notes.is_empty());
}

#[test]
fn review_task_backend_walk_keeps_folders_with_unusable_entries() {
    let mut dirs = HashMap::new();
    dirs.insert(
        "/".to_string(),
        vec![
            file("ok.bin", 7),
            file("a\\b.bin", 3),
            dir("x\\y"),
            dir("twice"),
            dir("twice"),
            file("same.jpg", 2),
            file("same.jpg", 2),
        ],
    );
    dirs.insert("/twice".to_string(), vec![file("in.bin", 4)]);
    let backend = Listings { dirs, parallel: 1 };
    let outcome = scan_backend(&backend, "/", &Progress::default());
    assert_eq!(outcome.status, ScanStatus::Partial);
    let paths: Vec<&str> = outcome
        .issues
        .iter()
        .map(|issue| issue.path.as_str())
        .collect();
    assert_eq!(paths, ["/x\\y", "/twice"]);
    let tree = outcome.tree.expect("the folder stays readable");
    // ok 7 + unusable file 3 (aggregated) + twice 4 (once) + two same-named files 2 + 2.
    assert_eq!(tree.size, 18);
}
