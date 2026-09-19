use super::*;
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Cursor, Read, Write};

#[test]
fn analytics_access_task_sizes_and_counts() {
    let fixture = tempfile::tempdir().unwrap();
    let base = fixture.path();
    std::fs::create_dir_all(base.join("sub")).unwrap();
    std::fs::write(base.join("a.txt"), vec![0u8; 100]).unwrap();
    std::fs::write(base.join("sub/b.bin"), vec![0u8; 250]).unwrap();
    std::fs::write(base.join("sub/c.bin"), vec![0u8; 150]).unwrap();

    let p = Progress::default();
    let outcome = scan(&base, &p);
    assert_eq!(outcome.status, ScanStatus::Complete);
    let root = outcome.tree.unwrap();
    assert!(root.is_dir);
    assert_eq!(root.size, 500);
    assert_eq!(p.files.load(Ordering::Relaxed), 3);
    let sub = root.children.iter().find(|c| &*c.name == "sub").unwrap();
    assert_eq!(sub.size, 400);
    assert_eq!(sub.children.len(), 2);
    let a = root.children.iter().find(|c| &*c.name == "a.txt").unwrap();
    assert_eq!(a.size, 100);
    assert!(!a.is_dir && a.children.is_empty());
}

#[test]
fn analytics_access_task_missing_local_root_is_failed_not_empty_success() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("se_an_missing");
    let outcome = scan(&root, &Progress::default());
    assert_eq!(outcome.status, ScanStatus::Failed);
    assert!(outcome.tree.is_none());
    assert_eq!(outcome.issues.len(), 1);
    assert!(outcome.issues[0].path.contains("se_an_missing"));
}

#[test]
fn analytics_access_task_first_entry_error_preserves_readable_sibling() {
    let fixture = tempfile::tempdir().unwrap();
    let progress = Progress::default();
    let diagnostics = Diagnostics::default();
    let budget = AnalyticsBudget::default();
    let traversal = Traversal {
        progress: &progress,
        diagnostics: &diagnostics,
        budget: &budget,
        parallel: false,
    };
    let entries = vec![
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "first child denied",
        )),
        Ok(LocalEntry {
            name: "readable.bin".into(),
            kind: EntryKind::File,
            size: 7,
            unreachable: false,
            ..Default::default()
        }),
    ];
    let tree = scan_entries(
        &traversal,
        fixture.path(),
        "root".into(),
        Ok(entries.into_iter()),
        0,
        true,
    );
    let outcome = diagnostics.finish(tree, false);
    assert_eq!(outcome.status, ScanStatus::Partial);
    assert_eq!(outcome.permission_denied, 1);
    assert_eq!(outcome.issues.len(), 1);
    assert!(outcome.issues[0].detail.contains("first child denied"));
    let tree = outcome
        .tree
        .expect("one child error must not discard readable siblings");
    assert_eq!(tree.size, 7);
    assert_eq!(&*tree.children[0].name, "readable.bin");
    assert_eq!(progress.files.load(Ordering::Relaxed), 1);
    assert_eq!(progress.bytes.load(Ordering::Relaxed), 7);
}

#[test]
fn analytics_access_task_huge_directory_keeps_exact_totals_with_folded_files() {
    let fixture = tempfile::tempdir().unwrap();
    let base = fixture.path().join("many");
    std::fs::create_dir_all(&base).unwrap();
    let count = MAX_RETAINED_FILES_PER_DIRECTORY + 12;
    let mut expected = 0u64;
    for index in 0..count {
        let size = (index % 7) as u64 + 1;
        expected += size;
        std::fs::write(base.join(format!("f{index:05}")), vec![0u8; size as usize]).unwrap();
    }
    let p = Progress::default();
    let outcome = scan(&base, &p);
    assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
    let root = outcome.tree.unwrap();
    assert_eq!(root.size, expected, "every file is counted");
    assert_eq!(p.files.load(Ordering::Relaxed), count as u64);
    assert_eq!(root.children.len(), MAX_RETAINED_FILES_PER_DIRECTORY + 1);
    let folded = root
        .children
        .iter()
        .find(|child| child.name.starts_with("…"))
        .expect("aggregate node for the folded files");
    assert!(!folded.is_dir);
    assert_eq!(outcome.aggregated_files, 12);
    let retained: u64 = root
        .children
        .iter()
        .filter(|child| !child.name.starts_with("…"))
        .map(|child| child.size)
        .sum();
    assert_eq!(retained + folded.size, expected);
    // The largest files are the ones kept individually.
    assert!(root
        .children
        .iter()
        .filter(|child| !child.name.starts_with("…"))
        .all(|child| child.size >= 1));
}

#[test]
fn analytics_access_task_unrepresentable_and_erroring_entries_never_end_the_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let progress = Progress::default();
    let diagnostics = Diagnostics::default();
    let budget = AnalyticsBudget::default();
    let traversal = Traversal {
        progress: &progress,
        diagnostics: &diagnostics,
        budget: &budget,
        parallel: false,
    };
    let entries = vec![
        Ok(LocalEntry {
            name: "weird\u{fffd}dir".into(),
            kind: EntryKind::Directory,
            size: 0,
            unreachable: true,
            ..Default::default()
        }),
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Ungueltiger Eintrag",
        )),
        Ok(LocalEntry {
            name: "nul\u{fffd}".into(),
            kind: EntryKind::File,
            size: 5,
            unreachable: true,
            ..Default::default()
        }),
        Ok(LocalEntry {
            name: "after.bin".into(),
            kind: EntryKind::File,
            size: 9,
            unreachable: false,
            ..Default::default()
        }),
    ];
    let tree = scan_entries(
        &traversal,
        fixture.path(),
        "root".into(),
        Ok(entries.into_iter()),
        0,
        true,
    );
    let outcome = diagnostics.finish(tree, false);
    assert_eq!(outcome.status, ScanStatus::Partial);
    assert_eq!(outcome.issues.len(), 2, "{:?}", outcome.issues);
    let tree = outcome.tree.expect("root stays readable");
    assert_eq!(tree.size, 14, "unrepresentable files are still counted");
    assert_eq!(progress.files.load(Ordering::Relaxed), 2);
    assert!(tree
        .children
        .iter()
        .any(|child| &*child.name == "after.bin"));
}

#[test]
fn scan_backend_via_local_backend() {
    let base = std::env::temp_dir().join(format!("se_anbe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("sub")).unwrap();
    std::fs::write(base.join("a.txt"), vec![0u8; 100]).unwrap();
    std::fs::write(base.join("sub/b.bin"), vec![0u8; 250]).unwrap();

    let root = base.to_string_lossy().replace('\\', "/");
    let be = crate::vfs::LocalBackend::new("/");
    let p = Progress::default();
    let outcome = scan_backend(&be, &root, &p);
    assert_eq!(outcome.status, ScanStatus::Complete);
    let node = outcome.tree.unwrap();
    assert!(node.is_dir);
    assert_eq!(node.size, 350);
    assert_eq!(p.files.load(Ordering::Relaxed), 2);
    let sub = node.children.iter().find(|c| &*c.name == "sub").unwrap();
    assert_eq!(sub.size, 250);
    assert_eq!(sub.children.len(), 1);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn analytics_access_task_backend_child_error_is_partial_and_root_error_is_failed() {
    for parallel in [1, 3] {
        let p = Progress::default();
        let partial = scan_backend(
            &FailingBackend {
                fail_root: false,
                parallel,
            },
            "/",
            &p,
        );
        assert_eq!(partial.status, ScanStatus::Partial);
        assert_eq!(partial.tree.as_ref().map(|n| n.size), Some(7));
        assert_eq!(partial.issues.len(), 1);
        assert_eq!(partial.issues[0].path, "/broken");
        assert_eq!(partial.permission_denied, 1);

        let failed = scan_backend(
            &FailingBackend {
                fail_root: true,
                parallel,
            },
            "/",
            &Progress::default(),
        );
        assert_eq!(failed.status, ScanStatus::Failed);
        assert!(failed.tree.is_none());
        assert_eq!(failed.issues[0].path, "/");
        assert_eq!(failed.permission_denied, 1);
    }
}

#[test]
fn parallel_tree_assembly() {
    let mut listings: std::collections::HashMap<String, Vec<ChildMeta>> =
        std::collections::HashMap::new();
    listings.insert(
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
    listings.insert(
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
    let node = build_from_listings("/r", "r".into(), &listings);
    assert_eq!(node.size, 500);
    assert!(node.children[0].is_dir);
    let sub = node.children.iter().find(|c| &*c.name == "sub").unwrap();
    assert_eq!(sub.size, 400);
    assert_eq!(sub.children.len(), 2);
}

struct FailingBackend {
    fail_root: bool,
    parallel: usize,
}

impl Backend for FailingBackend {
    fn parallelism(&self) -> usize {
        self.parallel
    }
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if (self.fail_root && path == "/") || path == "/broken" {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        }
        Ok(vec![
            VfsMeta {
                name: "broken".into(),
                is_dir: true,
                ..VfsMeta::default()
            },
            VfsMeta {
                name: "ok.bin".into(),
                size: 7,
                ..VfsMeta::default()
            },
        ])
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Ok(())
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
}
