use super::super::{start_scan, RetentionHandle, ScanMessage, ScanOpts, ScanRetention};
use crate::types::FileEntry;
use crossbeam_channel::{unbounded, Receiver};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

/// `root/{a.txt, sub/{b.dat, deep/keep.txt}, other/{x.dat}}`.
fn retention_tree(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("se_scan_{label}_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(path.join("sub").join("deep")).unwrap();
    std::fs::create_dir_all(path.join("other")).unwrap();
    std::fs::write(path.join("a.txt"), b"hello").unwrap();
    std::fs::write(path.join("sub").join("b.dat"), b"xy").unwrap();
    std::fs::write(path.join("sub").join("deep").join("keep.txt"), b"k").unwrap();
    std::fs::write(path.join("other").join("x.dat"), b"x").unwrap();
    path
}

struct TxtOnly {
    skip_dir: Option<&'static str>,
}

impl ScanRetention for TxtOnly {
    fn retain(&self, entry: &FileEntry) -> bool {
        !entry.is_dir && entry.name.ends_with(".txt")
    }

    fn descend(&self, directory: &FileEntry) -> bool {
        self.skip_dir != Some(directory.name.as_ref())
    }
}

fn drain(rx: &Receiver<ScanMessage>) -> (HashSet<String>, Vec<FileEntry>, u64) {
    let mut names = HashSet::new();
    let mut entries = Vec::new();
    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(ScanMessage::Entries(batch)) => {
                for entry in batch {
                    names.insert(entry.name.to_string());
                    entries.push(entry);
                }
            }
            Ok(ScanMessage::Done(progress)) => return (names, entries, progress.scanned),
            Ok(_) => {}
            Err(error) => panic!("scan did not finish: {error}"),
        }
    }
}

#[test]
fn recursive_filter_task_unfiltered_scan_still_emits_every_entry() {
    let root = retention_tree("all");
    let (tx, rx) = unbounded();
    let handle = start_scan(root.clone(), ScanOpts::everything(None), tx);
    let (names, entries, scanned) = drain(&rx);
    for expected in [
        "a.txt", "sub", "b.dat", "deep", "keep.txt", "other", "x.dat",
    ] {
        assert!(names.contains(expected), "missing {expected}: {names:?}");
    }
    assert_eq!(scanned, 7);
    assert_eq!(entries.len(), 8, "root entry plus seven children");
    assert!(!handle.truncated.load(std::sync::atomic::Ordering::Relaxed));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn recursive_filter_task_filtered_scan_emits_matches_with_their_ancestors_only() {
    let root = retention_tree("filtered");
    let (tx, rx) = unbounded();
    let retention: RetentionHandle = Arc::new(TxtOnly { skip_dir: None });
    let opts = ScanOpts {
        follow_symlinks: false,
        max_depth: None,
        retention: Some(retention),
    };
    start_scan(root.clone(), opts, tx);
    let (names, entries, scanned) = drain(&rx);
    let root_name = root.file_name().unwrap().to_string_lossy().to_string();
    for expected in [root_name.as_str(), "a.txt", "sub", "deep", "keep.txt"] {
        assert!(names.contains(expected), "missing {expected}: {names:?}");
    }
    for pruned in ["b.dat", "other", "x.dat"] {
        assert!(!names.contains(pruned), "retained {pruned}: {names:?}");
    }
    assert_eq!(entries.len(), 5, "each ancestor is emitted exactly once");
    assert_eq!(scanned, 7, "progress still counts every visited entry");
    let keep = entries
        .iter()
        .find(|entry| entry.name.as_ref() == "keep.txt")
        .unwrap();
    let deep = entries
        .iter()
        .find(|entry| entry.name.as_ref() == "deep")
        .unwrap();
    assert_eq!(keep.parent, deep.path, "the tree can place the match");
    assert!(deep.is_dir && deep.depth == 2 && keep.depth == 3);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn recursive_filter_task_filtered_scan_prunes_subtrees_it_may_not_descend() {
    let root = retention_tree("pruned");
    let (tx, rx) = unbounded();
    let retention: RetentionHandle = Arc::new(TxtOnly {
        skip_dir: Some("sub"),
    });
    let opts = ScanOpts {
        follow_symlinks: false,
        max_depth: None,
        retention: Some(retention),
    };
    start_scan(root.clone(), opts, tx);
    let (names, entries, scanned) = drain(&rx);
    assert!(names.contains("a.txt"));
    assert!(!names.contains("sub") && !names.contains("keep.txt"));
    assert_eq!(entries.len(), 2, "root and a.txt");
    assert_eq!(scanned, 4, "root children plus other/x.dat only");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn recursive_filter_task_depth_limit_still_bounds_a_filtered_scan() {
    let root = retention_tree("depth");
    let (tx, rx) = unbounded();
    let retention: RetentionHandle = Arc::new(TxtOnly { skip_dir: None });
    let opts = ScanOpts {
        follow_symlinks: false,
        max_depth: Some(2),
        retention: Some(retention),
    };
    start_scan(root.clone(), opts, tx);
    let (names, _, scanned) = drain(&rx);
    assert!(names.contains("a.txt"));
    assert!(!names.contains("keep.txt"), "depth 3 is beyond the limit");
    assert_eq!(scanned, 6, "deep/ is listed at depth 2 but not entered");
    std::fs::remove_dir_all(&root).ok();
}
