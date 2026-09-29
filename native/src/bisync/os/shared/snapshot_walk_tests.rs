//! The snapshot walk of a remote side lists folders concurrently under its
//! flow and yields exactly the tree, omissions and hashes of a local walk of
//! the same folder; a canceled walk still fails closed.
use super::super::test_remote::{FakeRemote, REMOTE_ROOT};
use super::super::*;
use super::walk_snapshot;
use crate::bisync::link_fixture;
use crate::vfs::LocalBackend;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn write(root: &Path, rel: &str, content: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Eight sibling folders (listed concurrently), a deep branch, an ignored
/// temp file and a linked folder.
fn populate(root: &Path, outside: &Path) {
    for folder in 0..8 {
        for file in 0..3 {
            write(
                root,
                &format!("folder{folder}/file{file}.txt"),
                format!("{folder}-{file}").as_bytes(),
            );
        }
    }
    write(root, "a/b/c/d/deep.txt", b"deep");
    write(root, "a/skip.tmp", b"ignored");
    link_fixture::directory(outside, &root.join("linked"));
}

#[test]
fn transfer_engine_task_bisync_remote_walk_matches_local_walk() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private.txt"), b"outside").unwrap();
    populate(root.path(), outside.path());
    let mut builder = globset::GlobSetBuilder::new();
    builder.add(globset::Glob::new("*.tmp").unwrap());
    let globs = builder.build().unwrap();
    let filter = WalkFilter::basic(true, &globs);
    let cancel = AtomicBool::new(false);
    let local = LocalBackend::new(&forward(root.path()));
    let remote = FakeRemote::new(root.path(), "walk").with_delay(Duration::from_millis(30));

    let expected = walk_snapshot(
        &local,
        &forward(root.path()),
        &cancel,
        &filter,
        HashMode::None,
        None,
        false,
        false,
    )
    .unwrap();
    let walked = walk_snapshot(
        &remote,
        REMOTE_ROOT,
        &cancel,
        &filter,
        HashMode::None,
        None,
        false,
        false,
    )
    .unwrap();

    assert_eq!(walked.tree, expected.tree);
    assert_eq!(walked.tree.len(), 25);
    assert!(!walked.tree.contains_key("a/skip.tmp"));
    assert_eq!(
        walked.omissions.reported_paths().collect::<Vec<_>>(),
        expected.omissions.reported_paths().collect::<Vec<_>>()
    );
    assert_eq!(
        walked.omissions.reported_paths().collect::<Vec<_>>(),
        ["linked"]
    );
    // Folders were listed concurrently (the old walk used `parallelism()`,
    // which is 1 for this remote) and every folder exactly once.
    assert!(remote.calls.peak_lists.load(Ordering::SeqCst) >= 2);
    assert_eq!(remote.calls.list.load(Ordering::SeqCst), 13);
    link_fixture::remove_directory(&root.path().join("linked"));
}

#[test]
fn transfer_engine_task_bisync_remote_checksum_walk_matches_local_hashes() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..6 {
        write(
            root.path(),
            &format!("sub{}/f{index}.bin", index % 2),
            &[index as u8].repeat(70_000 + index * 1_000),
        );
    }
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let cancel = AtomicBool::new(false);
    let local = LocalBackend::new(&forward(root.path()));
    let remote = FakeRemote::new(root.path(), "checksum");

    let expected = walk_files(
        &local,
        &forward(root.path()),
        &cancel,
        &filter,
        HashMode::FullFresh,
        None,
    )
    .unwrap();
    let walked = walk_files(
        &remote,
        REMOTE_ROOT,
        &cancel,
        &filter,
        HashMode::FullFresh,
        None,
    )
    .unwrap();

    assert_eq!(walked, expected);
    assert_eq!(walked.len(), 6);
    assert!(walked.values().all(|sig| sig.hash != 0));
    assert_eq!(remote.calls.reads.load(Ordering::SeqCst), 6);
}

#[test]
fn transfer_engine_task_bisync_remote_walk_cancel_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    for folder in 0..6 {
        write(root.path(), &format!("folder{folder}/file.txt"), b"x");
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let remote = FakeRemote::new(root.path(), "walk-cancel")
        .with_delay(Duration::from_millis(10))
        .with_hook(Arc::new(move |operation: &str, _path: &str| {
            if operation == "list" {
                flag.store(true, Ordering::SeqCst);
            }
        }));
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);

    let error = walk_files(&remote, REMOTE_ROOT, &cancel, &filter, HashMode::None, None)
        .expect_err("a canceled walk never yields a tree");

    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
}
