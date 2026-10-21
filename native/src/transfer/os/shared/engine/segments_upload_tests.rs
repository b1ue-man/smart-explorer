//! Large files in parts on the writing side (E13, U48-9 and U48-10): a local
//! file and a copy between two remotes are assembled from ranges in a
//! segmented stage of the target and published without replacing anything.
//! The target is the fake remote wrapped in a backend that offers a stage in
//! memory; nothing is written under the destination name before the
//! publication.
use super::test_backend::{job, run, write, Fake};
use crate::transfer::Endpoint;
use crate::vfs::{
    Backend, BackendHandle, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink, Scheme,
    SegmentedStage, VfsMeta, VfsResult,
};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const MIB: usize = 1024 * 1024;
/// Two parts at least (`SPLIT_FROM` is 16 MiB).
const SIZE: usize = 17 * MIB + 777;

fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| ((index * 7 + index / 251) % 251) as u8)
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What the target's stage did.
#[derive(Default)]
struct Stages {
    offer: bool,
    /// The next this many stage names are answered "taken".
    taken: AtomicUsize,
    attempts: AtomicUsize,
    stages: Mutex<HashMap<String, Arc<Mutex<Vec<u8>>>>>,
    ranges: Mutex<Vec<(u64, u64)>>,
    aborts: AtomicUsize,
    /// `flush` of every range fails with this kind.
    flush_fault: Option<io::ErrorKind>,
    /// Runs when the first range is opened.
    on_first: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

struct MemoryStage {
    name: String,
    size: u64,
    data: Arc<Mutex<Vec<u8>>>,
    covered: Arc<AtomicU64>,
    state: Arc<Stages>,
}

struct RangeWriter {
    data: Arc<Mutex<Vec<u8>>>,
    covered: Arc<AtomicU64>,
    offset: u64,
    expected: u64,
    accepted: u64,
    fault: Option<io::ErrorKind>,
}

impl Write for RangeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.accepted + bytes.len() as u64 > self.expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "grown"));
        }
        let at = (self.offset + self.accepted) as usize;
        lock(&self.data)[at..at + bytes.len()].copy_from_slice(bytes);
        self.accepted += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(kind) = self.fault {
            return Err(io::Error::new(kind, "fixture: the range was refused"));
        }
        if self.accepted != self.expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "short range"));
        }
        self.covered.fetch_add(self.accepted, Ordering::SeqCst);
        Ok(())
    }
}

impl SegmentedStage for MemoryStage {
    fn stage_path(&self) -> &str {
        &self.name
    }

    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        lock(&self.state.ranges).push((offset, len));
        if let Some(hook) = lock(&self.state.on_first).take() {
            hook();
        }
        if offset + len > self.size {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Box::new(RangeWriter {
            data: self.data.clone(),
            covered: self.covered.clone(),
            offset,
            expected: len,
            accepted: 0,
            fault: self.state.flush_fault,
        }))
    }

    fn finish(self: Box<Self>) -> VfsResult<()> {
        if self.covered.load(Ordering::SeqCst) != self.size {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "gaps"));
        }
        Ok(())
    }

    fn abort(self: Box<Self>) {
        self.state.aborts.fetch_add(1, Ordering::SeqCst);
        lock(&self.state.stages).remove(&self.name);
    }
}

/// The fake remote plus a segmented stage in memory.
struct Seg {
    fake: Arc<Fake>,
    state: Arc<Stages>,
}

impl Backend for Seg {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.fake.root_display()
    }
    fn namespace_identity(&self) -> String {
        self.fake.namespace_identity()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.fake.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.fake.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.fake.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.fake.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.fake.open_write_new(path)
    }
    fn open_write_fresh(
        &self,
        _path: &str,
        _size: u64,
    ) -> VfsResult<Option<Box<dyn Write + Send>>> {
        Ok(None)
    }
    fn open_segmented_stage(
        &self,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
        self.state.attempts.fetch_add(1, Ordering::SeqCst);
        if !self.state.offer {
            return Ok(None);
        }
        let forced = self
            .state
            .taken
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            });
        if forced.is_ok() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let data = Arc::new(Mutex::new(vec![0u8; size as usize]));
        lock(&self.state.stages).insert(stage.to_string(), data.clone());
        Ok(Some(Box::new(MemoryStage {
            name: stage.to_string(),
            size,
            data,
            covered: Arc::new(AtomicU64::new(0)),
            state: self.state.clone(),
        })))
    }
    fn promote_copy_stage(&self, stage: &str, destination: &str) -> VfsResult<()> {
        let held = lock(&self.state.stages).get(stage).cloned();
        let Some(data) = held else {
            return self.fake.promote_copy_stage(stage, destination);
        };
        // Published create-only: an existing name stays untouched.
        let mut writer = self.fake.inner.open_write_new(destination)?;
        writer.write_all(&lock(&data))?;
        writer.flush()?;
        lock(&self.state.stages).remove(stage);
        self.fake.counters.promotes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        if lock(&self.state.stages).remove(stage).is_some() {
            return Ok(());
        }
        self.fake.discard_copy_stage(stage)
    }
    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<u64>> {
        self.fake.server_copy_to_stage(src, stage, size, cancel)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.fake.rename(src, dst)
    }
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.fake.rename_no_replace(src, dst)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.fake.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.fake.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.fake.mkdir_all(path)
    }
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.fake.create_dir(path)
    }
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.fake.create_dir_new(path)
    }
    fn case_sensitive_paths(&self, root: &str) -> bool {
        self.fake.case_sensitive_paths(root)
    }
    fn flow_key(&self, path: &str) -> String {
        self.fake.flow_key(path)
    }
    fn batch_limits(&self, dir: &str) -> Option<BatchLimits> {
        self.fake.batch_limits(dir)
    }
    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        self.fake.put_batch(entries, data)
    }
    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        self.fake.get_batch(items, sink)
    }
}

fn target(fake: Fake, state: Stages) -> (Arc<Fake>, Arc<Stages>, BackendHandle) {
    let fake = Arc::new(fake);
    let state = Arc::new(state);
    let handle: BackendHandle = Arc::new(Seg {
        fake: fake.clone(),
        state: state.clone(),
    });
    (fake, state, handle)
}

fn offering() -> Stages {
    Stages {
        offer: true,
        ..Stages::default()
    }
}

/// A local file `big.bin` and an empty remote folder.
struct Upload {
    _root: tempfile::TempDir,
    source: PathBuf,
    remote: PathBuf,
}

impl Upload {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temp dir");
        let source = root.path().join("local/big.bin");
        write(&source, &pattern(SIZE));
        let remote = root.path().join("remote");
        fs::create_dir(&remote).expect("remote");
        Self {
            _root: root,
            source,
            remote,
        }
    }

    fn run(&self, handle: &BackendHandle) -> super::test_backend::Finished {
        run(job(
            Endpoint::Local,
            Endpoint::Remote(handle.clone()),
            &self.remote,
            &[&self.source],
        ))
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.remote)
            .expect("remote")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

#[test]
fn speed_task_segments_upload_assembles_the_file_from_ranges() {
    let upload = Upload::new();
    let (fake, state, handle) = target(Fake::new("seg-up"), offering());
    let finished = upload.run(&handle);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    assert_eq!(
        fs::read(upload.remote.join("big.bin")).expect("published"),
        pattern(SIZE)
    );
    let mut ranges = lock(&state.ranges).clone();
    ranges.sort_unstable();
    assert!(ranges.len() >= 2, "{ranges:?}");
    let mut next = 0;
    for (offset, len) in &ranges {
        assert_eq!(*offset, next, "the ranges follow each other: {ranges:?}");
        next += len;
    }
    assert_eq!(next, SIZE as u64, "and cover the file");
    assert_eq!(fake.counters.promotes.load(Ordering::SeqCst), 1);
    assert!(lock(&state.stages).is_empty(), "the stage was published");
    assert_eq!(upload.names(), vec!["big.bin".to_string()]);
    assert_eq!(finished.progress.bytes_done, SIZE as u64);
}

#[test]
fn speed_task_segments_upload_failure_aborts_the_stage() {
    let upload = Upload::new();
    let state = Stages {
        flush_fault: Some(io::ErrorKind::PermissionDenied),
        ..offering()
    };
    let (_fake, state, handle) = target(Fake::new("seg-fail"), state);
    let finished = upload.run(&handle);
    assert_eq!(finished.progress.files_done, 0);
    assert!(!finished.issues.is_empty(), "the failure is reported");
    assert!(
        state.aborts.load(Ordering::SeqCst) >= 1,
        "the stage was given up"
    );
    assert!(lock(&state.stages).is_empty(), "no stage is left");
    assert!(
        upload.names().is_empty(),
        "nothing under any name: {:?}",
        upload.names()
    );
}

#[test]
fn speed_task_segments_upload_source_change_is_refused() {
    let upload = Upload::new();
    let source = upload.source.clone();
    let state = offering();
    *lock(&state.on_first) = Some(Box::new(move || {
        // The file is touched while the first range is being opened.
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .expect("source");
        file.set_modified(SystemTime::now() + Duration::from_secs(3600))
            .expect("touch");
    }));
    let (_fake, state, handle) = target(Fake::new("seg-change"), state);
    let finished = upload.run(&handle);
    assert_eq!(finished.progress.files_done, 0);
    assert!(
        finished.has_issue("geändert"),
        "the change is named: {:?}",
        finished.issues
    );
    assert!(lock(&state.stages).is_empty());
    assert!(upload.names().is_empty(), "{:?}", upload.names());
}

#[test]
fn speed_task_segments_upload_name_taken_is_numbered() {
    let upload = Upload::new();
    write(&upload.remote.join("big.bin"), b"old");
    let state = Stages {
        taken: AtomicUsize::new(2),
        ..offering()
    };
    let (_fake, state, handle) = target(Fake::new("seg-name"), state);
    let finished = upload.run(&handle);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    // Two stage names were taken, the third was used.
    assert_eq!(state.attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        fs::read(upload.remote.join("big.bin")).expect("old"),
        b"old"
    );
    assert_eq!(
        fs::read(upload.remote.join("big (2).bin")).expect("numbered"),
        pattern(SIZE)
    );
}

#[test]
fn speed_task_segments_upload_without_segmented_stage_streams() {
    let upload = Upload::new();
    // A target that offers no stage: one stream, as before.
    let (fake, state, handle) = target(Fake::new("seg-none"), Stages::default());
    let finished = upload.run(&handle);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(
        fs::read(upload.remote.join("big.bin")).expect("published"),
        pattern(SIZE)
    );
    assert_eq!(state.attempts.load(Ordering::SeqCst), 1, "asked once");
    assert!(lock(&state.ranges).is_empty());
    assert_eq!(
        fake.counters.stages.load(Ordering::SeqCst),
        1,
        "one private stage"
    );
}

#[path = "segments_copy_tests.rs"]
mod copy_tests;
