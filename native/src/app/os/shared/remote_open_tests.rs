use super::*;

fn app_with_pending(name: &str) -> (App, PathBuf) {
    let mut app = App::new_for_copy_task();
    let temp = add_pending(&mut app, name);
    (app, temp)
}

fn add_pending(app: &mut App, name: &str) -> PathBuf {
    let temp = open_temp_path(name).unwrap();
    app.remote_edits.push(RemoteEdit {
        phase: RemoteEditPhase::Downloading,
        temp: temp.clone(),
        backend: Arc::new(crate::vfs::LocalBackend::new("/")),
        remote_path: format!("/unused/{name}"),
        name: name.into(),
        baseline_mtime: 0,
        seen_mtime: 0,
        remote_known_mtime: 0,
        dirty: false,
        uploading: false,
        process: None,
    });
    temp
}

fn poll(app: &mut App) {
    app.last_edit_poll = Instant::now() - std::time::Duration::from_secs(2);
    app.poll_remote_edits();
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(
        crate::transfer::session_temp_dir().join("preserved-recovery.txt")
    ).unwrap()).unwrap()
}

fn finish(mut app: App) {
    for edit in app.remote_edits.drain(..) {
        cleanup_temp_copy(&edit.temp);
    }
    sync_recovery_manifest(&app.remote_edits).unwrap();
}

#[test]
fn remote_drive_task_existing_temp_does_not_enter_missing_recovery_state() {
    assert!(!missing_temp_requires_recovery(123));
}

#[test]
fn remote_drive_task_missing_temp_is_retained_for_atomic_editor_save_recovery() {
    assert!(missing_temp_requires_recovery(0));
}

#[test]
#[ignore = "requires the isolated remote Direct opening task runner"]
fn direct_open_task_pending_download_is_neither_edit_nor_recovery() {
    let (mut app, temp) = app_with_pending("slow.jpg");
    poll(&mut app);
    assert!(app.error_msg.is_none());
    assert!(!app.remote_edits[0].dirty);
    assert!(!app.remote_edits[0].uploading);
    assert!(app.edit_save_rx.is_empty());
    sync_recovery_manifest(&app.remote_edits).unwrap();
    // Even publication ahead of the channel/UI drain is still download-owned.
    std::fs::write(&temp, b"downloaded").unwrap();
    poll(&mut app);
    assert_eq!(app.remote_edits[0].phase, RemoteEditPhase::Downloading);
    assert_eq!(app.remote_edits[0].baseline_mtime, 0);
    assert!(app.edit_save_rx.is_empty());
    assert!(app.prepare_downloaded_edit(&temp, 123).unwrap());
    poll(&mut app);
    assert_eq!(app.remote_edits[0].phase, RemoteEditPhase::Editing);
    assert_eq!(app.remote_edits[0].remote_known_mtime, 123);
    assert!(!app.remote_edits[0].dirty);
    assert!(app.edit_save_rx.is_empty());
    assert_eq!(manifest()["entries"].as_array().unwrap().len(), 1);
    finish(app);
}

#[test]
#[ignore = "requires the isolated remote Direct opening task runner"]
fn direct_open_task_parallel_download_and_atomic_save_do_not_block_new_open() {
    let (mut app, first) = app_with_pending("first.pdf");
    std::fs::write(&first, b"first").unwrap();
    app.prepare_downloaded_edit(&first, 1).unwrap();
    std::fs::remove_file(&first).unwrap();
    let pending = add_pending(&mut app, "still-downloading.jpg");
    let second = add_pending(&mut app, "second.jpg");
    std::fs::write(&second, b"second").unwrap();
    poll(&mut app);
    assert!(app.error_msg.is_none());
    assert!(app.remote_edits[0].dirty);
    assert!(app.prepare_downloaded_edit(&second, 2).unwrap());
    let entries = manifest()["entries"].as_array().unwrap().clone();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e["name"] == "first.pdf" && e["dirty"] == true));
    assert!(entries.iter().any(|e| e["name"] == "second.jpg"));
    assert!(!pending.exists());
    assert_eq!(app.remote_edits[1].phase, RemoteEditPhase::Downloading);
    finish(app);
}

#[test]
#[ignore = "requires the isolated remote Direct opening task runner"]
fn direct_open_task_new_missing_payload_and_failed_manifest_never_become_editing() {
    let (mut app, temp) = app_with_pending("absent.jpg");
    assert!(app.prepare_downloaded_edit(&temp, 1).is_err());
    assert_eq!(app.remote_edits[0].phase, RemoteEditPhase::Downloaded);
    std::fs::write(&temp, b"complete").unwrap();
    let marker = crate::transfer::session_temp_dir().join("preserved-recovery.txt");
    let _ = std::fs::remove_file(&marker);
    std::fs::create_dir(&marker).unwrap();
    assert!(app.prepare_downloaded_edit(&temp, 1).is_err());
    assert_eq!(app.remote_edits[0].phase, RemoteEditPhase::Downloaded);
    assert!(temp.is_file());
    std::fs::remove_dir(marker).unwrap();
    finish(app);
}

#[test]
#[ignore = "requires the isolated remote Direct opening task runner"]
fn direct_open_task_recovery_rejects_escaped_and_nonregular_editor_paths() {
    let (mut app, temp) = app_with_pending("unsafe.jpg");
    std::fs::create_dir(&temp).unwrap();
    assert!(app.prepare_downloaded_edit(&temp, 1).is_err());
    std::fs::remove_dir(&temp).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let escaped = outside.path().join("outside.jpg");
    std::fs::write(&escaped, b"foreign data").unwrap();
    app.remote_edits[0].temp = escaped.clone();
    assert!(app.prepare_downloaded_edit(&escaped, 1).is_err());
    app.remote_edits[0].phase = RemoteEditPhase::Editing;
    assert!(sync_recovery_manifest(&app.remote_edits).is_err());
    assert_eq!(std::fs::read(&escaped).unwrap(), b"foreign data");
    app.remote_edits[0].temp = temp;
    finish(app);
}

#[test]
#[ignore = "requires the isolated remote Direct opening task runner"]
fn direct_open_task_failed_and_disconnected_workers_remove_only_their_downloads() {
    let (mut app, good) = app_with_pending("already-open.pdf");
    std::fs::write(&good, b"user data").unwrap();
    app.prepare_downloaded_edit(&good, 1).unwrap();
    for disconnected in [false, true] {
        let temp = add_pending(&mut app, "failed.jpg");
        let (tx, rx) = unbounded();
        app.file_open_rx.push((rx, OpenMode::Default, temp.clone()));
        if !disconnected {
            tx.send(Err("connection lost: timed out".into())).unwrap();
        }
        drop(tx);
        app.drain_file_open();
        assert!(app.file_open_rx.is_empty());
        assert_eq!(app.remote_edits.len(), 1);
        assert!(!temp.parent().unwrap().exists());
        assert_eq!(std::fs::read(&good).unwrap(), b"user data");
        assert_eq!(manifest()["entries"].as_array().unwrap().len(), 1);
        assert!(!app.error_msg.as_ref().unwrap().contains("temporarily absent"));
    }
    finish(app);
}

#[test]
#[ignore = "requires the isolated remote Direct opening task runner"]
fn direct_open_task_atomic_save_waits_for_stability_then_saves_back() {
    let remote = tempfile::tempdir().unwrap();
    let remote_file = remote.path().join("original.txt");
    std::fs::write(&remote_file, b"original").unwrap();
    let (mut app, temp) = app_with_pending("edit.txt");
    app.remote_edits[0].remote_path = remote_file.to_string_lossy().into_owned();
    std::fs::write(&temp, b"original").unwrap();
    app.prepare_downloaded_edit(&temp, file_mtime_ms(&remote_file)).unwrap();
    std::fs::remove_file(&temp).unwrap();
    poll(&mut app);
    assert!(app.remote_edits[0].dirty);
    assert!(app.edit_save_rx.is_empty());
    std::fs::write(&temp, b"saved after replacement").unwrap();
    poll(&mut app);
    assert!(app.edit_save_rx.is_empty());
    poll(&mut app);
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while !app.edit_save_rx.is_empty() && Instant::now() < deadline {
        app.drain_edit_saves();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(app.edit_save_rx.is_empty());
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
    assert!(!app.remote_edits[0].dirty);
    assert_eq!(std::fs::read(remote_file).unwrap(), b"saved after replacement");
    finish(app);
}

#[test]
#[ignore = "requires the isolated remote Direct opening task runner"]
fn direct_open_task_manifest_failure_suppresses_upload_and_preserves_edit() {
    let (mut app, temp) = app_with_pending("must-stay-local.txt");
    std::fs::write(&temp, b"unsaved").unwrap();
    app.prepare_downloaded_edit(&temp, 1).unwrap();
    app.remote_edits[0].baseline_mtime = 0;
    let marker = crate::transfer::session_temp_dir().join("preserved-recovery.txt");
    std::fs::remove_file(&marker).unwrap();
    std::fs::create_dir(&marker).unwrap();
    poll(&mut app);
    assert!(app.error_msg.is_some());
    assert!(app.edit_save_rx.is_empty());
    assert!(!app.remote_edits[0].uploading);
    assert!(app.remote_edits[0].dirty);
    assert_eq!(std::fs::read(&temp).unwrap(), b"unsaved");
    std::fs::remove_dir(&marker).unwrap();
    finish(app);
}

#[test]
#[ignore = "requires the isolated remote Direct opening task runner"]
fn direct_open_task_save_conflict_and_failed_revision_check_preserve_remote() {
    let remote = tempfile::tempdir().unwrap();
    let remote_file = remote.path().join("remote.txt");
    std::fs::write(&remote_file, b"newer remote").unwrap();
    let (app, temp) = app_with_pending("edit.txt");
    std::fs::write(&temp, b"local edit").unwrap();
    let edit = &app.remote_edits[0];
    assert!(matches!(save_remote_edit(&*edit.backend, &temp,
        &remote_file.to_string_lossy(), 1), SaveResult::Conflict(_)));
    std::fs::remove_file(&remote_file).unwrap();
    assert!(matches!(save_remote_edit(&*edit.backend, &temp,
        &remote_file.to_string_lossy(), 1), SaveResult::Failed(_)));
    assert!(!remote_file.exists(), "failed stat must not recreate a missing remote");
    assert_eq!(std::fs::read(&temp).unwrap(), b"local edit");
    finish(app);
}
