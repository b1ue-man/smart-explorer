//! Test doubles for the engine: a "remote" backend over a local folder with
//! controllable faults, capabilities and counters, and helpers that run a job
//! to its terminal message.
pub(super) use super::test_faults::Faults;
use super::test_faults::{rewrite_same_length, RefusedFresh};
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

#[path = "test_backend_evidence.rs"]
mod evidence;
#[path = "test_backend_evidence_tests.rs"]
mod evidence_tests;
#[path = "test_backend_streams.rs"]
mod streams;
use streams::{FakeReader, FakeWriter, Guard};

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
    /// Requests the engine asked of the fake per kind (`stat`,
    /// `file_identity`, single folder creations, bulk folder creations).
    pub stats: AtomicUsize,
    pub identities: AtomicUsize,
    pub dirs: AtomicUsize,
    pub bulk_dirs: AtomicUsize,
}

/// Range reads of the fake: what it offers and what was asked of it.
#[derive(Default)]
pub(super) struct Ranges {
    pub supported: bool,
    /// Claims to read ranges but answers every request with `Ok(None)` (a
    /// server that ignores `Range`).
    pub ignored: bool,
    /// The range starting here fails once, after that many bytes.
    pub fail_once: Mutex<Option<(u64, usize, io::ErrorKind)>>,
    /// Every answer ends this many bytes early (the source shrank).
    pub short_by: u64,
    /// `(offset, len)` of every request, in arrival order.
    pub requests: Mutex<Vec<(u64, u64)>>,
    /// Requests answered with `Ok(None)`.
    pub refused: AtomicUsize,
}

/// A remote connection over a local folder.
pub(super) struct Fake {
    pub(super) inner: LocalBackend,
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
    /// Explicitly expose the fixture filesystem's durable object proofs.
    pub identity_proofs: bool,
    pub batch: Option<BatchLimits>,
    /// Reports transfer evidence like an `evidence_v1` Share host.
    pub evidence: bool,
    /// Creates folders in bulk like a `create_dirs_v1` Share host.
    pub bulk_dirs: bool,
    pub batch_ambiguous: bool,
    pub read_delay: Duration,
    pub extra: Option<Vec<u8>>,
    pub counters: Arc<Counters>,
    pub faults: Faults,
    pub ranges: Ranges,
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
            identity_proofs: false,
            batch: None,
            evidence: false,
            bulk_dirs: false,
            batch_ambiguous: false,
            read_delay: Duration::ZERO,
            extra: None,
            counters: Arc::new(Counters::default()),
            faults: Faults::default(),
            ranges: Ranges::default(),
        }
    }

    pub(super) fn handle(self) -> (Arc<Fake>, BackendHandle) {
        let fake = Arc::new(self);
        let handle: BackendHandle = fake.clone();
        (fake, handle)
    }

    fn reader(&self, path: &str, inner: Box<dyn Read + Send>) -> Box<dyn Read + Send> {
        let fault = self.read_faults.lock().expect("fixture lock").pop_front();
        self.reader_with(path, inner, fault)
    }

    fn reader_with(
        &self,
        path: &str,
        inner: Box<dyn Read + Send>,
        fault: Option<ReadFault>,
    ) -> Box<dyn Read + Send> {
        self.counters
            .opened
            .lock()
            .expect("fixture lock")
            .push(path.to_string());
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
    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        (self.identity_proofs || self.evidence || self.bulk_dirs).then_some(self)
    }
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
        self.counters.stats.fetch_add(1, Ordering::SeqCst);
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
    fn supports_range_reads(&self, _path: &str) -> bool {
        self.ranges.supported
    }
    fn open_read_range(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
        len: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        if !self.ranges.supported || self.ranges.ignored {
            self.ranges.refused.fetch_add(1, Ordering::SeqCst);
            return Ok(None);
        }
        self.ranges
            .requests
            .lock()
            .expect("fixture lock")
            .push((offset, len));
        let shortened = len.saturating_sub(self.ranges.short_by);
        let inner = self
            .inner
            .open_read_range(path, id, offset, shortened)?
            .ok_or_else(|| io::Error::other("fixture: the local backend reads ranges"))?;
        let mut pending = self.ranges.fail_once.lock().expect("fixture lock");
        let fault = match *pending {
            Some((at, after, kind)) if at == offset => {
                *pending = None;
                Some(ReadFault {
                    after: Some(after),
                    kind,
                })
            }
            _ => None,
        };
        Ok(Some(self.reader_with(path, inner, fault)))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::other(
            "the engine never opens a replacing writer",
        ))
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.counters.stages.fetch_add(1, Ordering::SeqCst);
        if let Some(refused) = self.faults.refuse_stage() {
            return Err(refused);
        }
        let inner = self.inner.open_write_new(path)?;
        Ok(Box::new(FakeWriter {
            inner,
            fault: self.write_fault,
            _guard: Guard::new(&self.counters, false),
        }))
    }
    fn open_write_fresh(
        &self,
        _path: &str,
        _size: u64,
    ) -> VfsResult<Option<Box<dyn Write + Send>>> {
        Ok(self
            .faults
            .fresh_refused
            .map(|kind| Box::new(RefusedFresh(kind)) as Box<dyn Write + Send>))
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
    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        _size: u64,
        _cancel: &AtomicBool,
    ) -> VfsResult<Option<u64>> {
        if !self.server_copy {
            return Ok(None);
        }
        self.counters.server_copies.fetch_add(1, Ordering::SeqCst);
        if let Some(kind) = self.server_copy_error {
            return Err(io::Error::new(kind, "fixture: server copy refused"));
        }
        if let Some(path) = self
            .faults
            .rewrite_before_copy
            .lock()
            .expect("fault")
            .take()
        {
            rewrite_same_length(&path)?;
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
        self.counters.dirs.fetch_add(1, Ordering::SeqCst);
        if let Some(refused) = self.faults.refuse_dir() {
            return Err(refused);
        }
        self.inner.create_dir(path)
    }
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.counters.dirs.fetch_add(1, Ordering::SeqCst);
        if let Some(refused) = self.faults.refuse_dir() {
            return Err(refused);
        }
        self.inner.create_dir_new(path)?;
        if self.faults.dir_answer_lost.swap(false, Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "fixture: created, answer lost",
            ));
        }
        Ok(())
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
        self.fake_put_batch(entries, data)
    }
    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        if self.evidence {
            return self.evidenced_get_batch(items, sink);
        }
        self.fake_get_batch(items, sink)
    }
}

impl crate::vfs::BackendExtensions for Fake {
    fn directory_reservation_supported(&self, root: &str) -> bool {
        crate::vfs::directory_reservation_supported(&self.inner, root)
    }
    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        crate::vfs::create_dir_exclusive(&self.inner, path)
    }
    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        crate::vfs::directory_identity(&self.inner, path)
    }
    fn file_identity(&self, path: &str) -> VfsResult<Option<String>> {
        self.counters.identities.fetch_add(1, Ordering::SeqCst);
        crate::vfs::file_identity(&self.inner, path)
    }
    fn transfer_evidence(&self, _path: &str) -> bool {
        self.evidence
    }
    fn put_batch_evidence(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
        prepare: &mut crate::vfs::PreparePut<'_>,
    ) -> VfsResult<Vec<crate::vfs::EvidencePutOutcome>> {
        self.evidenced_put_batch(entries, data, prepare)
    }
    fn promote_copy_stage_evidence(
        &self,
        staged: &str,
        destination: &str,
    ) -> VfsResult<Option<crate::vfs::FileEvidence>> {
        self.promote_copy_stage(staged, destination)?;
        Ok(self
            .evidence
            .then(|| self.evidence_of(destination))
            .flatten())
    }
    fn open_read_evidence(
        &self,
        path: &str,
        id: Option<&str>,
    ) -> VfsResult<Option<(Box<dyn Read + Send>, crate::vfs::FileEvidence)>> {
        self.evidenced_read(path, id)
    }
    fn take_stage_evidence(&self, stage: &str) -> Option<crate::vfs::StageEvidence> {
        self.stage_evidence(stage)
    }
    fn supports_create_dirs(&self, _root: &str) -> bool {
        self.bulk_dirs
    }
    fn create_dirs(&self, paths: &[String]) -> VfsResult<Vec<io::Result<bool>>> {
        self.bulk_create_dirs(paths)
    }
}
