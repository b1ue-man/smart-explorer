//! `hash_file_cached` with the digests of a run (`RunDigests`): two asks
//! read a file once, the checksum compare mode reads fresh, and both give
//! the MD5 of the content. (A local sync side has one walk since G6 of the
//! third review, `snapshot_local_keys.rs`, and asks without run digests.)
use super::*;
use crate::agent_proto::RunDigests;
use crate::vfs::LocalBackend;
use std::sync::atomic::AtomicBool;

fn md5_of(content: &[u8]) -> u64 {
    md5_to_u64(&md5::compute(content).0)
}

#[test]
fn abgleich_task_both_walks_of_a_local_side_take_one_digest_per_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.bin");
    std::fs::write(&path, b"content of a").unwrap();
    let path = path.to_string_lossy().replace('\\', "/");
    let backend = LocalBackend::new(&dir.path().to_string_lossy().replace('\\', "/"));
    let cancel = AtomicBool::new(false);
    let run = RunDigests::default();
    let reads = LocalReads {
        fresh: true,
        run: Some(&run),
    };
    let expected = md5_of(b"content of a");
    let first = hash_file_cached(&backend, &path, &cancel, reads).unwrap();
    assert_eq!(first, expected);
    assert_eq!(
        run.held(),
        1,
        "the first walk's digest waits for the second"
    );
    let second = hash_file_cached(&backend, &path, &cancel, reads).unwrap();
    assert_eq!(second, expected);
    assert_eq!(run.held(), 0, "both walks took it");
}

#[test]
fn abgleich_task_two_threads_asking_for_one_file_read_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.bin");
    std::fs::write(&path, vec![7u8; 1 << 20]).unwrap();
    let path = path.to_string_lossy().replace('\\', "/");
    let backend = LocalBackend::new(&dir.path().to_string_lossy().replace('\\', "/"));
    let cancel = AtomicBool::new(false);
    let run = RunDigests::default();
    let reads = LocalReads {
        fresh: true,
        run: Some(&run),
    };
    let expected = md5_of(&vec![7u8; 1 << 20]);
    std::thread::scope(|scope| {
        let ask = || hash_file_cached(&backend, &path, &cancel, reads).unwrap();
        let (left, right) = (scope.spawn(ask), scope.spawn(ask));
        assert_eq!(left.join().unwrap(), expected);
        assert_eq!(right.join().unwrap(), expected);
    });
    assert_eq!(run.held(), 0);
}

#[test]
fn abgleich_task_a_changed_file_is_hashed_again_within_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.bin");
    std::fs::write(&path, b"first").unwrap();
    let path = path.to_string_lossy().replace('\\', "/");
    let backend = LocalBackend::new(&dir.path().to_string_lossy().replace('\\', "/"));
    let cancel = AtomicBool::new(false);
    let run = RunDigests::default();
    let reads = LocalReads {
        fresh: true,
        run: Some(&run),
    };
    assert_eq!(
        hash_file_cached(&backend, &path, &cancel, reads).unwrap(),
        md5_of(b"first")
    );
    // Another size: the run's digest of the old content does not answer.
    std::fs::write(&path, b"second content").unwrap();
    assert_eq!(
        hash_file_cached(&backend, &path, &cancel, reads).unwrap(),
        md5_of(b"second content")
    );
}
