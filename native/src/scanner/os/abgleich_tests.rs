//! Local recursive view: thread count, device facts asked once per device, the
//! optical guard only on a protected subtree, and equal rows. Test names start
//! with `abgleich_task_` (the remote task suite selects them).
use super::{collect_recursive, start_scan, ScanMessage, ScanOpts};
use crate::source_media::{test_profiles, Source};
use crate::types::FileEntry;
use crossbeam_channel::unbounded;
use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A temporary tree, removed when dropped.
struct Tree(PathBuf);

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `plain/{a.txt, deep/b.txt, many/fNN/x.txt}` and `optical/{c.txt, sub/d.txt}`
/// below a fresh root. Returns the tree and every relative path the view shows.
fn tree(label: &str) -> (Tree, BTreeSet<String>) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "se_abgleich_{label}_{}_{nanos}",
        std::process::id()
    ));
    let tree = Tree(root.clone());
    let mut files = vec![
        "plain/a.txt".to_string(),
        "plain/deep/b.txt".to_string(),
        "optical/c.txt".to_string(),
        "optical/sub/d.txt".to_string(),
    ];
    for number in 0..40 {
        files.push(format!("plain/many/f{number:02}/x.txt"));
    }
    let mut rows = BTreeSet::new();
    for rel in &files {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();
        let mut prefix = String::new();
        for part in rel.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            rows.insert(prefix.clone());
        }
    }
    (tree, rows)
}

/// Every row the view delivers below `root`, as paths relative to it.
fn rows_below(root: &Path) -> BTreeSet<String> {
    let (tx, rx) = unbounded();
    let _handle = start_scan(root.to_path_buf(), ScanOpts::everything(None), tx);
    let mut entries: Vec<FileEntry> = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(ScanMessage::Entries(batch)) => entries.extend(batch),
            Ok(ScanMessage::Done(_)) => break,
            Ok(_) => {}
            Err(error) => panic!("scan did not finish: {error}"),
        }
    }
    let root_text = entries
        .iter()
        .find(|entry| entry.depth == 0)
        .map(|entry| entry.path.to_string())
        .unwrap_or_default();
    entries
        .iter()
        .filter(|entry| entry.depth > 0)
        .map(|entry| {
            let path: &str = &entry.path;
            path.strip_prefix(root_text.as_str())
                .map_or(path, |rest| rest.trim_start_matches('/'))
                .to_string()
        })
        .collect()
}

#[test]
fn abgleich_task_scan_threads_follow_the_cores_available_at_run_time() {
    let sixteen = NonZeroUsize::new(16).unwrap();
    assert_eq!(
        super::walk::scan_threads_for(Ok(sixteen)),
        16,
        "no cap at four threads"
    );
    assert_eq!(
        super::walk::scan_threads_for(Err(std::io::Error::other("unknown"))),
        1,
        "one thread when the count is unknown"
    );
    let available = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    assert_eq!(super::walk::scan_threads(), available);
}

#[test]
fn abgleich_task_plain_tree_asks_for_its_device_once_not_per_folder() {
    let (tree, expected) = tree("probes");
    let watch = test_profiles::watch_probes(&tree.0);
    assert_eq!(rows_below(&tree.0), expected, "the view shows every entry");
    assert!(
        watch.count() <= 3,
        "device probes below the root: {} (the folders are not probed)",
        watch.count()
    );
}

#[test]
fn abgleich_task_only_the_protected_subtree_takes_the_device_guard() {
    let (tree, expected) = tree("guard");
    let optical = tree.0.join("optical");
    let device = format!("abgleich-task-optical-{}", tree.0.display());
    let _profile = test_profiles::register(
        &optical,
        Source {
            device: device.clone(),
            generation: "abgleich-task".to_string(),
            read_only: true,
        },
    );
    let plain_watch = test_profiles::watch_probes(&tree.0.join("plain"));
    let optical_watch = test_profiles::watch_probes(&optical);
    assert_eq!(
        rows_below(&tree.0),
        expected,
        "the view shows every entry, the protected ones included"
    );
    assert_eq!(
        plain_watch.count(),
        0,
        "no device probe below the plain folders"
    );
    assert!(
        optical_watch.count() >= 1,
        "the protected subtree was determined"
    );
    assert!(
        test_profiles::acquisitions(&device) >= 2,
        "the protected listings took the device lock"
    );
}

/// The rows of a copy expansion below `root`, as paths relative to it.
fn expansion_rows(root: &Path, entries: &[FileEntry]) -> BTreeSet<String> {
    let root_text = root.to_string_lossy().into_owned();
    entries
        .iter()
        .map(|entry| {
            let path: &str = &entry.path;
            path.strip_prefix(root_text.as_str())
                .map_or(path, |rest| rest.trim_start_matches('/'))
                .to_string()
        })
        .collect()
}

#[test]
fn abgleich_task_copy_expansion_takes_the_guard_only_on_the_protected_subtree() {
    let (tree, expected) = tree("collect-guard");
    let optical = tree.0.join("optical");
    let device = format!("abgleich-task-collect-{}", tree.0.display());
    let _profile = test_profiles::register(
        &optical,
        Source {
            device: device.clone(),
            generation: "abgleich-task".to_string(),
            read_only: true,
        },
    );
    let plain_watch = test_profiles::watch_probes(&tree.0.join("plain"));
    let optical_watch = test_profiles::watch_probes(&optical);
    let cancel = AtomicBool::new(false);
    let outcome = collect_recursive(&tree.0, false, 1, &cancel);
    assert!(outcome.is_complete(), "the copy expansion is complete");
    assert_eq!(
        expansion_rows(&tree.0, &outcome.entries),
        expected,
        "the copy expansion lists every entry, the protected ones included"
    );
    assert_eq!(
        plain_watch.count(),
        0,
        "no device probe below the plain folders"
    );
    assert!(
        optical_watch.count() >= 1,
        "the protected subtree was determined"
    );
    assert!(
        test_profiles::acquisitions(&device) >= 2,
        "the protected listings took the device lock"
    );
}

#[test]
fn abgleich_task_copy_expansion_asks_for_its_device_once_not_per_folder() {
    let (tree, expected) = tree("collect-probes");
    let watch = test_profiles::watch_probes(&tree.0);
    let cancel = AtomicBool::new(false);
    let outcome = collect_recursive(&tree.0, false, 1, &cancel);
    assert!(outcome.is_complete(), "the copy expansion is complete");
    assert_eq!(
        expansion_rows(&tree.0, &outcome.entries),
        expected,
        "the copy expansion lists every entry"
    );
    // The expansion probes the root for its own metadata read, its optical check
    // and its device; no folder below the root is probed.
    assert!(
        watch.count() <= 4,
        "device probes below the root: {} for {} rows (the folders are not probed)",
        watch.count(),
        expected.len()
    );
}
