use super::*;
use super::super::callback_reporter::CallbackReporter;
use crate::share::CopyPastePeerFixture;
use std::path::PathBuf;
use std::sync::mpsc;

struct MountedPeer {
    filesystem: Option<DokanyFileSystem>,
    storage: CallbackStorage,
    _lease: CacheLease,
    _temporary: tempfile::TempDir,
}

impl Drop for MountedPeer {
    fn drop(&mut self) {
        eprintln!("[remote mount] close begin");
        self.storage.request_metadata_refresh_stop();
        if let Some(filesystem) = self.filesystem.take() { filesystem.close(); }
        self.storage.join_metadata_refresh();
        eprintln!("[remote mount] close complete");
    }
}

#[test]
#[ignore = "requires the remote Windows task runner and pinned Dokany runtime"]
fn windows_remote_task_actual_drive_mounts_case_colliding_share_roots() -> io::Result<()> {
    assert_eq!(std::env::var("SMART_EXPLORER_WINDOWS_REMOTE_TASK").as_deref(), Ok("1"));
    // Exercise the observed intermittent native failure in the same bounded
    // acceptance case, including repeated runtime creation and teardown.
    for attempt in 1..=4 {
        eprintln!("[remote mount] lifecycle {attempt}/4");
        exercise_volume()?;
    }
    Ok(())
}

fn exercise_volume() -> io::Result<()> {
    eprintln!("[remote mount] peer setup");
    let peer = CopyPastePeerFixture::with_labels(["Docs", "docs"])?;
    eprintln!("[remote mount] peer ready");
    std::fs::write(peer.root_a.join("note.txt"), b"upper")?;
    std::fs::write(peer.root_b.join("note.txt"), b"lower")?;
    let backend = crate::daemon::windows_remote_task_rooted(peer.backend.clone(), MountMode::ReadOnly)?;
    let temporary = tempfile::tempdir()?;
    let spool = prepare_spool_root(&temporary.path().join("spool"))?;
    let id = MountId::new_random()?;
    let lease = CacheLease::acquire(&spool, &id)?;
    let engine = Arc::new(MountEngine::open_host_cache(
        MountRuntimeConfig::new(id.clone(), MountMode::ReadOnly), backend, &spool,
    )?);
    engine.prepare_host_remote()?;
    engine.preload_metadata()?;
    eprintln!("[remote mount] metadata loaded");
    let runtime = DokanyRuntime::preflight_private(&spool)
        .map_err(|error| io::Error::other(format!("Dokany preflight: {error:?}")))?;
    assert!(runtime.is_private());
    eprintln!("[remote mount] private runtime ready");
    let candidates = drive_candidates(DriveSelection::Automatic).map_err(io::Error::other)?;
    let initial = *candidates.first().ok_or_else(|| io::Error::other("no unused drive letter"))?;
    let (send, statuses) = mpsc::channel();
    let context = Box::new(CallbackContext::new(engine, runtime.clone(),
        CallbackReporter::Capture(send), initial, true, "Remote regression task".into(),
        super::super::metadata::volume_serial(id.as_str()), absolute_path_wide(&spool)?)?);
    let mut storage = CallbackStorage::new(context, true);
    eprintln!("[remote mount] create filesystem");
    let filesystem = start_on_available_drive(&runtime, &mut storage, &candidates)
        .map_err(|error| io::Error::other(format!("Dokany start: {error:?}")))?;
    let volume = MountedPeer { filesystem: Some(filesystem), storage, _lease: lease, _temporary: temporary };
    eprintln!("[remote mount] filesystem created");
    let drive = volume.storage.context.selected_drive()?;
    assert!(matches!(statuses.recv_timeout(Duration::from_secs(10)), Ok(MountStatus::Mounted { drive: mounted }) if mounted == drive));
    let root = PathBuf::from(format!("{}:\\", drive.get()));
    eprintln!("[remote mount] list {}", root.display());
    let mut contents = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        eprintln!("[remote mount] read {}", entry.path().display());
        assert!(crate::mount::peer_names::is_peer_alias(&entry.file_name().to_string_lossy()));
        contents.push(std::fs::read(entry.path().join("note.txt"))?);
        eprintln!("[remote mount] deny write {}", entry.path().display());
        assert!(std::fs::write(entry.path().join("forbidden.txt"), b"denied").is_err());
    }
    contents.sort();
    assert_eq!(contents, [b"lower".to_vec(), b"upper".to_vec()]);
    assert!(!volume.storage.context.stop_requested());
    drop(volume);
    assert!(!root.exists(), "mounted drive did not retire");
    eprintln!("[remote mount] drive retired");
    Ok(())
}
