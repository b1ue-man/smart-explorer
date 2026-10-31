//! The digest cache next to the data (E9): an unchanged, not racy file is
//! not read again (also after a restart, from the cache file); a written
//! file is; a file changed within the racy slack is never answered from it.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// The cache is one per process: these tests take turns.
static TURN: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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
