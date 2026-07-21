use super::{
    BackendRoot, DriveSelection, FlushOutcome, MountConfig, MountEngine, MountId, MountMode,
    MountSource, OpenDisposition, OpenFileOptions, RenameOutcome,
};
use crate::vfs::{Backend, BackendHandle, LocalBackend};
use std::io;
use std::path::{Path, PathBuf};
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
