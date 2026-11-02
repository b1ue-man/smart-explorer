use super::*;
use super::super::callback_reporter::CallbackReporter;
use super::super::runtime_selection::RuntimeSelection;
use crate::mount::MountRuntimePreference;
use crate::share::CopyPastePeerFixture;
use std::path::PathBuf;
use std::sync::mpsc;
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

struct MountedPeer {
    filesystem: Option<DokanyFileSystem>,
    storage: CallbackStorage,
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
    assert_eq!(std::env::var("SMART_EXPLORER_DOKANY_DLL_SHA256").as_deref(),
        Ok(super::super::private_payload::BUNDLED_DOKANY_SHA256));
    // Exercise the observed intermittent native failure in the same bounded
    // acceptance case, including repeated runtime creation and teardown.
    for preference in [MountRuntimePreference::Auto, MountRuntimePreference::System] {
        for attempt in 1..=4 {
            eprintln!("[remote mount] {preference:?} lifecycle {attempt}/4");
            exercise_volume(preference)?;
        }
    }
    Ok(())
}

fn exercise_volume(preference: MountRuntimePreference) -> io::Result<()> {
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
    let selection = RuntimeSelection::select(&spool, &id, preference)
        .map_err(|error| io::Error::other(format!("Dokany preflight: {error:?}")))?;
    let runtime = &selection.runtime;
    let private = preference == MountRuntimePreference::Auto;
    assert_eq!(runtime.is_private(), private, "runtime must not silently fall back");
    let loaded: Vec<u16> = runtime.loaded_path()?.as_os_str().encode_wide().chain(Some(0)).collect();
    let marker = spool.join(id.as_str()).join(format!("private-{}.attempt",
        super::super::private_payload::BUNDLED_DOKANY_SHA256));
    assert_eq!(marker.exists(), private, "incorrect private runtime recovery marker");
    eprintln!("[remote mount] runtime ready");
    let candidates = drive_candidates(DriveSelection::Automatic).map_err(io::Error::other)?;
    let initial = *candidates.first().ok_or_else(|| io::Error::other("no unused drive letter"))?;
    let (send, statuses) = mpsc::channel();
    let context = Box::new(CallbackContext::new(engine, runtime.clone(),
        CallbackReporter::Capture(send), initial, true, "Remote regression task".into(),
        super::super::metadata::volume_serial(id.as_str()), absolute_path_wide(&spool)?)?);
    let mut storage = CallbackStorage::new(context, true);
    eprintln!("[remote mount] create filesystem");
    let filesystem = start_on_available_drive(runtime, &mut storage, &candidates)
        .map_err(|error| io::Error::other(format!("Dokany start: {error:?}")))?;
    let volume = MountedPeer { filesystem: Some(filesystem), storage };
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
    std::thread::scope(|scope| -> io::Result<()> {
        let (ready, started) = mpsc::channel();
        let mut readers = Vec::new();
        for _ in 0..4 {
            let path = root.clone();
            let ready = ready.clone();
            readers.push(scope.spawn(move || -> io::Result<()> {
                let _ = std::fs::read_dir(&path)?.collect::<io::Result<Vec<_>>>()?;
                ready.send(()).map_err(io::Error::other)?;
                for _ in 0..64 {
                    let Ok(entries) = std::fs::read_dir(&path) else { break };
                    if entries.collect::<io::Result<Vec<_>>>().is_err() { break; }
                }
                Ok(())
            }));
        }
        for _ in 0..4 {
            started.recv_timeout(Duration::from_secs(10)).map_err(io::Error::other)?;
        }
        // Close while real kernel directory requests can still be in flight.
        drop(volume);
        for reader in readers {
            reader.join().map_err(|_| io::Error::other("concurrent drive reader panicked"))??;
        }
        Ok(())
    })?;
    assert!(!root.exists(), "mounted drive did not retire");
    eprintln!("[remote mount] drive retired");
    assert_eq!(marker.exists(), private, "recovery marker changed before runtime teardown");
    selection.complete();
    assert!(!marker.exists(), "controlled teardown did not clear the owned marker");
    let still_loaded = !unsafe { GetModuleHandleW(loaded.as_ptr()) }.is_null();
    assert_eq!(still_loaded, !private, "incorrect callback-code lifetime after teardown");
    eprintln!("[remote mount] runtime retired");
    drop(lease);
    temporary.close()?;
    drop(peer);
    eprintln!("[remote mount] peer retired");
    Ok(())
}
