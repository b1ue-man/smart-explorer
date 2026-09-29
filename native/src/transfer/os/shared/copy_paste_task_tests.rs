use super::copy_paste_task_backend::{
    done, download, fwd, same_backend_copy, succeeded, upload, Sandbox, StageFault, TaskBackend,
    FOREIGN,
};
use super::*;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
    assert_eq!(
        fs::read(target.join("Vault/résumé.txt")).unwrap(),
        b"root bytes"
    );
    assert_eq!(
        fs::read(target.join("Vault/子/note.md")).unwrap(),
        b"nested bytes"
    );
    assert!(target.join("Vault/子/empty").is_dir());
    succeeded(download(&backend, &target.join("Vault"), &local), 2);
    assert_eq!(backend.get_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        fs::read(local.join("Vault/résumé.txt")).unwrap(),
        b"root bytes"
    );
    assert_eq!(
        fs::read(local.join("Vault/子/note.md")).unwrap(),
        b"nested bytes"
    );
    assert!(local.join("Vault/子/empty").is_dir());
    assert_eq!(
        fs::read(source.join("子/note.md")).unwrap(),
        b"nested bytes"
    );
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
    upload_pairs_progress(
        &backend,
        &[
            (fwd(&source.join("one")), "Vault/子/one.md".into()),
            (fwd(&source.join("two")), "Vault/two.md".into()),
        ],
        &fwd(&target),
        &tx,
        &AtomicBool::new(false),
    );
    succeeded(done(&rx), 2);
    assert_eq!(fs::read(target.join("Vault/occupied")).unwrap(), FOREIGN);
    assert_eq!(
        fs::read(target.join("Vault (2)/子/one.md")).unwrap(),
        b"one"
    );
    assert_eq!(fs::read(target.join("Vault (2)/two.md")).unwrap(), b"two");
    assert!(!target.join("one.md").exists());

    let invalid = [
        vec!["../escape"],
        vec!["/absolute"],
        vec!["C:/drive"],
        vec!["a\\b"],
        vec!["a//b"],
        vec!["a/./b"],
        vec!["a\0b"],
        vec!["a", "a/b"],
        vec!["a/b", "a"],
        vec!["same", "same"],
    ];
    for (index, relative) in invalid.into_iter().enumerate() {
        let target = sandbox.dir(&format!("invalid-{index}"));
        let backend = TaskBackend::new(&target);
        let pairs = relative
            .into_iter()
            .map(|name| (fwd(&source.join("one")), name.to_string()))
            .collect::<Vec<_>>();
        let (tx, rx) = crossbeam_channel::unbounded();
        upload_pairs_progress(
            &backend,
            &pairs,
            &fwd(&target),
            &tx,
            &AtomicBool::new(false),
        );
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
    assert_eq!(
        fs::read(target.join("note (2).txt")).unwrap(),
        b"source bytes"
    );
    upload_file(&backend, &source, &fwd(&target.join("note.txt"))).unwrap();
    assert_eq!(fs::read(target.join("note.txt")).unwrap(), b"source bytes");

    // A foreign file appears at the destination after the target listing:
    // the create-only publication fails for that name and the copy takes the
    // next free "Name (n)" (umsetzung.md Block A.6, K14). Formerly the file
    // failed; either way the foreign file is never replaced.
    let target = sandbox.dir("raced");
    let mut backend = TaskBackend::new(&target);
    backend.race_on_promote = true;
    succeeded(upload(&backend, &source, &target), 1);
    assert_eq!(fs::read(target.join("note.txt")).unwrap(), FOREIGN);
    assert_eq!(
        fs::read(target.join("note (2).txt")).unwrap(),
        b"source bytes"
    );
    assert_eq!(backend.replacing_writes.load(Ordering::Relaxed), 0);
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
        if raced {
            backend.race_on_read = Some(target.join("note.txt"));
        } else {
            fs::write(target.join("note.txt"), FOREIGN).unwrap();
        }
        // A taken name at a local target keeps both: the download lands as
        // "note (2).txt" (spec F7, umsetzung.md Block A.2/A.6; formerly an
        // error). The foreign file, present before or created during the
        // read, is never replaced.
        succeeded(download(&backend, &source, &target), 1);
        assert_eq!(fs::read(target.join("note.txt")).unwrap(), FOREIGN);
        assert_eq!(
            fs::read(target.join("note (2).txt")).unwrap(),
            b"source bytes"
        );
        assert_eq!(
            fs::read_dir(target).unwrap().count(),
            2,
            "owned download part must be cleaned"
        );
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
            assert_eq!(
                fs::read(target.join("zero.txt")).unwrap(),
                b"exported bytes"
            );
        } else {
            assert_eq!(result.0.files_done, 0);
            assert!(result.1.iter().any(|error| error.contains("gewachsen")));
            assert!(!target.join("zero.txt").exists());
            assert_eq!(
                fs::read_dir(&target).unwrap().count(),
                0,
                "a grown source leaves no part behind"
            );
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
    for (index, fault) in [
        StageFault::Unsupported,
        StageFault::ForeignOnOpen,
        StageFault::ForeignOnFlush,
    ]
    .into_iter()
    .enumerate()
    {
        let target = sandbox.dir(&format!("target-{index}"));
        let mut backend = TaskBackend::new(&target);
        backend.stage_fault = fault;
        let (progress, errors, canceled) = upload(&backend, &source, &target);
        assert!(!canceled);
        assert_eq!(progress.files_done, 0);
        assert!(!errors.is_empty());
        let stages = backend.stages.lock().unwrap().clone();
        match fault {
            StageFault::Unsupported => {
                // No exclusive writer: the file fails, never a replacing
                // fallback, and nothing is created.
                assert_eq!(stages.len(), 1, "no unsafe fallback");
                assert!(
                    errors.iter().any(|error| error.contains(&fwd(&stages[0]))),
                    "{errors:?}"
                );
                assert!(!stages[0].exists());
            }
            StageFault::ForeignOnOpen => {
                // A taken stage name is never adopted: the engine opens a new
                // random name instead (umsetzung.md Block A.6; formerly the
                // file failed at once). Every foreign file stays as it was.
                assert!(stages.len() > 1, "a fresh name after a taken one");
                let mut distinct = stages.clone();
                distinct.sort();
                distinct.dedup();
                assert_eq!(distinct.len(), stages.len(), "never the same name twice");
                for stage in &stages {
                    assert_eq!(fs::read(stage).unwrap(), FOREIGN);
                }
            }
            StageFault::ForeignOnFlush | StageFault::None => {
                // A failed stage flush published nothing, so the transient
                // failure is retried exactly once with a new stage
                // (umsetzung.md Block A.7; formerly never). Neither stage can
                // be proven ours: both stay and are reported (K17).
                assert_eq!(stages.len(), 2, "one retry, not more");
                for stage in &stages {
                    assert_eq!(fs::read(stage).unwrap(), FOREIGN);
                    assert!(
                        errors.iter().any(|error| error.contains(&fwd(stage))),
                        "{errors:?}"
                    );
                }
            }
        }
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
    assert!(
        errors
            .iter()
            .any(|error| error.contains("gewachsen") || error.contains("geändert")),
        "{errors:?}"
    );
    assert!(!target.join("note.txt").exists());
    assert_eq!(
        fs::read(source).unwrap(),
        b"originalgrew",
        "fixture-induced source change is retained"
    );
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
    // One operation at a time on this connection, so the other file can
    // only start after the first commit was acknowledged and the cancel was
    // requested; the engine otherwise runs files in parallel (Block A.5).
    backend.ceiling = Some(1);
    // A stage the canceled file may already have opened is removed.
    backend.discard_stages = true;
    let (tx, rx) = crossbeam_channel::unbounded();
    upload_paths_progress(
        &backend,
        &[fwd(&source.join("one.txt")), fwd(&source.join("two.txt"))],
        &fwd(&target),
        &tx,
        &cancel,
    );
    let (progress, errors, canceled) = done(&rx);
    assert!(canceled);
    assert_eq!(progress.files_done, 1, "the acknowledged commit counts");
    assert!(errors.is_empty(), "{errors:?}");
    // Discovery lists both selected files in parallel, so either may be the
    // one that went first.
    let published: Vec<&str> = ["one.txt", "two.txt"]
        .into_iter()
        .filter(|name| target.join(name).exists())
        .collect();
    assert_eq!(published.len(), 1, "nothing is published after the cancel");
    assert_eq!(
        fs::read(target.join(published[0])).unwrap(),
        fs::read(source.join(published[0])).unwrap()
    );
    assert_eq!(
        fs::read_dir(&target).unwrap().count(),
        1,
        "no stage is left"
    );
    assert_eq!(fs::read(source.join("one.txt")).unwrap(), b"one");
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
        let result = download_remote_clipboard_items(
            &backend,
            &[(fwd(&source), "Vault".into(), true)],
            None,
        );
        assert_eq!(backend.get_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            backend.read_calls.load(Ordering::Relaxed),
            0,
            "no per-file retry after dispatched bulk"
        );
        if mode == 0 {
            let paths = result.unwrap();
            assert_eq!(paths.len(), 1);
            let local = Path::new(&paths[0]);
            assert_eq!(fs::read(local.join("note.txt")).unwrap(), b"clipboard");
            assert!(local.join("empty").is_dir());
            cleanup_temp_copy(local);
        } else {
            assert!(
                result.is_err(),
                "ambiguous bulk result must not be replayed as success"
            );
            assert!(
                !backend.bulk_destinations.lock().unwrap()[0].exists(),
                "failed owned clipboard tree must be cleaned"
            );
        }
        assert_eq!(fs::read(source.join("note.txt")).unwrap(), b"clipboard");
    }
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_transfer_same_backend_uses_protected_bridge() {
    let sandbox = Sandbox::new("same-backend");
    let source = sandbox.dir("source");
    fs::write(source.join("note.txt"), b"source bytes").unwrap();
    let note = source.join("note.txt");
    let length = b"source bytes".len() as u64;

    // Reads and writes may overlap on this connection: the bytes stream
    // directly, each counted once (umsetzung.md Block A.6; formerly always a
    // local temp bridge counting every byte twice).
    let backend = TaskBackend::new(&sandbox.0);
    let streamed = sandbox.dir("streamed");
    let result = same_backend_copy(&backend, &note, &streamed);
    assert_eq!(result.0.bytes_total, length);
    succeeded(result, 1);
    assert_eq!(
        fs::read(streamed.join("note.txt")).unwrap(),
        b"source bytes"
    );
    assert_eq!(backend.copy_calls.load(Ordering::Relaxed), 0);

    // One session that cannot read and write at once (FTP-like): the engine
    // bridges through a local temporary file and never holds a reader and a
    // writer open together.
    let mut backend = TaskBackend::new(&sandbox.0);
    backend.single_session = true;
    let bridged = sandbox.dir("bridged");
    succeeded(same_backend_copy(&backend, &note, &bridged), 1);
    assert!(
        !backend.sessions.overlapped.load(Ordering::SeqCst),
        "the bridge never overlaps reading and writing"
    );
    assert_eq!(fs::read(bridged.join("note.txt")).unwrap(), b"source bytes");
    assert_eq!(backend.copy_calls.load(Ordering::Relaxed), 0);

    // Inside one share the server copies into a private stage (Block A.6),
    // never through the replacing `copy_file`.
    let mut backend = TaskBackend::new(&sandbox.0);
    backend.server_copy = true;
    let served = sandbox.dir("served");
    succeeded(same_backend_copy(&backend, &note, &served), 1);
    assert_eq!(
        backend.read_calls.load(Ordering::Relaxed),
        0,
        "no client read"
    );
    assert_eq!(backend.copy_calls.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(served.join("note.txt")).unwrap(), b"source bytes");

    // A foreign file appearing at the destination is never replaced: the
    // copy takes "note (2).txt" (Block A.6, K14; formerly the file failed).
    let mut backend = TaskBackend::new(&sandbox.0);
    backend.race_on_promote = true;
    let target = sandbox.dir("target");
    succeeded(same_backend_copy(&backend, &note, &target), 1);
    assert_eq!(fs::read(target.join("note.txt")).unwrap(), FOREIGN);
    assert_eq!(
        fs::read(target.join("note (2).txt")).unwrap(),
        b"source bytes"
    );
    assert_eq!(backend.replacing_writes.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(&note).unwrap(), b"source bytes");
}
