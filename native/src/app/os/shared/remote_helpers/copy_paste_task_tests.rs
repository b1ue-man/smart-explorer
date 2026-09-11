use super::copy_paste_task_backend::{done, fwd, Sandbox, StageFault, TaskBackend, FOREIGN};
use super::*;
use crate::app::app_models::{TransferMsg, TransferProgress};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn upload(backend: &TaskBackend, source: &Path, target: &Path) -> (TransferProgress, Vec<String>, bool) {
    let (tx, rx) = crossbeam_channel::unbounded();
    upload_paths_progress(backend, &[fwd(source)], &fwd(target), &tx, &AtomicBool::new(false));
    done(&rx)
}

fn download(backend: &TaskBackend, source: &Path, target: &Path) -> (TransferProgress, Vec<String>, bool) {
    let (tx, rx) = crossbeam_channel::unbounded();
    download_paths_progress(backend, &[fwd(source)], &fwd(target), None, &tx, &AtomicBool::new(false));
    done(&rx)
}

fn succeeded(result: (TransferProgress, Vec<String>, bool), files: u64) {
    let (progress, errors, canceled) = result;
    assert!(!canceled);
    assert_eq!(progress.files_done, files, "{errors:?}");
    assert_eq!(progress.errors, 0, "{errors:?}");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_tree_roundtrip_without_final_bulk() {
    let sandbox = Sandbox::new("tree");
    let source = sandbox.dir("source/Vault");
    fs::create_dir_all(source.join("子/empty")).unwrap();
    fs::write(source.join("résumé.txt"), b"root bytes").unwrap();
    fs::write(source.join("子/note.md"), b"nested bytes").unwrap();
    let target = sandbox.dir("target");
    let local = sandbox.dir("download");
    let backend = TaskBackend::new(&target);
    succeeded(upload(&backend, &source, &target), 2);
    assert_eq!(backend.put_calls.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(target.join("Vault/résumé.txt")).unwrap(), b"root bytes");
    assert_eq!(fs::read(target.join("Vault/子/note.md")).unwrap(), b"nested bytes");
    assert!(target.join("Vault/子/empty").is_dir());
    succeeded(download(&backend, &target.join("Vault"), &local), 2);
    assert_eq!(backend.get_calls.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(local.join("Vault/résumé.txt")).unwrap(), b"root bytes");
    assert_eq!(fs::read(local.join("Vault/子/note.md")).unwrap(), b"nested bytes");
    assert!(local.join("Vault/子/empty").is_dir());
    assert_eq!(fs::read(source.join("子/note.md")).unwrap(), b"nested bytes");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_filtered_hierarchy_and_invalid_pairs() {
    let sandbox = Sandbox::new("pairs");
    let source = sandbox.dir("source");
    fs::write(source.join("one"), b"one").unwrap();
    fs::write(source.join("two"), b"two").unwrap();
    let target = sandbox.dir("target");
    fs::create_dir(target.join("Vault")).unwrap();
    fs::write(target.join("Vault/occupied"), FOREIGN).unwrap();
    let backend = TaskBackend::new(&target);
    let (tx, rx) = crossbeam_channel::unbounded();
    upload_pairs_progress(&backend, &[
        (fwd(&source.join("one")), "Vault/子/one.md".into()),
        (fwd(&source.join("two")), "Vault/two.md".into()),
    ], &fwd(&target), &tx, &AtomicBool::new(false));
    succeeded(done(&rx), 2);
    assert_eq!(fs::read(target.join("Vault/occupied")).unwrap(), FOREIGN);
    assert_eq!(fs::read(target.join("Vault (2)/子/one.md")).unwrap(), b"one");
    assert_eq!(fs::read(target.join("Vault (2)/two.md")).unwrap(), b"two");
    assert!(!target.join("one.md").exists());

    let invalid = [
        vec!["../escape"], vec!["/absolute"], vec!["C:/drive"], vec!["a\\b"],
        vec!["a//b"], vec!["a/./b"], vec!["a\0b"], vec!["a", "a/b"],
        vec!["a/b", "a"], vec!["same", "same"],
    ];
    for (index, relative) in invalid.into_iter().enumerate() {
        let target = sandbox.dir(&format!("invalid-{index}"));
        let backend = TaskBackend::new(&target);
        let pairs = relative.into_iter().map(|name| (fwd(&source.join("one")), name.to_string())).collect::<Vec<_>>();
        let (tx, rx) = crossbeam_channel::unbounded();
        upload_pairs_progress(&backend, &pairs, &fwd(&target), &tx, &AtomicBool::new(false));
        let (progress, errors, canceled) = done(&rx);
        assert!(!canceled);
        assert_eq!(progress.files_done, 0);
        assert!(!errors.is_empty(), "invalid pair set {index} was accepted");
        assert_eq!(backend.mutations.load(Ordering::Relaxed), 0);
        assert!(fs::read_dir(target).unwrap().next().is_none());
    }
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_upload_collisions_and_explicit_saveback() {
    let sandbox = Sandbox::new("upload-collision");
    let source = sandbox.0.join("note.txt");
    fs::write(&source, b"source bytes").unwrap();
    let target = sandbox.dir("target");
    fs::write(target.join("note.txt"), FOREIGN).unwrap();
    let backend = TaskBackend::new(&target);
    succeeded(upload(&backend, &source, &target), 1);
    assert_eq!(fs::read(target.join("note.txt")).unwrap(), FOREIGN);
    assert_eq!(fs::read(target.join("note (2).txt")).unwrap(), b"source bytes");
    upload_file(&backend, &source, &fwd(&target.join("note.txt"))).unwrap();
    assert_eq!(fs::read(target.join("note.txt")).unwrap(), b"source bytes");

    let target = sandbox.dir("raced");
    let mut backend = TaskBackend::new(&target);
    backend.race_on_promote = true;
    let (progress, errors, _) = upload(&backend, &source, &target);
    assert_eq!(progress.files_done, 0);
    assert!(errors.iter().any(|error| error.contains("AlreadyExists")), "{errors:?}");
    assert_eq!(fs::read(target.join("note.txt")).unwrap(), FOREIGN);
    assert_eq!(fs::read(&source).unwrap(), b"source bytes");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_download_collisions_and_read_size() {
    let sandbox = Sandbox::new("download-collision");
    let remote = sandbox.dir("remote");
    let source = remote.join("note.txt");
    fs::write(&source, b"source bytes").unwrap();
    for raced in [false, true] {
        let target = sandbox.dir(if raced { "raced" } else { "occupied" });
        let mut backend = TaskBackend::new(&remote);
        if raced { backend.race_on_read = Some(target.join("note.txt")); }
        else { fs::write(target.join("note.txt"), FOREIGN).unwrap(); }
        let (progress, errors, _) = download(&backend, &source, &target);
        assert_eq!(progress.files_done, 0);
        assert!(!errors.is_empty());
        assert_eq!(fs::read(target.join("note.txt")).unwrap(), FOREIGN);
        assert_eq!(fs::read_dir(target).unwrap().count(), 1, "owned download part must be cleaned");
    }
    fs::write(remote.join("zero.txt"), b"").unwrap();
    for unknown in [false, true] {
        let target = sandbox.dir(if unknown { "transformed" } else { "known-zero" });
        let mut backend = TaskBackend::new(&remote);
        backend.read_bytes = Some(b"exported bytes".to_vec());
        backend.unknown_read_size = unknown;
        let result = download(&backend, &remote.join("zero.txt"), &target);
        if unknown {
            succeeded(result, 1);
            assert_eq!(fs::read(target.join("zero.txt")).unwrap(), b"exported bytes");
        } else {
            assert_eq!(result.0.files_done, 0);
            assert!(result.1.iter().any(|error| error.contains("gewachsen")));
            assert!(!target.join("zero.txt").exists());
        }
    }
    assert_eq!(fs::read(&source).unwrap(), b"source bytes");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_stage_failures_preserve_foreign_data() {
    let sandbox = Sandbox::new("stages");
    let source = sandbox.0.join("note.txt");
    fs::write(&source, b"source bytes").unwrap();
    for (index, fault) in [StageFault::Unsupported, StageFault::ForeignOnOpen, StageFault::ForeignOnFlush].into_iter().enumerate() {
        let target = sandbox.dir(&format!("target-{index}"));
        let mut backend = TaskBackend::new(&target);
        backend.stage_fault = fault;
        let (progress, errors, canceled) = upload(&backend, &source, &target);
        assert!(!canceled);
        assert_eq!(progress.files_done, 0);
        assert!(!errors.is_empty());
        let stages = backend.stages.lock().unwrap();
        assert_eq!(stages.len(), 1, "no unsafe fallback or blind retry");
        assert!(errors.iter().any(|error| error.contains(&fwd(&stages[0]))), "{errors:?}");
        if fault == StageFault::Unsupported { assert!(!stages[0].exists()); }
        else { assert_eq!(fs::read(&stages[0]).unwrap(), FOREIGN); }
        assert_eq!(backend.remove_calls.load(Ordering::Relaxed), 0);
        assert_eq!(backend.replacing_writes.load(Ordering::Relaxed), 0);
        assert!(!target.join("note.txt").exists());
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
    }
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_changed_upload_source_is_not_published() {
    let sandbox = Sandbox::new("changed-source");
    let source = sandbox.0.join("note.txt");
    fs::write(&source, b"original").unwrap();
    let target = sandbox.dir("target");
    let mut backend = TaskBackend::new(&target);
    backend.mutate_source = Some(source.clone());
    let (progress, errors, _) = upload(&backend, &source, &target);
    assert_eq!(progress.files_done, 0);
    assert!(errors.iter().any(|error| error.contains("gewachsen") || error.contains("geändert")), "{errors:?}");
    assert!(!target.join("note.txt").exists());
    assert_eq!(fs::read(source).unwrap(), b"originalgrew", "fixture-induced source change is retained");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_cancellation_counts_acknowledged_commit() {
    let sandbox = Sandbox::new("commit-cancel");
    let source = sandbox.dir("source");
    fs::write(source.join("one.txt"), b"one").unwrap();
    fs::write(source.join("two.txt"), b"two").unwrap();
    let target = sandbox.dir("target");
    let cancel = Arc::new(AtomicBool::new(false));
    let mut backend = TaskBackend::new(&target);
    backend.cancel_after_promote = Some(cancel.clone());
    let (tx, rx) = crossbeam_channel::unbounded();
    upload_paths_progress(&backend, &[fwd(&source.join("one.txt")), fwd(&source.join("two.txt"))],
        &fwd(&target), &tx, &cancel);
    let (progress, errors, canceled) = done(&rx);
    assert!(canceled);
    assert_eq!(progress.files_done, 1);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(fs::read(target.join("one.txt")).unwrap(), b"one");
    assert!(!target.join("two.txt").exists());
    assert_eq!(backend.stages.lock().unwrap().len(), 1);
    assert_eq!(fs::read(source.join("two.txt")).unwrap(), b"two");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_private_clipboard_bulk_and_honest_failure() {
    let sandbox = Sandbox::new("private-bulk");
    let source = sandbox.dir("source/Vault");
    fs::create_dir(source.join("empty")).unwrap();
    fs::write(source.join("note.txt"), b"clipboard").unwrap();
    for mode in 0..3 {
        let mut backend = TaskBackend::new(&source);
        backend.bulk_mismatch = mode == 1;
        backend.bulk_error = mode == 2;
        let result = download_remote_clipboard_items(&backend, &[(fwd(&source), "Vault".into(), true)], None);
        assert_eq!(backend.get_calls.load(Ordering::Relaxed), 1);
        assert_eq!(backend.read_calls.load(Ordering::Relaxed), 0, "no per-file retry after dispatched bulk");
        if mode == 0 {
            let paths = result.unwrap();
            assert_eq!(paths.len(), 1);
            let local = Path::new(&paths[0]);
            assert_eq!(fs::read(local.join("note.txt")).unwrap(), b"clipboard");
            assert!(local.join("empty").is_dir());
            cleanup_temp_copy(local);
        } else {
            assert!(result.is_err(), "ambiguous bulk result must not be replayed as success");
            assert!(!backend.bulk_destinations.lock().unwrap()[0].exists(), "failed owned clipboard tree must be cleaned");
        }
        assert_eq!(fs::read(source.join("note.txt")).unwrap(), b"clipboard");
    }
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_same_backend_uses_protected_bridge() {
    let sandbox = Sandbox::new("same-backend");
    let source = sandbox.dir("source");
    let target = sandbox.dir("target");
    fs::write(source.join("note.txt"), b"source bytes").unwrap();
    let mut backend = TaskBackend::new(&sandbox.0);
    let healthy = sandbox.dir("healthy");
    let (tx, rx) = crossbeam_channel::unbounded();
    copy_remote_paths_progress(&backend, &[fwd(&source.join("note.txt"))], &backend,
        &fwd(&healthy), true, None, &tx, &AtomicBool::new(false));
    succeeded(done(&rx), 1);
    assert_eq!(fs::read(healthy.join("note.txt")).unwrap(), b"source bytes");
    backend.race_on_promote = true;
    let (tx, rx) = crossbeam_channel::unbounded::<TransferMsg>();
    copy_remote_paths_progress(&backend, &[fwd(&source.join("note.txt"))], &backend,
        &fwd(&target), true, None, &tx, &AtomicBool::new(false));
    let (progress, errors, _) = done(&rx);
    assert_eq!(progress.files_done, 0);
    assert_eq!(progress.bytes_total, 2 * b"source bytes".len() as u64);
    assert!(!errors.is_empty());
    assert_eq!(backend.copy_calls.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(target.join("note.txt")).unwrap(), FOREIGN);
    assert_eq!(fs::read(source.join("note.txt")).unwrap(), b"source bytes");
}
