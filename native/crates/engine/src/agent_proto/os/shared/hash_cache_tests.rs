//! The digest cache next to the data (E9): an unchanged, not racy file is
//! not read again (also after a restart, from the cache file); a written
//! file is; a file changed within the racy slack is never answered from it.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// The cache is one per process: these tests take turns (`test_turn`).
fn turn() -> MutexGuard<'static, ()> {
    test_turn()
}

fn old_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(3_600))
        .unwrap();
    path
}

fn digest_of(path: &Path, calls: &AtomicUsize) -> String {
    digest(
        super::super::local_platform::path_stamp(path),
        crate::agent_proto::digest::MD5,
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("{}", std::fs::read(path)?.len()))
        },
        || super::super::local_platform::path_stamp(path),
    )
    .unwrap()
}

#[test]
fn abgleich_task_hash_cache_reuses_an_unchanged_file_after_a_restart() {
    let _turn = turn();
    let folder = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    let path = old_file(data.path(), "a.bin", b"abc");
    // Writing set the change time to now: let it age past the racy slack.
    std::thread::sleep(Duration::from_millis(2_100));
    let calls = AtomicUsize::new(0);
    assert_eq!(digest_of(&path, &calls), "3");
    assert_eq!(digest_of(&path, &calls), "3");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the second digest came from the cache"
    );
    flush();
    assert!(folder.path().join(FILE_NAME).exists());
    // A new process: the file is read again, the digest is not computed.
    configure(None);
    configure(Some(folder.path().to_path_buf()));
    assert_eq!(digest_of(&path, &calls), "3");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "loaded from the cache file"
    );
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_reads_a_changed_file_again() {
    let _turn = turn();
    configure(None);
    let data = tempfile::tempdir().unwrap();
    let path = old_file(data.path(), "b.bin", b"abc");
    std::thread::sleep(Duration::from_millis(2_100));
    let calls = AtomicUsize::new(0);
    digest_of(&path, &calls);
    // Same size, same modification time, other content: the change time
    // moved, so the stored digest no longer counts.
    std::fs::write(&path, b"xyz").unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(3_600))
        .unwrap();
    digest_of(&path, &calls);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn abgleich_task_hash_cache_never_answers_a_racy_file() {
    let _turn = turn();
    configure(None);
    let data = tempfile::tempdir().unwrap();
    // Written just now: modification and change time lie within the slack
    // of the moment the digest is taken (Git's racy rule).
    let path = data.path().join("fresh.bin");
    std::fs::write(&path, b"abc").unwrap();
    let calls = AtomicUsize::new(0);
    for _ in 0..3 {
        digest_of(&path, &calls);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

fn fresh_digest_of(path: &Path, calls: &AtomicUsize) -> String {
    digest_fresh(
        super::super::local_platform::path_stamp(path),
        crate::agent_proto::digest::MD5,
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("{}", std::fs::read(path)?.len()))
        },
        || super::super::local_platform::path_stamp(path),
    )
    .unwrap()
}

#[test]
fn abgleich_task_hash_cache_fresh_read_never_takes_a_stored_digest_and_stores_its_own() {
    let _turn = turn();
    configure(None);
    let data = tempfile::tempdir().unwrap();
    let path = old_file(data.path(), "fresh-read.bin", b"abc");
    std::thread::sleep(Duration::from_millis(2_100));
    let calls = AtomicUsize::new(0);
    assert_eq!(digest_of(&path, &calls), "3");
    assert_eq!(fresh_digest_of(&path, &calls), "3");
    assert_eq!(fresh_digest_of(&path, &calls), "3");
    assert_eq!(calls.load(Ordering::SeqCst), 3, "a fresh read reads");
    assert_eq!(digest_of(&path, &calls), "3");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "the digest a fresh read stored answers the next ordinary ask"
    );
}

#[test]
fn abgleich_task_hash_cache_a_file_without_a_stamp_is_read_every_time() {
    let _turn = turn();
    configure(None);
    let calls = AtomicUsize::new(0);
    for _ in 0..3 {
        let digest = digest(
            None,
            crate::agent_proto::digest::MD5,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("x".into())
            },
            || None,
        );
        assert_eq!(digest.unwrap(), "x");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn abgleich_task_hash_cache_collects_digests_and_writes_them_together() {
    let _turn = turn();
    let folder = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    let calls = AtomicUsize::new(0);
    let first = old_file(data.path(), "w1.bin", b"abc");
    let second = old_file(data.path(), "w2.bin", b"abcd");
    let third = old_file(data.path(), "w3.bin", b"abcde");
    std::thread::sleep(Duration::from_millis(2_100));
    let file = folder.path().join(FILE_NAME);
    digest_of(&first, &calls);
    flush();
    let one_entry = std::fs::metadata(&file).unwrap().len();
    // Digests taken within the interval are collected, not written per file.
    digest_of(&second, &calls);
    flush();
    digest_of(&third, &calls);
    flush();
    assert_eq!(std::fs::metadata(&file).unwrap().len(), one_entry);
    // The end of a walk writes them together.
    flush_now();
    assert!(std::fs::metadata(&file).unwrap().len() > 2 * one_entry);
    // Nothing new: nothing is written again.
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    flush_now();
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        written
    );
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_threads_do_not_wait_for_a_write() {
    let _turn = turn();
    let folder = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    let calls = AtomicUsize::new(0);
    let first = old_file(data.path(), "n1.bin", b"abc");
    let second = old_file(data.path(), "n2.bin", b"abcd");
    std::thread::sleep(Duration::from_millis(2_100));
    digest_of(&first, &calls);
    // A write is under way (taken, not yet on disk): the lock is free.
    let pending = take_pending(true).expect("a digest is waiting");
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            digest_of(&second, &calls);
            done.send(()).unwrap();
        });
        finished
            .recv_timeout(Duration::from_secs(5))
            .expect("a hashing thread waited for the write");
    });
    file::replace_if_generation(&pending.folder, pending.expected, &pending.bytes).unwrap();
    assert!(folder.path().join(FILE_NAME).exists());
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_clear_forgets_memory_and_file() {
    let _turn = turn();
    let folder = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    let path = old_file(data.path(), "c.bin", b"abc");
    std::thread::sleep(Duration::from_millis(2_100));
    let calls = AtomicUsize::new(0);
    digest_of(&path, &calls);
    flush_now();
    let before = file::generation_on_disk(folder.path());
    assert_ne!(before, file::NO_GENERATION);
    clear(folder.path()).unwrap();
    let after = file::generation_on_disk(folder.path());
    assert!(
        after != file::NO_GENERATION && after != before,
        "the file names a new generation"
    );
    assert_eq!(
        std::fs::metadata(folder.path().join(FILE_NAME))
            .unwrap()
            .len(),
        file::HEADER_LEN as u64,
        "the file holds no digest"
    );
    digest_of(&path, &calls);
    assert_eq!(calls.load(Ordering::SeqCst), 2, "read again after clearing");
    // Clearing an empty folder is no error.
    clear(&folder.path().join("none")).unwrap();
    configure(None);
}

/// Windows: the stamp is volume serial, 128-bit file ID, size, last write
/// and change time (`GetFileInformationByHandleEx`), so the cache works
/// there too, and a write moves it even when the old times are set back.
#[cfg(windows)]
#[test]
fn abgleich_task_hash_cache_windows_stamp_follows_every_write() {
    let data = tempfile::tempdir().unwrap();
    let path = old_file(data.path(), "w.bin", b"abc");
    let before = super::super::local_platform::path_stamp(&path).expect("NTFS has file IDs");
    let opened = std::fs::File::open(&path).unwrap();
    assert_eq!(
        super::super::local_platform::file_stamp(&opened),
        Some(before)
    );
    drop(opened);
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(&path, b"xyz").unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(3_600))
        .unwrap();
    drop(file);
    let after = super::super::local_platform::path_stamp(&path).unwrap();
    assert_eq!(before[0..3], after[0..3], "same volume, file and size");
    assert_ne!(before[4], after[4], "the change time moved");
}

#[test]
fn abgleich_task_hash_cache_clear_file_names_a_new_generation_without_digests() {
    let folder = tempfile::tempdir().unwrap();
    let first = file::clear_file(folder.path()).unwrap();
    let second = file::clear_file(folder.path()).unwrap();
    assert_ne!(first, file::NO_GENERATION);
    assert_ne!(first, second);
    assert_eq!(file::generation_on_disk(folder.path()), second);
    assert_eq!(
        std::fs::metadata(folder.path().join(FILE_NAME))
            .unwrap()
            .len(),
        file::HEADER_LEN as u64
    );
}

#[test]
fn abgleich_task_hash_cache_a_process_that_predates_a_clear_never_writes_its_digests_back() {
    let _turn = turn();
    let folder = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    let path = old_file(data.path(), "stale.bin", b"abc");
    std::thread::sleep(Duration::from_millis(2_100));
    let calls = AtomicUsize::new(0);
    digest_of(&path, &calls);
    // Another process clears the file while this one holds the digest in
    // memory and has not written it yet.
    let other = file::clear_file(folder.path()).unwrap();
    flush_now();
    assert_eq!(file::generation_on_disk(folder.path()), other);
    assert_eq!(
        std::fs::metadata(folder.path().join(FILE_NAME))
            .unwrap()
            .len(),
        file::HEADER_LEN as u64,
        "the forgotten digest was not written back"
    );
    digest_of(&path, &calls);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "read again after the clear"
    );
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_a_file_of_another_format_names_no_generation() {
    let folder = tempfile::tempdir().unwrap();
    // The format before the generation header: its digests are not trusted.
    let mut old = b"SEHC\x01".to_vec();
    old.extend_from_slice(&[0u8; 64]);
    std::fs::write(folder.path().join(FILE_NAME), old).unwrap();
    assert_eq!(file::generation_on_disk(folder.path()), file::NO_GENERATION);
    assert!(file::generation_in(b"SEHC\x01").is_none());
    // The format with a generation but without blocks (digests in a row):
    // a reader of the block format must not take its records for blocks.
    let mut unframed = b"SEHC\x02".to_vec();
    unframed.extend_from_slice(&[7u8; 16]);
    unframed.extend_from_slice(&[0u8; 64]);
    assert!(file::generation_in(&unframed).is_none());
}

#[test]
fn abgleich_task_hash_cache_a_process_the_clear_cannot_reach_forgets_within_the_check() {
    let _turn = turn();
    let folder = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    let path = old_file(data.path(), "unreached.bin", b"abc");
    std::thread::sleep(Duration::from_millis(2_100));
    let calls = AtomicUsize::new(0);
    digest_of(&path, &calls);
    digest_of(&path, &calls);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "answered from memory");
    // Another process clears the file; this one is never asked and never
    // writes in between.
    file::clear_file(folder.path()).unwrap();
    std::thread::sleep(GENERATION_CHECK + Duration::from_millis(100));
    digest_of(&path, &calls);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the digest held before the clear is not used after the check"
    );
    configure(None);
}
