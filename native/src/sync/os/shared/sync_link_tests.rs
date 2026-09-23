use super::*;
use crate::bisync::link_fixture;
use crate::vfs::{BackendHandle, LocalBackend};
use std::path::Path;
use std::sync::Arc;

fn forward(path: &Path) -> String { path.to_string_lossy().replace('\\', "/") }

fn mirror(a: &Path, b: &Path, dry_run: bool) -> SyncResult {
    let (ra, rb) = (forward(a), forward(b));
    let source: BackendHandle = Arc::new(LocalBackend::new(&ra));
    let destination: BackendHandle = Arc::new(LocalBackend::new(&rb));
    let (tx, rx) = crossbeam_channel::unbounded();
    let _handle = start_sync(source, ra, destination, rb,
        SyncOptions { delete_extra: true, dry_run }, tx);
    loop {
        if let SyncMsg::Done(result) = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
            return result;
        }
    }
}

#[test]
fn sync_links_task_quick_mirror_preserves_links_counterparts_and_parent_directories() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private.txt"), b"outside").unwrap();
    link_fixture::directory(outside.path(), &a.path().join("node_modules"));
    std::fs::create_dir(b.path().join("node_modules")).unwrap();
    std::fs::write(b.path().join("node_modules/keep.txt"), b"keep destination").unwrap();
    std::fs::create_dir(b.path().join("extra_parent")).unwrap();
    link_fixture::directory(outside.path(), &b.path().join("extra_parent/target_link"));
    std::fs::write(b.path().join("extra_parent/extra.txt"), b"delete extra sibling").unwrap();
    std::fs::create_dir(a.path().join("target_link")).unwrap();
    std::fs::write(a.path().join("target_link/not-outside.txt"), b"protected source").unwrap();
    link_fixture::directory(outside.path(), &b.path().join("target_link"));
    std::fs::write(a.path().join("ordinary.txt"), b"ordinary sync").unwrap();
    let dry = mirror(a.path(), b.path(), true);
    assert!(dry.errors.is_empty(), "{:?}", dry.errors);
    assert_eq!(dry.omissions.reported_paths().collect::<Vec<_>>(),
        ["extra_parent/target_link", "node_modules", "target_link"]);
    assert!(!b.path().join("ordinary.txt").exists());
    assert!(b.path().join("extra_parent/extra.txt").exists());
    let result = mirror(a.path(), b.path(), false);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.errors, 0);
    assert!(result.omissions.summary().unwrap().contains("node_modules"));
    assert_eq!(std::fs::read(b.path().join("ordinary.txt")).unwrap(), b"ordinary sync");
    assert_eq!(std::fs::read(b.path().join("node_modules/keep.txt")).unwrap(), b"keep destination");
    assert!(!b.path().join("node_modules/private.txt").exists());
    assert!(!b.path().join("extra_parent/extra.txt").exists());
    assert!(b.path().join("extra_parent").exists());
    assert!(!outside.path().join("not-outside.txt").exists());
    assert_eq!(std::fs::read(outside.path().join("private.txt")).unwrap(), b"outside");
    link_fixture::remove_directory(&a.path().join("node_modules"));
    link_fixture::remove_directory(&b.path().join("extra_parent/target_link"));
    link_fixture::remove_directory(&b.path().join("target_link"));
}
