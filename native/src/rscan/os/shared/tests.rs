use super::*;
use crate::vfs::LocalBackend;
use crossbeam_channel::unbounded;
use std::collections::HashSet;
use std::path::PathBuf;

fn temp_tree() -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("rscan_{}_{}", std::process::id(), nanos));
    std::fs::create_dir_all(path.join("sub")).unwrap();
    std::fs::write(path.join("a.txt"), b"hello").unwrap();
    std::fs::write(path.join("sub").join("b.dat"), b"xy").unwrap();
    path
}

/// `root/{a.txt, sub/{b.dat, deep/keep.txt}, other/{x.dat}}`: one match below
/// two unemitted directories, one subtree without matches.
fn retention_tree() -> PathBuf {
    let path = temp_tree();
    std::fs::create_dir_all(path.join("sub").join("deep")).unwrap();
    std::fs::write(path.join("sub").join("deep").join("keep.txt"), b"k").unwrap();
    std::fs::create_dir_all(path.join("other")).unwrap();
    std::fs::write(path.join("other").join("x.dat"), b"x").unwrap();
    path
}

/// Keeps `.txt` files; optionally refuses to descend into one directory name.
struct TxtOnly {
    skip_dir: Option<&'static str>,
}

impl crate::scanner::ScanRetention for TxtOnly {
    fn retain(&self, entry: &crate::types::FileEntry) -> bool {
        !entry.is_dir && entry.name.ends_with(".txt")
    }

    fn descend(&self, directory: &crate::types::FileEntry) -> bool {
        self.skip_dir != Some(directory.name.as_ref())
    }
}

fn drain(rx: &crossbeam_channel::Receiver<ScanMessage>) -> (HashSet<String>, u64) {
    let mut names = HashSet::new();
    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(ScanMessage::Entries(entries)) => {
                for entry in entries {
                    names.insert(entry.name.to_string());
                }
            }
            Ok(ScanMessage::Done(progress)) => return (names, progress.scanned),
            Ok(_) => {}
            Err(_) => return (names, 0),
        }
    }
}

#[test]
fn walks_full_tree_via_backend() {
    let directory = temp_tree();
    let root = directory.to_string_lossy().replace('\\', "/");
    let backend: BackendHandle = Arc::new(LocalBackend::new(&root));
    let (tx, rx) = unbounded();
    start_scan_backend(backend, root, None, None, tx);
    let (names, scanned) = drain(&rx);
    assert!(names.contains("a.txt"), "names: {names:?}");
    assert!(names.contains("sub"));
    assert!(names.contains("b.dat"), "should recurse into sub");
    assert_eq!(scanned, 3);
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn recursive_filter_task_remote_retention_emits_matches_with_their_ancestors_only() {
    let directory = retention_tree();
    let root = directory.to_string_lossy().replace('\\', "/");
    let backend: BackendHandle = Arc::new(LocalBackend::new(&root));
    let (tx, rx) = unbounded();
    let retention: crate::scanner::RetentionHandle = Arc::new(TxtOnly { skip_dir: None });
    start_scan_backend(backend, root.clone(), None, Some(retention), tx);
    let (names, scanned) = drain(&rx);
    let root_name = directory.file_name().unwrap().to_string_lossy().to_string();
    for expected in [root_name.as_str(), "a.txt", "sub", "deep", "keep.txt"] {
        assert!(names.contains(expected), "missing {expected}: {names:?}");
    }
    for pruned in ["b.dat", "other", "x.dat"] {
        assert!(!names.contains(pruned), "retained {pruned}: {names:?}");
    }
    assert_eq!(scanned, 7, "progress still counts every visited entry");
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn recursive_filter_task_remote_retention_can_prune_whole_subtrees() {
    let directory = retention_tree();
    let root = directory.to_string_lossy().replace('\\', "/");
    let backend: BackendHandle = Arc::new(LocalBackend::new(&root));
    let (tx, rx) = unbounded();
    let retention: crate::scanner::RetentionHandle = Arc::new(TxtOnly {
        skip_dir: Some("sub"),
    });
    start_scan_backend(backend, root, None, Some(retention), tx);
    let (names, scanned) = drain(&rx);
    assert!(names.contains("a.txt"));
    assert!(!names.contains("sub") && !names.contains("keep.txt"));
    assert_eq!(scanned, 4, "root children plus other/x.dat only");
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn flat_depth_one_does_not_recurse() {
    let directory = temp_tree();
    let root = directory.to_string_lossy().replace('\\', "/");
    let backend: BackendHandle = Arc::new(LocalBackend::new(&root));
    let (tx, rx) = unbounded();
    start_scan_backend(backend, root, Some(1), None, tx);
    let (names, scanned) = drain(&rx);
    assert!(names.contains("a.txt") && names.contains("sub"));
    assert!(!names.contains("b.dat"), "depth 1 must not recurse");
    assert_eq!(scanned, 2);
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn recursive_scan_uses_parallel_backend_width() {
    use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
    use std::io::{self, Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ParallelBackend {
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    impl ParallelBackend {
        fn enter(&self) {
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(40));
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl Backend for ParallelBackend {
        fn scheme(&self) -> Scheme {
            Scheme::GDrive
        }

        fn root_display(&self) -> String {
            "/root".into()
        }

        fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
            self.enter();
            if path == "/root" {
                return Ok((0..4)
                    .map(|index| VfsMeta {
                        name: format!("d{index}"),
                        is_dir: true,
                        ..Default::default()
                    })
                    .collect());
            }
            let name = path.rsplit('/').next().unwrap_or("x");
            Ok(vec![VfsMeta {
                name: format!("{name}.txt"),
                size: 1,
                ..Default::default()
            }])
        }

        fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
            Ok(VfsMeta {
                name: "root".into(),
                is_dir: true,
                ..Default::default()
            })
        }

        fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }

        fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }

        fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
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

        fn parallelism(&self) -> usize {
            4
        }
    }

    let typed = Arc::new(ParallelBackend {
        active: AtomicUsize::new(0),
        max_active: AtomicUsize::new(0),
    });
    let backend: BackendHandle = typed.clone();
    let (tx, rx) = unbounded();
    start_scan_backend(backend, "/root".into(), None, None, tx);
    let (names, scanned) = drain(&rx);
    assert_eq!(scanned, 8);
    assert!(names.contains("d0.txt") && names.contains("d3.txt"));
    assert!(
        typed.max_active.load(Ordering::SeqCst) > 1,
        "recursive scan did not list sibling folders concurrently"
    );
}

#[test]
fn thread_start_failures_have_error_and_done_messages() {
    let (tx, rx) = unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    report_spawn_failure(
        &tx,
        &cancel,
        "test scan",
        std::io::Error::other("no thread"),
    );
    assert!(cancel.load(Ordering::Relaxed));
    assert!(matches!(rx.recv().unwrap(), ScanMessage::Error(_)));
    assert!(matches!(
        rx.recv().unwrap(),
        ScanMessage::Done(ScanProgress { errors: 1, .. })
    ));
}
