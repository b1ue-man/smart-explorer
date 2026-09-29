//! Reading both sides at once: a failing side stops the other and is the one
//! reported; the user's cancel reaches both walks.
use super::super::incremental::SyncEndpoints;
use super::super::test_remote::{FakeRemote, REMOTE_ROOT};
use super::super::*;
use super::read_pair;
use crate::vfs::LocalBackend;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Twelve folders of one file each: a walk of at least thirteen listings.
fn populate(root: &Path) {
    for folder in 0..12 {
        let dir = root.join(format!("folder{folder}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("file.txt"), b"x").unwrap();
    }
}

#[test]
fn transfer_engine_task_bisync_read_pair_failure_stops_the_other_side() {
    let a = tempfile::tempdir().unwrap();
    let holder = tempfile::tempdir().unwrap();
    populate(a.path());
    let slow = FakeRemote::new(a.path(), "pair-slow").with_delay(Duration::from_millis(40));
    // Side B's folder does not exist: its walk fails at once.
    let missing = forward(&holder.path().join("missing"));
    let gone = LocalBackend::new(&missing);
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);

    let error = read_pair(
        SyncEndpoints::new(&slow, REMOTE_ROOT, &gone, &missing),
        BisyncOptions::default(),
        &cancel,
        &filter,
        &Baseline::new(),
    )
    .err()
    .expect("a side that cannot be read fails the pair");

    assert_eq!(error.0, missing);
    assert!(slow.calls.list.load(Ordering::SeqCst) < 13);
    assert!(!cancel.load(Ordering::SeqCst));
}

#[test]
fn transfer_engine_task_bisync_read_pair_cancel_reaches_both_sides() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    populate(a.path());
    populate(b.path());
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    // The user cancels as soon as side A lists its first folder.
    let side_a = FakeRemote::new(a.path(), "pair-a")
        .with_delay(Duration::from_millis(40))
        .with_hook(Arc::new(move |operation: &str, _path: &str| {
            if operation == "list" {
                flag.store(true, Ordering::SeqCst);
            }
        }));
    let side_b = FakeRemote::new(b.path(), "pair-b").with_delay(Duration::from_millis(40));
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);

    let error = read_pair(
        SyncEndpoints::new(&side_a, REMOTE_ROOT, &side_b, REMOTE_ROOT),
        BisyncOptions::default(),
        &cancel,
        &filter,
        &Baseline::new(),
    )
    .err()
    .expect("a canceled read never yields trees");

    assert_eq!(error.0, REMOTE_ROOT);
    assert!(error.1.contains("canceled"), "{error:?}");
    assert!(side_b.calls.list.load(Ordering::SeqCst) < 13);
}
