use super::{
    validate_dokany_version_domains, BackendRoot, DokanyVersionCompatibilityError,
    DriveRuntimeInstallOutcome, DriveSelection, FlushOutcome, MountConfig, MountEngine, MountId,
    MountMode, MountRecovery, MountRuntimeConfig, MountSnapshot, MountSource, MountStatus,
    NamespaceOutcome, OpenDisposition, OpenFileOptions, RenameOutcome,
    DOKANY_DRIVER_PROTOCOL_VERSION, DOKANY_LIBRARY_API_VERSION,
};
use crate::vfs::{Backend, BackendHandle, LocalBackend, Scheme, VfsMeta};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Fixture {
    _temporary: tempfile::TempDir,
    remote: PathBuf,
    spool: PathBuf,
    config: MountConfig,
    backend: BackendHandle,
}

impl Fixture {
    fn new(id: &str) -> io::Result<Self> {
        let temporary = tempfile::tempdir()?;
        let remote = temporary.path().join("remote");
        let spool = temporary.path().join("spool");
        std::fs::create_dir_all(&remote)?;
        let remote_root = forward_slash(&remote);
        let config = MountConfig::new(
            MountId::parse(id)?,
            MountSource::SavedRemote {
                account: "remote-drive-task".into(),
                root: BackendRoot::parse(&remote_root)?,
            },
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Remote drive task",
        )?;
        let backend: BackendHandle = Arc::new(LocalBackend::new(&remote_root));
        Ok(Self {
            _temporary: temporary,
            remote,
            spool,
            config,
            backend,
        })
    }

    fn engine(&self) -> io::Result<MountEngine> {
        MountEngine::open(self.config.clone(), self.backend.clone(), &self.spool)
    }
}

fn forward_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn writable_existing() -> OpenFileOptions {
    OpenFileOptions {
        writable: true,
        disposition: OpenDisposition::OpenExisting,
    }
}

fn write_full(engine: &MountEngine, handle: super::HandleId, bytes: &[u8]) -> io::Result<()> {
    engine.truncate(handle, 0)?;
    assert_eq!(engine.write(handle, 0, bytes)?, bytes.len());
    assert_eq!(engine.flush(handle)?, FlushOutcome::Committed);
    engine.close(handle)
}

fn cached_root_names(engine: &MountEngine) -> io::Result<Vec<String>> {
    let mut names = engine
        .list_dir(r"\")?
        .into_iter()
        .map(|metadata| metadata.name)
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn assert_cached_missing(engine: &MountEngine, callback_path: &str) -> io::Result<()> {
    let error = engine.stat_cached(callback_path).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    Ok(())
}

#[test]
fn remote_drive_task_mutations_invalidate_warm_directory_and_point_metadata() -> io::Result<()> {
    let fixture = Fixture::new("metadata-mutation-invalidation")?;
    let engine = fixture.engine()?;

    // Every mutation below starts with a complete root snapshot already warm.
    assert!(cached_root_names(&engine)?.is_empty());

    let created = engine.open_file(
        r"\created.txt",
        OpenFileOptions {
            writable: true,
            disposition: OpenDisposition::CreateNew,
        },
    )?;
    write_full(&engine, created, b"fresh")?;
    assert_eq!(cached_root_names(&engine)?, ["created.txt"]);
    assert_eq!(engine.stat_cached(r"\created.txt")?.size, 5);

    assert_eq!(engine.mkdir(r"\Folder")?, NamespaceOutcome::Complete);
    assert_eq!(cached_root_names(&engine)?, ["Folder", "created.txt"]);
    assert!(engine.stat_cached(r"\Folder")?.is_dir);

    assert_eq!(
        engine.rename(r"\created.txt", r"\renamed.txt", false)?,
        RenameOutcome::Complete
    );
    assert_eq!(cached_root_names(&engine)?, ["Folder", "renamed.txt"]);
    assert_cached_missing(&engine, r"\created.txt")?;
    assert_eq!(engine.stat_cached(r"\renamed.txt")?.size, 5);

    engine.delete(r"\renamed.txt", false)?;
    assert_eq!(cached_root_names(&engine)?, ["Folder"]);
    assert_cached_missing(&engine, r"\renamed.txt")?;
    Ok(())
}

#[test]
fn remote_drive_task_lazy_writable_handle_commits_on_flush() -> io::Result<()> {
    let fixture = Fixture::new("lazy-writable-handle")?;
    std::fs::write(fixture.remote.join("doc.txt"), b"original")?;
    let engine = fixture.engine()?;

    // An open-existing handle starts lazy even when granted write access.
    let metadata = engine.stat(r"\doc.txt")?;
    let handle = engine.open_metadata_file(r"\doc.txt", metadata, true)?;
    assert_eq!(engine.len(handle)?, 8);

    // The first write fetches the current contents, then edits the spool.
    assert_eq!(engine.write(handle, 0, b"UPDATED!")?, 8);
    assert_eq!(engine.flush(handle)?, FlushOutcome::Committed);
    engine.close(handle)?;
    assert_eq!(std::fs::read(fixture.remote.join("doc.txt"))?, b"UPDATED!");

    // A truncating open of an existing file keeps the remote baseline for
    // conflict detection without transferring the discarded contents.
    let replaced = engine.open_file(
        r"\doc.txt",
        OpenFileOptions {
            writable: true,
            disposition: OpenDisposition::CreateAlways,
        },
    )?;
    assert_eq!(engine.len(replaced)?, 0);
    write_full(&engine, replaced, b"short")?;
    assert_eq!(std::fs::read(fixture.remote.join("doc.txt"))?, b"short");
    Ok(())
}

#[test]
fn remote_drive_task_obsidian_save_cycle_commits_long_short_and_empty_files() -> io::Result<()> {
    let fixture = Fixture::new("obsidian-save-cycle")?;
    let note = fixture.remote.join("note.md");
    std::fs::write(&note, b"seed")?;
    let engine = fixture.engine()?;

    let long = b"# Note\n\nThis is the longer Obsidian document body.\n";
    let short = b"# Short\n";
    for expected in [long.as_slice(), short.as_slice(), b"".as_slice()] {
        let handle = engine.open_file(r"\note.md", writable_existing())?;
        write_full(&engine, handle, expected)?;
        assert_eq!(std::fs::read(&note)?, expected);
    }

    assert!(engine.dirty_entries()?.is_empty());
    Ok(())
}

#[test]
fn remote_drive_task_atomic_replace_detaches_the_old_open_destination() -> io::Result<()> {
    let fixture = Fixture::new("obsidian-atomic-replace")?;
    let note = fixture.remote.join("note.md");
    let temporary_note = fixture.remote.join("note.md.tmp");
    std::fs::write(&note, b"old destination")?;
    let engine = fixture.engine()?;

    let old_destination = engine.open_file(r"\note.md", writable_existing())?;
    let replacement = engine.open_file(
        r"\note.md.tmp",
        OpenFileOptions {
            writable: true,
            disposition: OpenDisposition::CreateNew,
        },
    )?;
    assert_eq!(engine.write(replacement, 0, b"new complete note")?, 17);
    assert_eq!(engine.flush(replacement)?, FlushOutcome::Committed);
    engine.close(replacement)?;

    assert_eq!(
        engine.rename_with_shared_destination(r"\note.md.tmp", r"\note.md", true, true)?,
        RenameOutcome::Complete
    );
    assert_eq!(std::fs::read(&note)?, b"new complete note");
    assert!(!temporary_note.exists());

    engine.truncate(old_destination, 0)?;
    assert_eq!(engine.write(old_destination, 0, b"stale old handle")?, 16);
    assert_eq!(engine.flush(old_destination)?, FlushOutcome::NoChanges);
    engine.close(old_destination)?;
    assert_eq!(std::fs::read(&note)?, b"new complete note");
    assert!(engine.dirty_entries()?.is_empty());
    Ok(())
}

#[test]
fn remote_drive_task_exclusive_writer_collision_preserves_existing_bytes() -> io::Result<()> {
    let fixture = Fixture::new("exclusive-writer-collision")?;
    let occupied = fixture.remote.join("occupied.txt");
    std::fs::write(&occupied, b"foreign bytes")?;
    let backend = LocalBackend::new(&forward_slash(&fixture.remote));

    let result = backend.open_write_new(&forward_slash(&occupied));
    let error = match result {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "exclusive writer replaced an existing file",
            ))
        }
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&occupied)?, b"foreign bytes");
    Ok(())
}

#[test]
fn remote_drive_task_restart_retries_dirty_spool_and_pending_delete() -> io::Result<()> {
    let fixture = Fixture::new("restart-recovery")?;
    let note = fixture.remote.join("note.md");
    let doomed = fixture.remote.join("doomed.txt");
    std::fs::write(&note, b"old")?;
    std::fs::write(&doomed, b"delete me")?;

    let engine = fixture.engine()?;
    let dirty = engine.open_file(r"\note.md", writable_existing())?;
    engine.truncate(dirty, 0)?;
    assert_eq!(engine.write(dirty, 0, b"recovered edit")?, 14);
    assert_eq!(engine.dirty_entries()?.len(), 1);
    drop(engine);

    let engine = fixture.engine()?;
    assert_eq!(engine.dirty_entries()?.len(), 1);
    engine.retry_pending_changes()?;
    assert_eq!(std::fs::read(&note)?, b"recovered edit");
    assert!(engine.dirty_entries()?.is_empty());

    let _token = engine.begin_delete(r"\doomed.txt", false)?;
    assert!(!doomed.exists());
    assert_eq!(engine.pending_deletes()?.len(), 1);
    drop(engine);

    let engine = fixture.engine()?;
    assert_eq!(engine.pending_deletes()?.len(), 1);
    engine.retry_pending_changes()?;
    assert!(engine.pending_deletes()?.is_empty());
    assert!(!doomed.exists());
    assert!(std::fs::read_dir(&fixture.remote)?.all(|entry| {
        entry
            .ok()
            .and_then(|entry| entry.file_name().into_string().ok())
            .map_or(true, |name| !name.contains(".se-mount-delete-"))
    }));
    Ok(())
}

#[test]
fn remote_drive_task_recovery_wire_state_is_conservative_and_compatible() -> io::Result<()> {
    let fixture = Fixture::new("recovery-wire-state")?;
    let snapshot = MountSnapshot {
        config: fixture.config,
        status: MountStatus::Unmounted,
        recovery: MountRecovery::Required,
        recovery_required_compat: true,
    };
    let current = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(current["recovery"], "Required");
    assert_eq!(current["recovery_required"], true);

    let mut legacy_required = current.clone();
    legacy_required.as_object_mut().unwrap().remove("recovery");
    assert_eq!(
        serde_json::from_value::<MountSnapshot>(legacy_required)
            .unwrap()
            .recovery,
        MountRecovery::Required
    );

    let mut legacy_clean = current.clone();
    legacy_clean.as_object_mut().unwrap().remove("recovery");
    legacy_clean["recovery_required"] = false.into();
    assert_eq!(
        serde_json::from_value::<MountSnapshot>(legacy_clean)
            .unwrap()
            .recovery,
        MountRecovery::Clean
    );

    let mut absent = current.clone();
    absent.as_object_mut().unwrap().remove("recovery");
    absent.as_object_mut().unwrap().remove("recovery_required");
    assert_eq!(
        serde_json::from_value::<MountSnapshot>(absent)
            .unwrap()
            .recovery,
        MountRecovery::Unknown
    );

    let mut new_wins = current;
    new_wins["recovery"] = "Clean".into();
    new_wins["recovery_required"] = true.into();
    assert_eq!(
        serde_json::from_value::<MountSnapshot>(new_wins)
            .unwrap()
            .recovery,
        MountRecovery::Clean
    );
    assert!(!MountRecovery::Clean.requires_retention());
    assert!(MountRecovery::Required.requires_retention());
    assert!(MountRecovery::Unknown.requires_retention());
    Ok(())
}

struct RemoteProbe {
    stats: Arc<AtomicUsize>,
}

impl Backend for RemoteProbe {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, _path: &str) -> io::Result<Vec<VfsMeta>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn stat(&self, _path: &str) -> io::Result<VfsMeta> {
        self.stats.fetch_add(1, Ordering::SeqCst);
        Ok(VfsMeta {
            name: "/".into(),
            is_dir: true,
            ..VfsMeta::default()
        })
    }

    fn open_read(&self, _path: &str) -> io::Result<Box<dyn std::io::Read + Send>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn open_write(&self, _path: &str) -> io::Result<Box<dyn std::io::Write + Send>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn rename(&self, _src: &str, _dst: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn remove_file(&self, _path: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn remove_dir(&self, _path: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn mkdir_all(&self, _path: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }
}

#[test]
fn remote_drive_task_host_audits_local_cache_before_remote_root() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let stats = Arc::new(AtomicUsize::new(0));
    let backend: BackendHandle = Arc::new(RemoteProbe {
        stats: stats.clone(),
    });
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse("local-first-recovery")?, MountMode::ReadOnly),
        backend,
        temporary.path(),
    )?;
    assert_eq!(stats.load(Ordering::SeqCst), 0);
    engine.prepare_host_remote()?;
    assert_eq!(stats.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn remote_drive_task_current_binary_mount_host_argument_is_exact() {
    assert!(super::run_host_if_requested(&["--other".into()]).is_none());
    assert!(super::run_host_if_requested(&["--mount-host".into()]).is_none());
    let result =
        super::run_host_if_requested(&["--mount-host".into(), "current-binary-host".into()]);
    assert!(result.is_some());
    assert!(result.unwrap().is_err());
}

#[test]
fn remote_drive_task_dokany_msi_outcomes_preserve_actionable_exit_codes() {
    let cases = [
        (0, false),
        (3010, false),
        (1641, false),
        (1223, false),
        (1602, false),
        (1618, true),
        (1603, true),
        (1633, true),
        (1654, true),
        (9999, true),
    ];
    for (code, failure) in cases {
        let outcome = DriveRuntimeInstallOutcome::from_msi_exit_code(code);
        assert_eq!(
            outcome.exit_code(),
            if code == 9999 { 1 } else { code as i32 }
        );
        assert_eq!(outcome.is_failure(), failure);
    }
}

#[test]
fn remote_drive_task_dokany_library_and_driver_versions_use_separate_domains() {
    assert_eq!(DOKANY_LIBRARY_API_VERSION, 231);
    assert_eq!(DOKANY_DRIVER_PROTOCOL_VERSION, 400);
    assert_eq!(validate_dokany_version_domains(231, 400), Ok(()));
    assert_eq!(
        validate_dokany_version_domains(400, 400),
        Err(DokanyVersionCompatibilityError::LibraryApiMismatch {
            expected: 231,
            found: 400,
        })
    );
    assert_eq!(
        validate_dokany_version_domains(231, 0),
        Err(DokanyVersionCompatibilityError::DriverUnavailable)
    );
    assert_eq!(
        validate_dokany_version_domains(231, 231),
        Err(DokanyVersionCompatibilityError::DriverProtocolMismatch {
            expected: 400,
            found: 231,
        })
    );
}
