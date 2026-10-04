//! Test-only VFS adapter: real local I/O, explicit faults and transfer observations.
use crate::vfs::{Backend, BackendExtensions, LocalBackend, Scheme, VfsMeta};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub(super) struct Probe {
    pub(super) inner: LocalBackend,
    root: String,
    pub(super) stage_opens: AtomicUsize,
    pub(super) promotions: AtomicUsize,
    pub(super) listings: AtomicUsize,
    pub(super) active: Arc<AtomicUsize>,
    pub(super) peak: Arc<AtomicUsize>,
    pub(super) write_fault: Mutex<Option<(io::ErrorKind, usize)>>,
    pub(super) read_fault: AtomicBool,
    pub(super) read_fault_hits: AtomicUsize,
    pub(super) cancel_on_read: Mutex<Option<Arc<AtomicBool>>>,
    pub(super) omission: Mutex<Option<(String, crate::vfs::OmissionReason)>>,
    pub(super) listing_fault: Mutex<Option<io::ErrorKind>>,
}

impl Probe {
    pub(super) fn new(root: String) -> Self {
        Self {
            inner: LocalBackend::new(&root), root,
            stage_opens: AtomicUsize::new(0), promotions: AtomicUsize::new(0),
            listings: AtomicUsize::new(0), active: Arc::new(AtomicUsize::new(0)),
            peak: Arc::new(AtomicUsize::new(0)), write_fault: Mutex::new(None),
            read_fault: AtomicBool::new(false), cancel_on_read: Mutex::new(None),
            read_fault_hits: AtomicUsize::new(0),
            omission: Mutex::new(None),
            listing_fault: Mutex::new(None),
        }
    }

    fn metadata(&self, mut meta: VfsMeta) -> VfsMeta {
        meta.hidden |= meta.name == "hidden.bin";
        meta
    }

    fn writer(&self, path: &str, size: u64) -> io::Result<Box<dyn Write + Send>> {
        self.stage_opens.fetch_add(1, Ordering::SeqCst);
        let mut fault = self.write_fault.lock().unwrap();
        if let Some((kind, remaining)) = fault.as_mut() {
            if *remaining > 0 {
                *remaining -= 1;
                return Err(io::Error::new(*kind, "task fixture stage creation failed"));
            }
        }
        drop(fault);
        let writer = self.inner.open_write_copy_stage_sized(path, size)?;
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        Ok(Box::new(TrackedWriter { writer, active: self.active.clone() }))
    }

    fn reader(&self, path: &str, regular: bool, id: Option<&str>) -> io::Result<Box<dyn Read + Send>> {
        let file = path == format!("{}/file.txt", self.root);
        if file && self.read_fault.load(Ordering::Acquire) {
            self.read_fault_hits.fetch_add(1, Ordering::SeqCst);
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "task backup read denied"));
        }
        let reader = if regular { crate::vfs::open_read_regular(&self.inner, path, id)? }
            else { self.inner.open_read(path)? };
        if file {
            if let Some(cancel) = self.cancel_on_read.lock().unwrap().take() {
                return Ok(Box::new(CancelReader { reader, cancel }));
            }
        }
        Ok(reader)
    }
}

struct TrackedWriter {
    writer: Box<dyn Write + Send>,
    active: Arc<AtomicUsize>,
}
impl Write for TrackedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.writer.write(bytes) }
    fn flush(&mut self) -> io::Result<()> { self.writer.flush() }
}
impl Drop for TrackedWriter {
    fn drop(&mut self) { self.active.fetch_sub(1, Ordering::SeqCst); }
}
struct CancelReader {
    reader: Box<dyn Read + Send>,
    cancel: Arc<AtomicBool>,
}
impl Read for CancelReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(bytes)?;
        if n > 0 { self.cancel.store(true, Ordering::Release); }
        Ok(n)
    }
}

impl Backend for Probe {
    fn scheme(&self) -> Scheme { self.inner.scheme() }
    fn root_display(&self) -> String { self.root.clone() }
    fn state_identity(&self) -> String { format!("sync-task:{}", self.root) }
    fn namespace_identity(&self) -> String { self.inner.namespace_identity() }
    fn extensions(&self) -> Option<&dyn BackendExtensions> { Some(self) }
    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        if path.trim_end_matches('/') == self.root.trim_end_matches('/') {
            self.listings.fetch_add(1, Ordering::SeqCst);
            if let Some(kind) = *self.listing_fault.lock().unwrap() {
                return Err(io::Error::new(kind, "task directory enumeration failed"));
            }
        }
        Ok(self.inner.list_dir(path)?.into_iter().map(|m| self.metadata(m)).collect())
    }
    fn stat(&self, path: &str) -> io::Result<VfsMeta> {
        self.inner.stat(path).map(|m| self.metadata(m))
    }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        self.reader(path, false, None)
    }
    fn open_write(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }
    fn open_write_copy_stage_sized(&self, path: &str, size: u64) -> io::Result<Box<dyn Write + Send>> {
        self.writer(path, size)
    }
    fn rename(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename(source, destination)
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename_no_replace(source, destination)
    }
    fn promote_staged(&self, stage: &str, destination: &str) -> io::Result<()> {
        self.promotions.fetch_add(1, Ordering::SeqCst);
        self.inner.promote_staged(stage, destination)
    }
    fn promote_staged_no_replace(&self, stage: &str, destination: &str) -> io::Result<()> {
        self.promotions.fetch_add(1, Ordering::SeqCst);
        self.inner.promote_staged_no_replace(stage, destination)
    }
    fn remove_file(&self, path: &str) -> io::Result<()> { self.inner.remove_file(path) }
    fn remove_dir(&self, path: &str) -> io::Result<()> { self.inner.remove_dir(path) }
    fn mkdir_all(&self, path: &str) -> io::Result<()> { self.inner.mkdir_all(path) }
    fn discard_copy_stage(&self, path: &str) -> io::Result<()> { self.inner.discard_copy_stage(path) }
    fn rename_overwrites(&self) -> bool { true }
    fn case_sensitive_paths(&self, root: &str) -> bool { self.inner.case_sensitive_paths(root) }
    // Deliberately skip local-only shortcuts so failures pass the shared VFS boundary.
    fn is_local(&self) -> bool { false }
}
impl BackendExtensions for Probe {
    fn list_dir_tolerant(&self, path: &str) -> io::Result<crate::vfs::VfsListing> {
        let mut listing = crate::vfs::VfsListing::complete(self.list_dir(path)?);
        if path.trim_end_matches('/') == self.root.trim_end_matches('/') {
            if let Some((name, reason)) = self.omission.lock().unwrap().as_ref() {
                assert!(listing.entries.iter().any(|m| m.name == *name));
                listing.entries.retain(|m| m.name != *name);
                listing.omitted.push(crate::vfs::VfsOmission {
                    rel: name.clone(), reason: *reason, detail: "task protected child".into(),
                });
            }
        }
        Ok(listing)
    }
    fn open_read_regular(&self, path: &str, id: Option<&str>) -> io::Result<Box<dyn Read + Send>> {
        self.reader(path, true, id)
    }
    fn finish_stage(&self, path: &str, finish: crate::vfs::StageFinish) -> io::Result<crate::vfs::StageFinished> {
        crate::vfs::finish_stage(&self.inner, path, finish)
    }
    fn confirm_namespace(&self, path: &str) -> io::Result<bool> {
        crate::vfs::confirm_namespace(&self.inner, path)
    }
    fn target_limits(&self, path: &str) -> crate::vfs::TargetLimits {
        crate::vfs::target_limits(&self.inner, path)
    }
    fn volume_identity(&self, path: &str) -> io::Result<Option<crate::vfs::VolumeIdentity>> {
        crate::vfs::volume_identity(&self.inner, path)
    }
}
