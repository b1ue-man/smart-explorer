//! Copies without a digest consumer go through the kernel (G4 MS4), copies
//! with one keep streaming their MD5; both publish exactly the source bytes.
use super::super::apply_guard::ExpectedFile;
use super::super::apply_transaction::copy;
use super::super::types::{BisyncOptions, PairSide, Throttle};
use super::super::versions::VersionSide;
use crate::vfs::{LocalBackend, StageDurability};
use std::sync::atomic::AtomicBool;

fn slash(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Copies `source/file.bin` to `target/file.bin` and returns the outcome.
fn copy_once(opts: BisyncOptions, throttle: &Throttle, bytes: &[u8]) -> super::CopyOutcome {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("file.bin"), bytes).unwrap();
    let backend = LocalBackend::new("/");
    let root = slash(target.path());
    let side = VersionSide {
        side: PairSide::B,
        backend: &backend,
        root: &root,
    };
    let cancel = AtomicBool::new(false);
    let outcome = copy(
        &backend,
        &slash(&source.path().join("file.bin")),
        ExpectedFile::Unknown,
        &side,
        &slash(&target.path().join("file.bin")),
        ExpectedFile::Missing,
        "file.bin",
        opts,
        None,
        StageDurability::Now,
        throttle,
        &cancel,
        None,
        |_| {},
        |_| {},
    )
    .map_err(|error| error.into_io())
    .unwrap();
    assert_eq!(
        std::fs::read(target.path().join("file.bin")).unwrap(),
        bytes
    );
    let staged: Vec<_> = std::fs::read_dir(target.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(staged.len(), 1, "no stage is left behind: {staged:?}");
    outcome
}

fn data() -> Vec<u8> {
    (0..3 * 1024 * 1024 + 7).map(|n| (n % 253) as u8).collect()
}

#[test]
fn speed_task_sync_copy_without_digest_consumer_skips_the_md5() {
    let bytes = data();
    let outcome = copy_once(BisyncOptions::default(), &Throttle::new(0), &bytes);
    assert_eq!(outcome.bytes, bytes.len() as u64);
    assert_eq!(outcome.destination.size, bytes.len() as u64);
    assert_eq!(outcome.source.hash, 0, "no digest was computed or recorded");
    assert_eq!(outcome.destination.hash, 0);
}

#[test]
fn speed_task_sync_verified_copy_keeps_its_streamed_digest() {
    let bytes = data();
    let opts = BisyncOptions {
        verify: true,
        ..Default::default()
    };
    let outcome = copy_once(opts, &Throttle::new(0), &bytes);
    assert_eq!(outcome.digest, md5::compute(&bytes).0);
    assert_ne!(outcome.destination.hash, 0);
}

#[test]
fn speed_task_sync_bandwidth_limit_keeps_the_limited_stream() {
    let bytes = data();
    // A limit above the file size: the stream is throttled, never stalled.
    let throttle = Throttle::new(64 * 1024 * 1024);
    let outcome = copy_once(BisyncOptions::default(), &throttle, &bytes);
    assert_eq!(outcome.digest, md5::compute(&bytes).0);
}
