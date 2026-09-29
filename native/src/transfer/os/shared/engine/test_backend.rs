//! Test doubles for the engine: a "remote" backend over a local folder with
//! controllable faults, capabilities and counters, and helpers that run a job
//! to its terminal message.
pub(super) use super::test_run::{collect, job, run, Finished};
use crate::vfs::{
    Backend, BackendHandle, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink,
    LocalBackend, Scheme, VfsMeta, VfsResult,
};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A unique name for flows and namespaces of one test.
pub(super) fn unique(tag: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!(
        "transfer-engine-task:{tag}:{}:{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub(super) fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture folder");
    }
    std::fs::write(path, bytes).expect("fixture file");
}

/// A read that fails: before any byte, or after `after` bytes.
#[derive(Clone, Copy, Debug)]
pub(super) struct ReadFault {
    pub after: Option<usize>,
    pub kind: io::ErrorKind,
}

#[derive(Default)]
pub(super) struct Counters {
    pub opens: AtomicUsize,
    pub opens_at: AtomicUsize,
    pub stages: AtomicUsize,
    pub promotes: AtomicUsize,
    pub server_copies: AtomicUsize,
    pub put_batches: AtomicUsize,
    pub get_batches: AtomicUsize,
    pub active: AtomicUsize,
    pub max_active: AtomicUsize,
    pub reads_open: AtomicUsize,
    pub writes_open: AtomicUsize,
    pub overlap: AtomicBool,
    pub opened: Mutex<Vec<String>>,
    pub batched: Mutex<Vec<String>>,
}

struct Guard {
    counters: Arc<Counters>,
    reading: bool,
}

impl Guard {
    fn new(counters: &Arc<Counters>, reading: bool) -> Self {
        let active = counters.active.fetch_add(1, Ordering::SeqCst) + 1;
        counters.max_active.fetch_max(active, Ordering::SeqCst);
        let (mine, other) = if reading {
            (&counters.reads_open, &counters.writes_open)
        } else {
            (&counters.writes_open, &counters.reads_open)
        };
        mine.fetch_add(1, Ordering::SeqCst);
        if other.load(Ordering::SeqCst) > 0 {
            counters.overlap.store(true, Ordering::SeqCst);
        }
        Self {
            counters: counters.clone(),
            reading,
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.counters.active.fetch_sub(1, Ordering::SeqCst);
        let mine = if self.reading {
            &self.counters.reads_open
        } else {
            &self.counters.writes_open
        };
        mine.fetch_sub(1, Ordering::SeqCst);
    }
}

struct FakeReader {
    inner: Box<dyn Read + Send>,
    fault: Option<ReadFault>,
    delivered: usize,
    delay: Duration,
    extra: Option<Vec<u8>>,
    _guard: Guard,
}

impl Read for FakeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        if let Some(fault) = self.fault {
            let allowed = fault.after.unwrap_or(0).saturating_sub(self.delivered);
            if allowed == 0 {
                return Err(io::Error::new(fault.kind, "fixture read fault"));
            }
            let limit = allowed.min(buffer.len());
            let read = self.inner.read(&mut buffer[..limit])?;
            self.delivered += read;
            return Ok(read);
        }
        let read = self.inner.read(buffer)?;
        if read == 0 {
            if let Some(extra) = self.extra.take() {
                let length = extra.len().min(buffer.len());
                buffer[..length].copy_from_slice(&extra[..length]);
                return Ok(length);
            }
        }
        self.delivered += read;
        Ok(read)
    }
}

struct FakeWriter {
    inner: Box<dyn Write + Send>,
    fault: Option<io::ErrorKind>,
    _guard: Guard,
}

impl Write for FakeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.fault {
            return Err(io::Error::new(kind, "fixture write fault"));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A remote connection over a local folder.
pub(super) struct Fake {
    inner: LocalBackend,
    pub identity: String,
    pub key: String,
    pub ceiling: Option<usize>,
    pub md5: HashMap<String, String>,
    pub read_faults: Mutex<VecDeque<ReadFault>>,
    pub write_fault: Option<io::ErrorKind>,
    pub promote_fault_after: Mutex<Option<io::ErrorKind>>,
    pub resumable: bool,
    pub server_copy: bool,
    /// The server copy fails with this kind after it was counted.
    pub server_copy_error: Option<io::ErrorKind>,
    pub concurrent: bool,
    pub batch: Option<BatchLimits>,
    pub batch_ambiguous: bool,
    pub read_delay: Duration,
    pub extra: Option<Vec<u8>>,
    pub counters: Arc<Counters>,
}

impl Fake {
    pub(super) fn new(tag: &str) -> Self {
        Self {
            inner: LocalBackend::new("/"),
            identity: unique(tag),
            key: unique(tag),
            ceiling: None,
            md5: HashMap::new(),
            read_faults: Mutex::new(VecDeque::new()),
            write_fault: None,
            promote_fault_after: Mutex::new(None),
            resumable: false,
            server_copy: false,
            server_copy_error: None,
            concurrent: true,
            batch: None,
            batch_ambiguous: false,
            read_delay: Duration::ZERO,
            extra: None,
            counters: Arc::new(Counters::default()),
        }
    }

    pub(super) fn handle(self) -> (Arc<Fake>, BackendHandle) {
        let fake = Arc::new(self);
        let handle: BackendHandle = fake.clone();
        (fake, handle)
    }

    fn reader(&self, path: &str, inner: Box<dyn Read + Send>) -> Box<dyn Read + Send> {
        self.counters
            .opened
            .lock()
            .expect("fixture lock")
            .push(path.to_string());
        let fault = self.read_faults.lock().expect("fixture lock").pop_front();
        Box::new(FakeReader {
            inner,
            fault,
            delivered: 0,
            delay: self.read_delay,
            extra: self.extra.clone(),
            _guard: Guard::new(&self.counters, true),
        })
    }

    fn with_md5(&self, dir: &str, mut meta: VfsMeta) -> VfsMeta {
        let path = format!("{}/{}", dir.trim_end_matches('/'), meta.name);
        if let Some(md5) = self.md5.get(&path) {
            meta.content_md5 = Some(md5.clone());
        }
        meta
    }
}

impl Backend for Fake {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.identity.clone()
    }
    fn namespace_identity(&self) -> String {
        self.identity.clone()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        Ok(self
            .inner
            .list_dir(path)?
            .into_iter()
            .map(|meta| self.with_md5(path, meta))
            .collect())
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let mut meta = self.inner.stat(path)?;
        if let Some(md5) = self.md5.get(path) {
            meta.content_md5 = Some(md5.clone());
        }
        Ok(meta)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.counters.opens.fetch_add(1, Ordering::SeqCst);
        let inner = self.inner.open_read(path)?;
        Ok(self.reader(path, inner))
    }
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        if !self.resumable {
            return Ok(None);
        }
        self.counters.opens_at.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .inner
            .open_read_at(path, id, offset)?
            .map(|inner| self.reader(path, inner)))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::other(
            "the engine never opens a replacing writer",
        ))
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.counters.stages.fetch_add(1, Ordering::SeqCst);
        let inner = self.inner.open_write_new(path)?;
        Ok(Box::new(FakeWriter {
            inner,
            fault: self.write_fault,
            _guard: Guard::new(&self.counters, false),
        }))
    }
    fn promote_copy_stage(&self, stage: &str, destination: &str) -> VfsResult<()> {
        self.counters.promotes.fetch_add(1, Ordering::SeqCst);
        self.inner.promote_staged_no_replace(stage, destination)?;
        if let Some(kind) = self
            .promote_fault_after
            .lock()
            .expect("fixture lock")
            .take()
        {
            return Err(io::Error::new(
                kind,
                "fixture: published, then the answer was lost",
            ));
        }
        Ok(())
    }
    fn server_copy_to_stage(&self, src: &str, stage: &str, _size: u64) -> VfsResult<Option<u64>> {
        if !self.server_copy {
            return Ok(None);
        }
        self.counters.server_copies.fetch_add(1, Ordering::SeqCst);
        if let Some(kind) = self.server_copy_error {
            return Err(io::Error::new(kind, "fixture: server copy refused"));
        }
        let mut reader = self.inner.open_read(src)?;
        let mut writer = self.inner.open_write_new(stage)?;
        let copied = io::copy(&mut reader, &mut writer)?;
        writer.flush()?;
        Ok(Some(copied))
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(src, dst)
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
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.create_dir(path)
    }
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.inner.create_dir_new(path)
    }
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        self.inner.discard_copy_stage(stage)
    }
    fn case_sensitive_paths(&self, _root: &str) -> bool {
        true
    }
    fn flow_key(&self, _path: &str) -> String {
        self.key.clone()
    }
    fn transfer_ceiling(&self, _path: &str) -> Option<usize> {
        self.ceiling
    }
    fn concurrent_read_write(&self) -> bool {
        self.concurrent
    }
    fn batch_limits(&self, _dir: &str) -> Option<BatchLimits> {
        self.batch
    }
    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        self.counters.put_batches.fetch_add(1, Ordering::SeqCst);
        let mut outcomes = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            if self.batch_ambiguous && index == 1 {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "fixture: packet answer lost",
                ));
            }
            self.counters
                .batched
                .lock()
                .expect("fixture lock")
                .push(entry.path.clone());
            let mut bytes = vec![0u8; entry.size as usize];
            data.read_exact(&mut bytes)?;
            let stage = format!("{}.fixture-stage", entry.path);
            let mut writer = self.inner.open_write_new(&stage)?;
            writer.write_all(&bytes)?;
            writer.flush()?;
            drop(writer);
            let published = entry.path.clone();
            match self.inner.promote_staged_no_replace(&stage, &published) {
                Ok(()) => outcomes.push(BatchPutOutcome::Published(published)),
                Err(error) => {
                    let _ = self.inner.remove_file(&stage);
                    outcomes.push(BatchPutOutcome::Failed(error));
                }
            }
        }
        Ok(outcomes)
    }
    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        self.counters.get_batches.fetch_add(1, Ordering::SeqCst);
        for (index, item) in items.iter().enumerate() {
            let bytes = match std::fs::read(&item.path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    sink.failed(index, error)?;
                    continue;
                }
            };
            sink.begin(index, bytes.len() as u64)?;
            for chunk in bytes.chunks(7) {
                sink.data(index, chunk)?;
            }
            sink.end(index, Ok(()))?;
        }
        Ok(())
    }
}
