//! Remote → remote in parts (E13, U48-10): the same ranges read from the
//! source and written to the segmented stage of the target, and the server
//! copy that goes first. Uses the stage in memory of `segments_upload_tests`.
use super::super::test_backend::{job, run, write, Fake};
use super::{lock, offering, pattern, target, SIZE};
use crate::transfer::Endpoint;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

/// Two remotes: a source that reads ranges over `remote_a/big.bin`, and the
/// target above over `remote_b`.
struct Pair {
    _root: tempfile::TempDir,
    source: PathBuf,
    folder: PathBuf,
}

impl Pair {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temp dir");
        let source = root.path().join("remote_a/big.bin");
        write(&source, &pattern(SIZE));
        let folder = root.path().join("remote_b");
        fs::create_dir(&folder).expect("folder");
        Self {
            _root: root,
            source,
            folder,
        }
    }
}

#[test]
fn speed_task_segments_copy_assembles_the_file_across_two_backends() {
    let pair = Pair::new();
    let mut origin = Fake::new("seg-origin");
    origin.ranges.supported = true;
    let (origin, origin_handle) = origin.handle();
    let (_fake, state, handle) = target(Fake::new("seg-dest"), offering());
    let finished = run(job(
        Endpoint::Remote(origin_handle),
        Endpoint::Remote(handle),
        &pair.folder,
        &[&pair.source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    assert_eq!(
        fs::read(pair.folder.join("big.bin")).expect("published"),
        pattern(SIZE)
    );
    // The same ranges were read from one side and written to the other.
    let mut read = origin.ranges.requests.lock().expect("fixture lock").clone();
    let mut written = lock(&state.ranges).clone();
    read.sort_unstable();
    written.sort_unstable();
    assert!(read.len() >= 2, "{read:?}");
    assert_eq!(read, written);
    assert!(lock(&state.stages).is_empty());
}

#[test]
fn speed_task_segments_copy_prefers_server_copy() {
    let pair = Pair::new();
    // One account: the target server copies by itself, before any range.
    let mut origin = Fake::new("seg-same");
    origin.ranges.supported = true;
    let identity = origin.identity.clone();
    let (origin, origin_handle) = origin.handle();
    let mut other = Fake::new("seg-same-target");
    other.identity = identity;
    other.server_copy = true;
    let (fake, state, handle) = target(other, offering());
    let finished = run(job(
        Endpoint::Remote(origin_handle),
        Endpoint::Remote(handle),
        &pair.folder,
        &[&pair.source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(fake.counters.server_copies.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.attempts.load(Ordering::SeqCst),
        0,
        "no segmented stage was opened"
    );
    assert!(origin
        .ranges
        .requests
        .lock()
        .expect("fixture lock")
        .is_empty());
    assert_eq!(
        fs::read(pair.folder.join("big.bin")).expect("published"),
        pattern(SIZE)
    );
}
