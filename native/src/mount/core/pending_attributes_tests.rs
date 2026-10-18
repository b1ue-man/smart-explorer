//! Signals of the `setattr` rules (DRV-M4, decision C2): a Windows drive
//! never stores times or modes and never calls `finish_stage` (Bestandsschutz
//! of every Dokany drive), and a POSIX drive hands them to exactly the next
//! publication of the dirty file.
use super::super::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use super::super::{
    FlushOutcome, MountEngine, MountFilesystem, MountId, MountMode, MountRuntimeConfig,
    OpenDisposition, OpenFileOptions, PathDialect, PendingAttributes,
};
use crate::vfs::{
    Backend, BackendExtensions, RootConfinement, Scheme, StageDurability, StageFinish,
    StageFinished, StagedWriteCapabilities, VfsMeta, VfsResult,
};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

const TIME: i64 = 1_600_000_000_000;

#[test]
fn drive_task_windows_drive_ignores_pending_attributes() {
    let directory = FixtureDirectory::new();
    let backend = RecordingBackend::new();
    backend.inner.put("/note.txt", b"old");
    let engine = engine(
        &directory,
        &backend,
        PathDialect::Windows,
        "windows-attributes",
    );
    let handle = engine
        .open_file(
            "\\note.txt",
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::TruncateExisting,
            },
        )
        .unwrap();
    engine.write(handle, 0, b"new").unwrap();
    // Dokany answers time and attribute changes with ERROR_NOT_SUPPORTED;
    // the engine of a Windows drive refuses them on every route.
    let attributes = PendingAttributes {
        mtime_ms: Some(TIME),
        mode: Some(0o600),
    };
    let refused = engine
        .set_pending_attributes(handle, attributes)
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::Unsupported);
    let filesystem: &dyn MountFilesystem = &engine;
    let refused = filesystem
        .set_pending_attributes(handle, attributes)
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::Unsupported);
    assert_eq!(engine.flush(handle).unwrap(), FlushOutcome::Committed);
    engine.close(handle).unwrap();
    assert_eq!(backend.inner.bytes("/note.txt"), b"new");
    assert!(
        backend.finished().is_empty(),
        "a Windows drive called finish_stage: {:?}",
        backend.finished()
    );
}

#[test]
fn pending_attributes_reach_exactly_the_next_publication() {
    let directory = FixtureDirectory::new();
    let backend = RecordingBackend::new();
    backend.inner.put("/kept.txt", b"kept");
    let engine = engine(&directory, &backend, PathDialect::Posix, "posix-attributes");
    let handle = engine
        .open_file(
            "/new.txt",
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::CreateNew,
            },
        )
        .unwrap();
    engine.write(handle, 0, b"content").unwrap();
    // `cp -a`: times first, then the mode; both ride on the publication.
    engine
        .set_pending_attributes(
            handle,
            PendingAttributes {
                mtime_ms: Some(TIME),
                mode: None,
            },
        )
        .unwrap();
    engine
        .set_pending_attributes(
            handle,
            PendingAttributes {
                mtime_ms: None,
                mode: Some(0o100_640),
            },
        )
        .unwrap();
    assert_eq!(engine.flush(handle).unwrap(), FlushOutcome::Committed);
    let finished = backend.finished();
    assert_eq!(finished.len(), 1, "{finished:?}");
    assert!(finished[0].0.contains(".se-mount-"), "{finished:?}");
    assert_eq!(
        finished[0].1,
        StageFinish {
            mtime_ms: Some(TIME),
            mode: Some(0o640),
            durability: StageDurability::NotRequired,
        }
    );
    // A later save of the same file carries no stale values.
    engine.write(handle, 7, b"!").unwrap();
    assert_eq!(engine.flush(handle).unwrap(), FlushOutcome::Committed);
    assert_eq!(backend.finished().len(), 1);
    engine.close(handle).unwrap();
    assert_eq!(backend.inner.bytes("/new.txt"), b"content!");

    // A clean file keeps nothing: success without effect.
    let clean = engine
        .open_file(
            "/kept.txt",
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::OpenExisting,
            },
        )
        .unwrap();
    engine
        .set_pending_attributes(
            clean,
            PendingAttributes {
                mtime_ms: Some(TIME),
                mode: Some(0o600),
            },
        )
        .unwrap();
    assert_eq!(engine.flush(clean).unwrap(), FlushOutcome::NoChanges);
    engine.close(clean).unwrap();
    assert_eq!(backend.finished().len(), 1);
}

fn engine(
    directory: &FixtureDirectory,
    backend: &Arc<RecordingBackend>,
    dialect: PathDialect,
    id: &str,
) -> MountEngine {
    let handle: crate::vfs::BackendHandle = backend.clone();
    MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse(id).unwrap(), MountMode::ReadWrite)
            .with_path_dialect(dialect),
        handle,
        directory.path(),
    )
    .unwrap()
}

/// The deterministic fixture remote plus a record of every `finish_stage`.
struct RecordingBackend {
    inner: Arc<OptimizationBackend>,
    finished: Mutex<Vec<(String, StageFinish)>>,
}

impl RecordingBackend {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: OptimizationBackend::new(),
            finished: Mutex::new(Vec::new()),
        })
    }

    fn finished(&self) -> Vec<(String, StageFinish)> {
        self.finished.lock().unwrap().clone()
    }
}

impl BackendExtensions for RecordingBackend {
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        self.finished
            .lock()
            .unwrap()
            .push((stage.to_string(), finish));
        Ok(StageFinished {
            mtime_applied: true,
            durable: false,
        })
    }
}

impl Backend for RecordingBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }
    fn case_sensitive_paths(&self, root: &str) -> bool {
        self.inner.case_sensitive_paths(root)
    }
    fn root_confinement(&self, root: &str) -> RootConfinement {
        self.inner.root_confinement(root)
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn staged_write_capabilities(&self, root: &str) -> StagedWriteCapabilities {
        self.inner.staged_write_capabilities(root)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read_id(path, id)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }
    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(source, destination)
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged_no_replace(staged, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        self.inner.remove_file_id(path, id)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
}
