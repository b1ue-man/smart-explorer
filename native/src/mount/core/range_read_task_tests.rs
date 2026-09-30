use super::optimization_fixture::FixtureDirectory;
use super::*;
use crate::vfs::{Backend, RootConfinement, Scheme, VfsMeta};
use std::io::{self, Read, Write};
use std::sync::{Arc, atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering}};

pub(crate) struct RangeBackend {
    pub size: u64,
    pub bytes: Arc<AtomicUsize>,
    pub range_reads: AtomicUsize,
    pub full_reads: AtomicUsize,
    pub version: Arc<AtomicI64>,
    pub ranges: AtomicBool,
    pub short: AtomicBool,
    pub change_on_read: AtomicBool,
}

impl RangeBackend {
    pub(crate) fn new(size: u64) -> Arc<Self> {
        Arc::new(Self { size, bytes: Arc::new(AtomicUsize::new(0)),
            range_reads: AtomicUsize::new(0), full_reads: AtomicUsize::new(0),
            version: Arc::new(AtomicI64::new(1)), ranges: AtomicBool::new(true),
            short: AtomicBool::new(false), change_on_read: AtomicBool::new(false) })
    }

    fn reader(&self, offset: u64) -> Box<dyn Read + Send> {
        Box::new(PatternReader { offset,
            end: if self.short.load(Ordering::SeqCst) { offset } else { self.size },
            bytes: self.bytes.clone(), version: self.version.clone(),
            change: self.change_on_read.load(Ordering::SeqCst), interrupted: true })
    }
}

struct PatternReader {
    offset: u64, end: u64, bytes: Arc<AtomicUsize>, version: Arc<AtomicI64>,
    change: bool, interrupted: bool,
}

impl Read for PatternReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupted) { return Err(io::ErrorKind::Interrupted.into()); }
        let count = self.end.saturating_sub(self.offset).min(out.len() as u64).min(113) as usize;
        for (index, byte) in out[..count].iter_mut().enumerate() { *byte = ((self.offset + index as u64) % 251) as u8; }
        self.offset += count as u64;
        self.bytes.fetch_add(count, Ordering::SeqCst);
        if std::mem::take(&mut self.change) { self.version.fetch_add(1, Ordering::SeqCst); }
        Ok(count)
    }
}

impl Backend for RangeBackend {
    fn scheme(&self) -> Scheme { Scheme::Sftp }
    fn root_display(&self) -> String { "/".into() }
    fn stat(&self, path: &str) -> io::Result<VfsMeta> {
        match path {
            "/" | "/root" => Ok(VfsMeta { name: "root".into(), is_dir: true, ..Default::default() }),
            "/large" | "/root/large" => Ok(VfsMeta { name: "large".into(), size: self.size,
                mtime_ms: self.version.load(Ordering::SeqCst), id: Some("private-provider-id".into()), ..Default::default() }),
            "/root/link" => Ok(VfsMeta { name: "link".into(), is_symlink: true, ..Default::default() }),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }
    fn list_dir(&self, _: &str) -> io::Result<Vec<VfsMeta>> { Ok(vec![self.stat("/root/large")?, self.stat("/root/link")?]) }
    fn open_read(&self, _: &str) -> io::Result<Box<dyn Read + Send>> {
        self.full_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.reader(0))
    }
    fn open_read_at(&self, path: &str, id: Option<&str>, offset: u64) -> io::Result<Option<Box<dyn Read + Send>>> {
        assert!(matches!(path, "/large" | "/root/large"));
        if path.starts_with("/root/") { assert_eq!(id, None, "rooted IDs must never bypass path checks"); }
        self.range_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.ranges.load(Ordering::SeqCst).then(|| self.reader(offset)))
    }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> { Err(io::ErrorKind::PermissionDenied.into()) }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> { Err(io::ErrorKind::PermissionDenied.into()) }
    fn remove_file(&self, _: &str) -> io::Result<()> { Err(io::ErrorKind::PermissionDenied.into()) }
    fn remove_dir(&self, _: &str) -> io::Result<()> { Err(io::ErrorKind::PermissionDenied.into()) }
    fn mkdir_all(&self, _: &str) -> io::Result<()> { Err(io::ErrorKind::PermissionDenied.into()) }
    fn case_sensitive_paths(&self, _: &str) -> bool { true }
    fn root_confinement(&self, _: &str) -> RootConfinement { RootConfinement::Enforced }
}

fn mount(dir: &FixtureDirectory, backend: &Arc<RangeBackend>, mode: MountMode) -> MountEngine {
    MountEngine::open_host_cache(MountRuntimeConfig::new(MountId::parse("range-task").unwrap(), mode)
        .with_cache_policy(MountCachePolicy::new(0).unwrap()), backend.clone(), dir.path()).unwrap()
}

#[test]
fn mount_recovery_cache_task_large_read_offsets_eof_and_no_disk_spool() {
    let dir = FixtureDirectory::new();
    let backend = RangeBackend::new(450 * 1024 * 1024 * 1024);
    let engine = mount(&dir, &backend, MountMode::ReadOnly);
    let handle = engine.open_file("\\large", OpenFileOptions {
        writable: false, disposition: OpenDisposition::OpenExisting,
    }).unwrap();
    assert_eq!(engine.read(handle, 0, &mut []).unwrap(), 0);
    for offset in [0, 32, backend.size - 15, backend.size, u64::MAX] {
        let mut out = [0; 1024];
        let count = engine.read(handle, offset, &mut out).unwrap();
        assert_eq!(count as u64, backend.size.saturating_sub(offset).min(1024));
        for (index, byte) in out[..count].iter().enumerate() { assert_eq!(*byte, ((offset + index as u64) % 251) as u8); }
    }
    assert_eq!(backend.bytes.load(Ordering::SeqCst), 2063);
    assert_eq!(backend.full_reads.load(Ordering::SeqCst), 0);
    assert_eq!(fs_count(&dir), 0);
    engine.close(handle).unwrap();
    assert_eq!(fs_count(&dir), 0);
}

#[test]
fn mount_recovery_cache_task_range_rejects_short_or_changed_data_and_retries() {
    let dir = FixtureDirectory::new();
    let backend = RangeBackend::new(450 * 1024 * 1024 * 1024);
    let engine = mount(&dir, &backend, MountMode::ReadOnly);
    let open = || engine.open_metadata_file("\\large", backend.stat("/large").unwrap(), false).unwrap();
    let handle = open();
    backend.short.store(true, Ordering::SeqCst);
    assert_eq!(engine.read(handle, 0, &mut [0; 16]).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    backend.short.store(false, Ordering::SeqCst);
    assert_eq!(engine.read(handle, 0, &mut [0; 16]).unwrap(), 16);
    backend.change_on_read.store(true, Ordering::SeqCst);
    assert_eq!(engine.read(handle, 0, &mut [0; 16]).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    backend.change_on_read.store(false, Ordering::SeqCst);
    assert_eq!(engine.read(handle, 0, &mut [0; 16]).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    engine.close(handle).unwrap();
    let reopened = open();
    assert_eq!(engine.read(reopened, 0, &mut [0; 16]).unwrap(), 16);
    engine.close(reopened).unwrap();
    assert_eq!(fs_count(&dir), 0);
}

#[test]
fn mount_recovery_cache_task_fallback_and_rw_keep_whole_file_semantics() {
    for mode in [MountMode::ReadOnly, MountMode::ReadWrite] {
        let dir = FixtureDirectory::new();
        let backend = RangeBackend::new(1024 * 1024 + 16);
        backend.ranges.store(false, Ordering::SeqCst);
        let engine = mount(&dir, &backend, mode);
        let handle = engine.open_metadata_file("\\large", backend.stat("/large").unwrap(), false).unwrap();
        assert_eq!(engine.read(handle, 1024, &mut [0; 16]).unwrap(), 16);
        assert_eq!(backend.full_reads.load(Ordering::SeqCst), 1);
        assert_eq!(fs_count(&dir), 1);
        engine.close(handle).unwrap();
        assert_eq!(fs_count(&dir), 0);
    }
}

fn fs_count(dir: &FixtureDirectory) -> usize {
    std::fs::read_dir(dir.path().join("range-task/files")).unwrap().count()
}
