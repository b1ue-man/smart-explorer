//! Local filesystem adapter with deterministic faults at transfer boundaries.
use crate::app::app_models::{TransferMsg, TransferProgress};
use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub(super) const FOREIGN: &[u8] = b"foreign data must survive";

pub(super) fn fwd(path: &Path) -> String { path.to_string_lossy().replace('\\', "/") }

pub(super) struct Sandbox(pub(super) PathBuf);

impl Sandbox {
    pub(super) fn new(tag: &str) -> Self {
        assert_eq!(std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(), Ok("1"),
            "run through the configured copy/paste task entrypoint");
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("se-copy-task-{tag}-{}-{stamp}-{}",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).expect("reserve a new owned fixture directory");
        Self(path)
    }

    pub(super) fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

pub(super) fn done(rx: &crossbeam_channel::Receiver<TransferMsg>) -> (TransferProgress, Vec<String>, bool) {
    // Workers are synchronous here. A missing or duplicated terminal message is
    // an assertion, never an unbounded receiver wait on broken behavior.
    let mut terminal = None;
    for message in rx.try_iter() {
        if let TransferMsg::Done { progress, errors, canceled } = message {
            assert!(terminal.is_none(), "duplicate terminal transfer message");
            terminal = Some((progress, errors, canceled));
        }
    }
    terminal.expect("transfer returned without its terminal result")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StageFault { None, Unsupported, ForeignOnOpen, ForeignOnFlush }

pub(super) struct TaskBackend {
    inner: LocalBackend,
    pub(super) stage_fault: StageFault,
    pub(super) race_on_promote: bool,
    pub(super) race_on_read: Option<PathBuf>,
    pub(super) read_bytes: Option<Vec<u8>>,
    pub(super) unknown_read_size: bool,
    pub(super) cancel_after_promote: Option<Arc<AtomicBool>>,
    pub(super) mutate_source: Option<PathBuf>,
    pub(super) bulk_mismatch: bool,
    pub(super) bulk_error: bool,
    pub(super) mutations: AtomicUsize,
    pub(super) read_calls: AtomicUsize,
    pub(super) replacing_writes: AtomicUsize,
    pub(super) get_calls: AtomicUsize,
    pub(super) put_calls: AtomicUsize,
    pub(super) copy_calls: AtomicUsize,
    pub(super) remove_calls: AtomicUsize,
    pub(super) stages: Mutex<Vec<PathBuf>>,
    pub(super) bulk_destinations: Mutex<Vec<PathBuf>>,
}

impl TaskBackend {
    pub(super) fn new(root: &Path) -> Self {
        Self {
            inner: LocalBackend::new(&fwd(root)), stage_fault: StageFault::None,
            race_on_promote: false, race_on_read: None, read_bytes: None,
            unknown_read_size: false, cancel_after_promote: None, mutate_source: None,
            bulk_mismatch: false, bulk_error: false, mutations: AtomicUsize::new(0),
            read_calls: AtomicUsize::new(0), replacing_writes: AtomicUsize::new(0),
            get_calls: AtomicUsize::new(0), put_calls: AtomicUsize::new(0),
            copy_calls: AtomicUsize::new(0), remove_calls: AtomicUsize::new(0),
            stages: Mutex::new(Vec::new()), bulk_destinations: Mutex::new(Vec::new()),
        }
    }
}

fn create_foreign(path: &Path) -> io::Result<()> {
    OpenOptions::new().write(true).create_new(true).open(path)?.write_all(FOREIGN)
}

struct StageWriter {
    inner: Option<Box<dyn Write + Send>>,
    path: PathBuf,
    foreign_on_flush: bool,
    mutate_source: Option<PathBuf>,
}

impl Write for StageWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.as_mut().expect("live fixture writer").write(bytes)?;
        if let Some(source) = self.mutate_source.take() {
            OpenOptions::new().append(true).open(source)?.write_all(b"grew")?;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.as_mut().expect("live fixture writer").flush()?;
        if self.foreign_on_flush {
            drop(self.inner.take());
            fs::remove_file(&self.path)?;
            create_foreign(&self.path)?;
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "ambiguous fixture flush"));
        }
        Ok(())
    }
}

impl Backend for TaskBackend {
    fn scheme(&self) -> Scheme { self.inner.scheme() }
    fn root_display(&self) -> String { self.inner.root_display() }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> { self.inner.list_dir(path) }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> { self.inner.stat(path) }
    fn read_size(&self, _path: &str, size: u64) -> VfsResult<Option<u64>> {
        Ok(if self.unknown_read_size { None } else { Some(size) })
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.read_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(destination) = &self.race_on_read { create_foreign(destination)?; }
        if let Some(bytes) = &self.read_bytes { return Ok(Box::new(io::Cursor::new(bytes.clone()))); }
        self.inner.open_read(path)
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.mutations.fetch_add(1, Ordering::Relaxed);
        self.replacing_writes.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::other("copy must not fall back to a replacing writer"))
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.mutations.fetch_add(1, Ordering::Relaxed);
        self.stages.lock().unwrap().push(PathBuf::from(path));
        match self.stage_fault {
            StageFault::Unsupported => return Err(io::Error::new(io::ErrorKind::Unsupported, "fixture exclusive writer unavailable")),
            StageFault::ForeignOnOpen => {
                create_foreign(Path::new(path))?;
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "foreign stage appeared"));
            }
            StageFault::None | StageFault::ForeignOnFlush => {}
        }
        Ok(Box::new(StageWriter {
            inner: Some(self.inner.open_write_new(path)?), path: PathBuf::from(path),
            foreign_on_flush: self.stage_fault == StageFault::ForeignOnFlush,
            mutate_source: self.mutate_source.clone(),
        }))
    }
    fn promote_copy_stage(&self, stage: &str, destination: &str) -> VfsResult<()> {
        self.mutations.fetch_add(1, Ordering::Relaxed);
        if self.race_on_promote { create_foreign(Path::new(destination))?; }
        self.inner.promote_staged_no_replace(stage, destination)?;
        if let Some(cancel) = &self.cancel_after_promote { cancel.store(true, Ordering::Release); }
        Ok(())
    }
    fn promote_staged(&self, stage: &str, destination: &str) -> VfsResult<()> {
        self.mutations.fetch_add(1, Ordering::Relaxed);
        self.inner.promote_staged(stage, destination)
    }
    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> { self.inner.rename(source, destination) }
    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(source, destination)
    }
    fn rename_overwrites(&self) -> bool { self.inner.rename_overwrites() }
    fn copy_file(&self, _source: &str, _destination: &str) -> VfsResult<u64> {
        self.copy_calls.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::other("GUI copy must not use replacing server-copy"))
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.remove_calls.fetch_add(1, Ordering::Relaxed);
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> { self.inner.remove_dir(path) }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.mutations.fetch_add(1, Ordering::Relaxed);
        self.inner.mkdir_all(path)
    }
    fn supports_bulk_tree(&self) -> bool { true }
    fn get_tree(&self, source: &str, destination: &Path) -> VfsResult<u64> {
        self.get_calls.fetch_add(1, Ordering::Relaxed);
        self.bulk_destinations.lock().unwrap().push(destination.to_path_buf());
        let count = copy_tree(Path::new(source), destination)?;
        if self.bulk_error { return Err(io::Error::other("ambiguous bulk fixture failure")); }
        Ok(if self.bulk_mismatch { count + 1 } else { count })
    }
    fn put_tree(&self, _source: &Path, _destination: &str) -> VfsResult<u64> {
        self.put_calls.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::other("copy must not dispatch final-tree bulk upload"))
    }
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<u64> {
    fs::create_dir_all(destination)?;
    let mut count = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        assert!(!kind.is_symlink(), "fixture tree contains no links");
        let target = destination.join(entry.file_name());
        if kind.is_dir() { count += copy_tree(&entry.path(), &target)?; }
        else { fs::copy(entry.path(), target)?; count += 1; }
    }
    Ok(count)
}
