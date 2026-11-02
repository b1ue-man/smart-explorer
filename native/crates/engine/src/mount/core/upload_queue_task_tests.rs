//! G4 ML1 invariants of the asynchronous write-back: `close` returns before
//! the upload, the journal entry is the source of truth until the upload is
//! confirmed (a crash in between resumes on the next engine), and a remote
//! change found by a background upload stays a conflict as before.
use super::super::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use super::super::{
    EntryCondition, FlushOutcome, MountEngine, MountId, MountMode, MountRuntimeConfig,
    OpenDisposition, OpenFileOptions, PathDialect,
};
use crate::vfs::Backend;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn engine(directory: &FixtureDirectory, backend: &Arc<OptimizationBackend>) -> Arc<MountEngine> {
    let handle: crate::vfs::BackendHandle = backend.clone();
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse("write-back").unwrap(), MountMode::ReadWrite)
            .with_path_dialect(PathDialect::Posix),
        handle,
        directory.path(),
    )
    .unwrap();
    let engine = Arc::new(engine);
    engine.start_write_back();
    engine
}

fn save(engine: &MountEngine, path: &str, disposition: OpenDisposition, bytes: &[u8]) {
    let handle = engine
        .open_file(
            path,
            OpenFileOptions {
                writable: true,
                disposition,
            },
        )
        .unwrap();
    engine.write(handle, 0, bytes).unwrap();
    assert_eq!(
        engine.flush_on_close(handle).unwrap(),
        FlushOutcome::NoChanges,
        "close returns once the file is in the local write cache"
    );
    engine.close(handle).unwrap();
}

fn wait_until(mut done: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn speed_task_drive_write_back_resumes_after_a_crash() -> io::Result<()> {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.fail_upload(true);
    let first = engine(&directory, &backend);
    save(&first, "/new.txt", OpenDisposition::CreateNew, b"hello");
    assert!(
        backend.stat("/new.txt").is_err(),
        "nothing reached the remote yet"
    );
    assert_eq!(
        first.dirty_entries()?,
        [("/new.txt".to_string(), EntryCondition::Dirty)],
        "the journal entry owns the change until the upload is confirmed"
    );
    wait_until(
        || {
            first
                .upload_status()
                .is_some_and(|status| status.pending_files == 1 && status.last_error.is_some())
        },
        "the failed upload is reported as pending with its error",
    );
    // A crash: the engine ends without draining its queue.
    let gone = Arc::downgrade(&first);
    drop(first);
    wait_until(|| gone.upgrade().is_none(), "the workers let the engine go");
    backend.fail_upload(false);

    let second = engine(&directory, &backend);
    assert_eq!(
        second.dirty_entries()?,
        [("/new.txt".to_string(), EntryCondition::Dirty)],
        "the next engine recovers the pending change from the journal"
    );
    second.retry_pending_changes()?;
    second.finish_write_back();
    assert_eq!(backend.bytes("/new.txt"), b"hello");
    assert!(second.dirty_entries()?.is_empty());
    Ok(())
}

#[test]
fn speed_task_drive_write_back_keeps_remote_change_as_conflict() -> io::Result<()> {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/shared.txt", b"old");
    backend.fail_upload(true);
    let engine = engine(&directory, &backend);
    save(
        &engine,
        "/shared.txt",
        OpenDisposition::OpenExisting,
        b"mine",
    );
    wait_until(
        || {
            engine
                .upload_status()
                .is_some_and(|status| status.last_error.is_some())
        },
        "the first upload attempt failed",
    );
    backend.fail_upload(false);
    // Another writer changes the remote file before the upload succeeds.
    backend.put("/shared.txt", b"theirs");
    engine.retry_pending_changes()?;
    engine.finish_write_back();
    assert_eq!(
        backend.bytes("/shared.txt"),
        b"theirs",
        "a background upload never overwrites a remote change"
    );
    let conflicts = engine.take_upload_conflicts();
    assert_eq!(conflicts.len(), 1, "the conflict is reported to the host");
    assert_eq!(conflicts[0].0, "/shared.txt");
    assert!(matches!(conflicts[0].1, EntryCondition::Conflict(_)));
    Ok(())
}

#[test]
fn speed_task_drive_write_back_publishes_newest_content() -> io::Result<()> {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    let engine = engine(&directory, &backend);
    save(&engine, "/doc.txt", OpenDisposition::CreateNew, b"first");
    save(
        &engine,
        "/doc.txt",
        OpenDisposition::TruncateExisting,
        b"second",
    );
    engine.finish_write_back();
    assert_eq!(backend.bytes("/doc.txt"), b"second");
    assert!(engine.dirty_entries()?.is_empty());
    for entry in backend.list_dir("/")? {
        assert!(
            !entry.name.contains(".se-mount-"),
            "no upload stage stays next to the file"
        );
    }
    Ok(())
}
