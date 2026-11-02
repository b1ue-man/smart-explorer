//! Acceptance signal `drive_task_boundary_delegates_every_operation` of
//! DRV-M2: every method of `MountFilesystem` reaches the engine, observed
//! through its effect on the remote fixture or its engine result, while the
//! caller holds nothing but `&dyn MountFilesystem`.
use super::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use super::{
    FlushOutcome, LockReadiness, MountEngine, MountFilesystem, MountId, MountMode,
    MountRuntimeConfig, NamespaceOutcome, OpenDisposition, OpenFileOptions, PathDialect,
    PendingAttributes, RenameOutcome,
};
use crate::vfs::Backend;
use std::io;

#[test]
fn drive_task_boundary_delegates_every_operation() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/seed.txt", b"seed");
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse("boundary").unwrap(), MountMode::ReadWrite),
        backend.clone(),
        directory.path(),
    )
    .unwrap();
    let fs: &dyn MountFilesystem = &engine;

    let facts = fs.volume_facts();
    assert_eq!(facts.id.as_str(), "boundary");
    assert_eq!(facts.root.as_str(), "/");
    assert_eq!(facts.dialect, PathDialect::Windows);
    assert!(facts.case_sensitive);
    assert!(!facts.read_only);
    assert_eq!(
        fs.project("\\docs\\a.txt").unwrap().backend(),
        "/docs/a.txt"
    );
    fs.preload_metadata().unwrap();

    // Namespace and data through one handle.
    assert_eq!(fs.mkdir("\\docs").unwrap(), NamespaceOutcome::Complete);
    assert!(backend.stat("/docs").unwrap().is_dir);
    let handle = fs
        .open_file(
            "\\docs\\a.txt",
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::CreateNew,
            },
        )
        .unwrap();
    assert_eq!(fs.write(handle, 0, b"hello").unwrap(), 5);
    assert_eq!(fs.append(handle, b"!!").unwrap(), 2);
    assert_eq!(fs.len(handle).unwrap(), 7);
    fs.truncate(handle, 6).unwrap();
    let mut buffer = [0u8; 16];
    let read = fs.read(handle, 0, &mut buffer).unwrap();
    assert_eq!(&buffer[..read], b"hello!");
    assert_eq!(fs.stat_handle(handle).unwrap().size, 6);
    assert_eq!(fs.dirty_entries().unwrap().len(), 1);
    assert_eq!(
        fs.flush_path("\\docs\\a.txt").unwrap(),
        FlushOutcome::Committed
    );
    assert_eq!(backend.bytes("/docs/a.txt"), b"hello!");
    assert_eq!(fs.flush(handle).unwrap(), FlushOutcome::NoChanges);
    let refused = fs
        .set_pending_attributes(
            handle,
            PendingAttributes {
                mtime_ms: Some(1_700_000_000_000),
                mode: Some(0o644),
            },
        )
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::Unsupported);
    fs.close(handle).unwrap();
    assert!(fs.dirty_entries().unwrap().is_empty());

    // Metadata.
    assert_eq!(fs.stat("\\docs\\a.txt").unwrap().size, 6);
    assert_eq!(fs.stat_cached("\\seed.txt").unwrap().size, 4);
    let names = fs
        .list_dir("\\docs")
        .unwrap()
        .into_iter()
        .map(|metadata| metadata.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["a.txt"]);
    assert_eq!(fs.list_dir_cached("\\docs").unwrap().len(), 1);

    // Both rename flavours.
    assert_eq!(
        fs.rename("\\docs\\a.txt", "\\docs\\b.txt", false).unwrap(),
        RenameOutcome::Complete
    );
    assert_eq!(
        fs.rename_with_shared_destination("\\docs\\b.txt", "\\docs\\c.txt", false, false)
            .unwrap(),
        RenameOutcome::Complete
    );
    assert_eq!(backend.bytes("/docs/c.txt"), b"hello!");
    assert!(backend.stat("/docs/a.txt").is_err());

    // A lazily opened handle fetches on its first read.
    let metadata = fs.stat("\\docs\\c.txt").unwrap();
    let lazy = fs
        .open_metadata_file("\\docs\\c.txt", metadata, false)
        .unwrap();
    let read = fs.read(lazy, 0, &mut buffer).unwrap();
    assert_eq!(&buffer[..read], b"hello!");
    fs.close(lazy).unwrap();
    assert!(fs.evict_clean("\\docs\\c.txt").is_ok());

    // The delete protocol: begin, cancel, begin, commit; then the one-call form.
    let token = fs.begin_delete("\\docs\\c.txt", false).unwrap();
    assert_eq!(fs.pending_deletes().unwrap(), [token]);
    fs.cancel_delete(token).unwrap();
    assert!(fs.pending_deletes().unwrap().is_empty());
    assert_eq!(backend.bytes("/docs/c.txt"), b"hello!");
    let token = fs.begin_delete("\\docs\\c.txt", false).unwrap();
    fs.commit_delete(token).unwrap();
    assert!(backend.stat("/docs/c.txt").is_err());
    fs.delete("\\docs", true).unwrap();
    assert!(backend.stat("/docs").is_err());

    // Recovery, maintenance and the background metadata work.
    fs.retry_pending_changes().unwrap();
    fs.maintain_cache().unwrap();
    fs.preload_metadata_while(&|| false).unwrap();
    fs.refresh_metadata_while(&|| false).unwrap();
    fs.refresh_metadata().unwrap();
    assert!(fs.drain_metadata_changes(16).is_ok());
    fs.invalidate_content("/seed.txt", false);
    assert_eq!(
        fs.prepare_lock().unwrap(),
        LockReadiness {
            pending: 0,
            detail: None
        }
    );
}
