//! U14: a local copy runs on the engine on every platform (no read-access
//! request is asked in advance), and the older spool path, still taken by
//! resumed runs, keeps the source's modification time.
use crate::cli::engine_copy::try_copy_with;
use crate::cli::target::Target;
use crate::cli::tree_observer::TreeObserver;
use crate::cli::tree_ops::copy_entry;
use crate::vfs::{Backend, BackendHandle, LocalBackend};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn text(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn local(path: &std::path::Path) -> Target {
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
    Target::with_backend_key(backend, text(path), "local")
}

/// 2001-01-01 00:00:00 UTC, an even number of seconds (FAT stores 2 s).
fn old_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(978_307_200)
}

#[test]
fn wechselmedium_task_local_copy_runs_on_the_engine_without_asking_first() {
    let scratch = tempfile::tempdir().expect("scratch");
    let source = scratch.path().join("source.txt");
    std::fs::write(&source, b"readable").expect("source");
    let copy = scratch.path().join("copy.txt");
    let src = local(&source);
    let meta = src.backend.stat(&src.path).expect("stat");
    let outcome = try_copy_with(
        true,
        &src,
        &meta,
        &meta,
        &local(&copy),
        &text(&copy),
        false,
        &TreeObserver::default(),
    )
    .expect("copy");
    assert!(outcome.is_some(), "the engine copied it, on every platform");
    assert_eq!(std::fs::read(&copy).expect("copy"), b"readable");
}

#[test]
fn wechselmedium_task_spool_path_keeps_the_modification_time() {
    let scratch = tempfile::tempdir().expect("scratch");
    let source = scratch.path().join("source.txt");
    std::fs::write(&source, b"dated").expect("source");
    std::fs::File::options()
        .write(true)
        .open(&source)
        .and_then(|file| file.set_modified(old_time()))
        .expect("source time");
    let copy = scratch.path().join("copy.txt");
    let backend = LocalBackend::new("/");
    copy_entry(
        &backend,
        &text(&source),
        &backend,
        &text(&copy),
        false,
        false,
    )
    .expect("copy");
    assert_eq!(std::fs::read(&copy).expect("copy"), b"dated");
    let copied = std::fs::metadata(&copy)
        .and_then(|metadata| metadata.modified())
        .expect("time");
    assert_eq!(
        copied,
        old_time(),
        "no time of writing (CopyFile2 keeps it)"
    );
}
