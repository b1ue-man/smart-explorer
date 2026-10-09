//! Saves through the drive never leave the upload stage spelling
//! (`<name>.se-mount-<16 hex>`) next to the user's file (user report
//! 2026-10-09: „Drucken als PDF“ onto the drive).
use super::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use super::{
    FlushOutcome, HandleId, MountCachePolicy, MountEngine, MountId, MountMode, MountRuntimeConfig,
    OpenDisposition, OpenFileOptions,
};
use crate::vfs::Backend;
use std::sync::{mpsc, Arc};
use std::time::Duration;

fn new_engine(directory: &FixtureDirectory, backend: &Arc<OptimizationBackend>) -> MountEngine {
    let config =
        MountRuntimeConfig::new(MountId::parse("save-task").unwrap(), MountMode::ReadWrite)
            .with_cache_policy(MountCachePolicy::new(0).unwrap());
    let engine = MountEngine::open_host_cache(config, backend.clone(), directory.path()).unwrap();
    engine.prepare_host_remote().unwrap();
    engine
}

fn open_writable(engine: &MountEngine, path: &str) -> HandleId {
    engine
        .open_file(
            path,
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::OpenExisting,
            },
        )
        .unwrap()
}

fn names(backend: &OptimizationBackend) -> Vec<String> {
    backend
        .list_dir("/")
        .unwrap()
        .into_iter()
        .map(|meta| meta.name)
        .collect()
}

#[test]
fn mount_save_task_failed_upload_leaves_no_stage_and_retries_cleanly() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/Scan.pdf", b"old");
    let engine = new_engine(&directory, &backend);
    let handle = open_writable(&engine, "\\Scan.pdf");
    engine.write(handle, 0, b"new content").unwrap();
    backend.fail_upload(true);
    assert!(engine.flush(handle).is_err());
    assert_eq!(backend.bytes("/Scan.pdf"), b"old");
    assert_eq!(
        names(&backend),
        ["Scan.pdf"],
        "no stage sibling after a failed upload"
    );
    backend.fail_upload(false);
    assert_eq!(engine.flush(handle).unwrap(), FlushOutcome::Committed);
    assert_eq!(backend.bytes("/Scan.pdf"), b"new content");
    assert_eq!(names(&backend), ["Scan.pdf"]);
    engine.close(handle).unwrap();
}

#[test]
fn mount_save_task_remote_change_during_save_becomes_a_typed_conflict_copy() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/Rechnung.pdf", b"old");
    let engine = Arc::new(new_engine(&directory, &backend));
    let handle = open_writable(&engine, "\\Rechnung.pdf");
    engine.write(handle, 0, b"mine").unwrap();
    // The first conflict check sees the old file; the remote then changes
    // while this save is uploading.
    let gate = backend.gate_stat("/Rechnung.pdf", 0);
    let (flush_tx, flush_rx) = mpsc::channel();
    let flushing = Arc::clone(&engine);
    let flusher = std::thread::spawn(move || {
        flush_tx.send(flushing.flush(handle)).unwrap();
    });
    gate.wait();
    backend.put("/Rechnung.pdf", b"theirs!");
    gate.release();
    let outcome = flush_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("flush finished")
        .unwrap();
    flusher.join().unwrap();
    assert!(matches!(outcome, FlushOutcome::Conflict(_)), "{outcome:?}");
    assert_eq!(
        backend.bytes("/Rechnung.pdf"),
        b"theirs!",
        "the other change stays"
    );
    let names = names(&backend);
    assert!(
        names.iter().all(|name| !name.contains(".se-mount-")),
        "{names:?}"
    );
    let copy = names
        .iter()
        .find(|name| name.starts_with("Rechnung (Konflikt ") && name.ends_with(").pdf"))
        .unwrap_or_else(|| panic!("conflict copy keeps name and type: {names:?}"));
    assert_eq!(backend.bytes(&format!("/{copy}")), b"mine");
    engine.close(handle).unwrap();
}
