//! G4 ML4 invariants of the streaming hydration: a read returns as soon as
//! its range arrived; a remote change during the download fails every later
//! read of that handle instead of mixing two versions, and the next open
//! reads the new version.
use super::super::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use super::super::{
    HandleId, MountEngine, MountId, MountMode, MountRuntimeConfig, OpenDisposition,
    OpenFileOptions, PathDialect,
};
use crate::vfs::{
    Backend, BackendHandle, RootConfinement, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult,
};
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Half of a file that streams (`STREAM_ABOVE` is 1 MiB).
const HALF: usize = 1024 * 1024;

/// The fixture remote whose next download stops after `HALF` bytes until
/// the test releases it.
struct HeldBackend {
    inner: Arc<OptimizationBackend>,
    hold: Mutex<Option<(Sender<()>, Receiver<()>)>>,
}

struct HeldReader {
    inner: Box<dyn Read + Send>,
    served: usize,
    hold: Option<(Sender<()>, Receiver<()>)>,
}

impl Read for HeldReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.served >= HALF {
            if let Some((reached, release)) = self.hold.take() {
                reached
                    .send(())
                    .map_err(|_| io::Error::other("test ended"))?;
                release
                    .recv_timeout(Duration::from_secs(20))
                    .map_err(|_| io::Error::other("test never released the download"))?;
            }
        }
        let limit = if self.served < HALF {
            output.len().min(HALF - self.served)
        } else {
            output.len()
        };
        let read = self.inner.read(&mut output[..limit])?;
        self.served += read;
        Ok(read)
    }
}

impl Backend for HeldBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
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
        self.open_read_id(path, None)
    }
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(HeldReader {
            inner: self.inner.open_read_id(path, id)?,
            served: 0,
            hold: self.hold.lock().unwrap().take(),
        }))
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
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged_no_replace(staged, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
}

/// Holds the next download; returns (reached, release).
fn hold_next(backend: &HeldBackend) -> (Receiver<()>, Sender<()>) {
    let (reached_tx, reached) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    *backend.hold.lock().unwrap() = Some((reached_tx, release_rx));
    (reached, release)
}

fn engine(directory: &FixtureDirectory, backend: &Arc<HeldBackend>) -> MountEngine {
    let handle: BackendHandle = backend.clone();
    MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse("hydration").unwrap(), MountMode::ReadWrite)
            .with_path_dialect(PathDialect::Posix),
        handle,
        directory.path(),
    )
    .unwrap()
}

fn open(engine: &MountEngine, path: &str) -> HandleId {
    engine
        .open_file(
            path,
            OpenFileOptions {
                writable: false,
                disposition: OpenDisposition::OpenExisting,
            },
        )
        .unwrap()
}

fn read_all(engine: &MountEngine, handle: HandleId) -> io::Result<Vec<u8>> {
    let mut content = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = engine.read(handle, content.len() as u64, &mut buffer)?;
        if read == 0 {
            return Ok(content);
        }
        content.extend_from_slice(&buffer[..read]);
    }
}

#[test]
fn speed_task_drive_first_read_does_not_wait_for_download() -> io::Result<()> {
    let directory = FixtureDirectory::new();
    let backend = Arc::new(HeldBackend {
        inner: OptimizationBackend::new(),
        hold: Mutex::new(None),
    });
    let old = vec![b'a'; 2 * HALF];
    backend.inner.put("/video.bin", &old);
    let (reached, release) = hold_next(&backend);
    let engine = engine(&directory, &backend);
    let handle = open(&engine, "/video.bin");
    let mut first = vec![0u8; 4096];
    // The download is held halfway; the first range is served anyway.
    assert_eq!(engine.read(handle, 0, &mut first)?, first.len());
    assert!(first.iter().all(|byte| *byte == b'a'));
    reached
        .recv_timeout(Duration::from_secs(20))
        .expect("the download reached its hold");
    assert_eq!(
        engine.len(handle)?,
        old.len() as u64,
        "the length is the final one while the download runs"
    );
    release.send(()).unwrap();
    assert_eq!(read_all(&engine, handle)?, old);
    engine.close(handle)?;
    Ok(())
}

#[test]
fn speed_task_drive_change_during_download_fails_reads() -> io::Result<()> {
    let directory = FixtureDirectory::new();
    let backend = Arc::new(HeldBackend {
        inner: OptimizationBackend::new(),
        hold: Mutex::new(None),
    });
    let old = vec![b'a'; 2 * HALF];
    backend.inner.put("/doc.bin", &old);
    let (reached, release) = hold_next(&backend);
    let engine = engine(&directory, &backend);
    let handle = open(&engine, "/doc.bin");
    let mut first = vec![0u8; 4096];
    assert_eq!(engine.read(handle, 0, &mut first)?, first.len());
    reached
        .recv_timeout(Duration::from_secs(20))
        .expect("the download reached its hold");
    // Another writer replaces the remote file during the download.
    let new = vec![b'b'; HALF / 2];
    backend.inner.put("/doc.bin", &new);
    release.send(()).unwrap();
    let mut rest = vec![0u8; 4096];
    let error = engine
        .read(handle, (2 * HALF - 4096) as u64, &mut rest)
        .expect_err("a read after the change must fail, never mix versions");
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert!(
        engine.read_still_fails(handle),
        "the handle keeps failing until it is reopened"
    );
    engine.close(handle)?;
    let reopened = open(&engine, "/doc.bin");
    assert_eq!(
        read_all(&engine, reopened)?,
        new,
        "the next open reads the new version"
    );
    engine.close(reopened)?;
    Ok(())
}

impl MountEngine {
    /// Test probe: another read of `handle` still reports the change.
    fn read_still_fails(&self, handle: HandleId) -> bool {
        let mut buffer = [0u8; 16];
        self.read(handle, 0, &mut buffer)
            .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
    }
}
