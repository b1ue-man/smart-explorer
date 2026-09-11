use crate::vfs::{Backend, CachingBackend, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

fn enabled() {
    assert_eq!(std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(), Ok("1"));
}

struct HeldReader {
    inner: Box<dyn Read + Send>,
    active: Arc<AtomicBool>,
}

impl Read for HeldReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> { self.inner.read(buffer) }
}

impl Drop for HeldReader {
    fn drop(&mut self) { self.active.store(false, Ordering::SeqCst); }
}

struct FixtureBackend {
    local: LocalBackend,
    active: Arc<AtomicBool>,
    private_only: bool,
    opens: AtomicUsize,
    promotions: AtomicUsize,
}

impl FixtureBackend {
    fn new(path: &str, active: Arc<AtomicBool>, private_only: bool) -> Self {
        Self { local: LocalBackend::new(path), active, private_only,
            opens: AtomicUsize::new(0), promotions: AtomicUsize::new(0) }
    }

    fn idle(&self) -> io::Result<()> {
        if self.active.load(Ordering::SeqCst) {
            Err(io::Error::new(io::ErrorKind::WouldBlock, "operation while serial reader owns connection"))
        } else { Ok(()) }
    }
}

impl Backend for FixtureBackend {
    fn scheme(&self) -> Scheme { Scheme::Local }
    fn root_display(&self) -> String { self.local.root_display() }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.idle()?;
        self.local.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.idle()?;
        self.local.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.idle()?;
        let inner = self.local.open_read(path)?;
        self.active.store(true, Ordering::SeqCst);
        Ok(Box::new(HeldReader { inner, active: self.active.clone() }))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "ordinary writer forbidden"))
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.idle()?;
        if self.private_only {
            Err(io::Error::new(io::ErrorKind::Unsupported, "requires explicit private copy API"))
        } else { self.local.open_write_new(path) }
    }
    fn open_write_copy_stage(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.idle()?;
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.local.open_write_new(path)
    }
    fn promote_copy_stage(&self, from: &str, to: &str) -> VfsResult<()> {
        self.idle()?;
        self.promotions.fetch_add(1, Ordering::SeqCst);
        self.local.promote_staged_no_replace(from, to)
    }
    fn read_size(&self, _path: &str, size: u64) -> VfsResult<Option<u64>> {
        Ok((!self.private_only).then_some(size))
    }
    fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        self.idle()?;
        self.local.rename(from, to)
    }
    fn rename_no_replace(&self, from: &str, to: &str) -> VfsResult<()> {
        self.idle()?;
        self.local.rename_no_replace(from, to)
    }
    fn rename_overwrites(&self) -> bool { self.local.rename_overwrites() }
    fn remove_file(&self, path: &str) -> VfsResult<()> { self.local.remove_file(path) }
    fn remove_dir(&self, path: &str) -> VfsResult<()> { self.local.remove_dir(path) }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> { self.local.mkdir_all(path) }
}

fn path(path: &std::path::Path) -> String { path.to_str().unwrap().replace('\\', "/") }

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_vfs_serial_reader_finishes_before_destination_access() {
    enabled();
    let directory = tempfile::tempdir().unwrap();
    let root = path(directory.path());
    let source = path(&directory.path().join("source.txt"));
    let destination = path(&directory.path().join("copy.txt"));
    let second = path(&directory.path().join("second.txt"));
    std::fs::write(&source, b"serial source").unwrap();
    std::fs::write(&destination, b"old explicit replacement").unwrap();
    let active = Arc::new(AtomicBool::new(false));
    let source_backend = FixtureBackend::new(&root, active.clone(), false);
    let alias = FixtureBackend::new(&root, active.clone(), false);
    assert_eq!(source_backend.copy_file(&source, &destination).unwrap(), 13);
    assert_eq!(super::copy_between(&source_backend, &source, &alias, &second).unwrap(), 13);
    assert!(!active.load(Ordering::SeqCst));
    assert_eq!(std::fs::read(&source).unwrap(), b"serial source");
    assert_eq!(std::fs::read(&destination).unwrap(), b"serial source");
    assert_eq!(std::fs::read(&second).unwrap(), b"serial source");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_vfs_cache_forwards_private_copy_and_read_contracts() {
    enabled();
    let directory = tempfile::tempdir().unwrap();
    let root = path(directory.path());
    let stage = path(&directory.path().join("private.stage"));
    let destination = path(&directory.path().join("visible.txt"));
    let inner = Arc::new(FixtureBackend::new(&root, Arc::new(AtomicBool::new(false)), true));
    let cached = CachingBackend::new(inner.clone());
    assert!(cached.list_dir(&root).unwrap().is_empty());
    assert_eq!(cached.read_size(&stage, 0).unwrap(), None);
    assert_eq!(cached.open_write_new(&stage).err().unwrap().kind(), io::ErrorKind::Unsupported);
    let mut writer = cached.open_write_copy_stage(&stage).unwrap();
    writer.write_all(b"copied").unwrap();
    writer.flush().unwrap();
    drop(writer);
    assert!(cached.list_dir(&root).unwrap().iter().any(|meta| meta.name == "private.stage"));
    cached.promote_copy_stage(&stage, &destination).unwrap();
    let files = cached.list_dir(&root).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "visible.txt");
    assert_eq!(std::fs::read(&destination).unwrap(), b"copied");
    assert_eq!(inner.opens.load(Ordering::SeqCst), 1);
    assert_eq!(inner.promotions.load(Ordering::SeqCst), 1);
}
