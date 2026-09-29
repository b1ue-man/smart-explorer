//! Test doubles for remote virtual files: an in-memory remote with counters
//! and gates, a small memory budget, an OLE apartment guard and descriptor
//! parsing the way Explorer reads it.
use super::data_object::Formats;
use super::handoff::{Held, Memory, RemoteContent};
use crate::transfer::{flow, ExternalSnapshot, Flow, ListedEntry, SelectionListing};
use std::collections::HashSet;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_UNEXPECTED, HGLOBAL, S_FALSE};
use windows::Win32::System::Com::{
    IDataObject, IStream, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED, TYMED_HGLOBAL,
    TYMED_ISTREAM,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize, ReleaseStgMedium};
use windows::Win32::UI::Shell::FILEDESCRIPTORW;

/// Generous for slow runners; waits end as soon as the condition holds.
const DEADLINE: Duration = Duration::from_secs(20);

pub(super) fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// OLE on the test thread (an STA, like the GUI thread).
pub(super) struct Apartment;

impl Apartment {
    pub(super) fn enter() -> Self {
        unsafe { OleInitialize(None) }.expect("OLE initializes on the test thread");
        Self
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

#[derive(Clone)]
struct FakeEntry {
    rel: String,
    bytes: Vec<u8>,
    is_dir: bool,
    size_known: bool,
    mtime_ms: i64,
    fail_after: Option<usize>,
    wait_for: Option<String>,
}

impl FakeEntry {
    fn file(rel: &str, bytes: Vec<u8>) -> Self {
        Self {
            rel: rel.to_string(),
            bytes,
            is_dir: false,
            size_known: true,
            mtime_ms: 0,
            fail_after: None,
            wait_for: None,
        }
    }
}

/// What the remote saw: listings, opened and fully read files, and how many
/// readers were open at once.
#[derive(Default)]
pub(super) struct Stats {
    listings: AtomicUsize,
    open_now: AtomicUsize,
    max_open: AtomicUsize,
    opened: Mutex<Vec<String>>,
    finished: Mutex<HashSet<String>>,
    changed: Condvar,
}

impl Stats {
    pub(super) fn listings(&self) -> usize {
        self.listings.load(Ordering::SeqCst)
    }

    pub(super) fn max_open(&self) -> usize {
        self.max_open.load(Ordering::SeqCst)
    }

    pub(super) fn opened(&self, rel: &str) -> bool {
        self.opened.lock().unwrap().iter().any(|name| name == rel)
    }

    pub(super) fn finished(&self, rel: &str) -> bool {
        self.finished.lock().unwrap().contains(rel)
    }

    fn open(&self, rel: &str) {
        let now = self.open_now.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_open.fetch_max(now, Ordering::SeqCst);
        self.opened.lock().unwrap().push(rel.to_string());
        self.changed.notify_all();
    }

    /// Blocks a reader until `rel` was opened (bounded, so a broken
    /// prefetch fails the test instead of hanging it).
    fn wait_opened(&self, rel: &str) {
        let deadline = Instant::now() + DEADLINE;
        let mut opened = self.opened.lock().unwrap();
        while !opened.iter().any(|name| name == rel) && Instant::now() < deadline {
            opened = self
                .changed
                .wait_timeout(opened, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }
}

pub(super) struct FakeRemote {
    label: String,
    ceiling: usize,
    entries: Vec<FakeEntry>,
    problems: Vec<(String, String)>,
    pub(super) stats: Arc<Stats>,
}

impl FakeRemote {
    pub(super) fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            ceiling: 4,
            entries: Vec::new(),
            problems: Vec::new(),
            stats: Arc::new(Stats::default()),
        }
    }

    pub(super) fn ceiling(mut self, ceiling: usize) -> Self {
        self.ceiling = ceiling;
        self
    }

    pub(super) fn dir(mut self, rel: &str) -> Self {
        let mut entry = FakeEntry::file(rel, Vec::new());
        entry.is_dir = true;
        self.entries.push(entry);
        self
    }

    pub(super) fn file(mut self, rel: &str, bytes: Vec<u8>, mtime_ms: i64) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.mtime_ms = mtime_ms;
        self.entries.push(entry);
        self
    }

    /// A provider export: its size is only known after reading it.
    pub(super) fn export(mut self, rel: &str, bytes: Vec<u8>) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.size_known = false;
        self.entries.push(entry);
        self
    }

    /// Fails with a connection error after `after` bytes.
    pub(super) fn failing(mut self, rel: &str, bytes: Vec<u8>, after: usize) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.fail_after = Some(after);
        self.entries.push(entry);
        self
    }

    /// An entry the listing could not read.
    pub(super) fn problem(mut self, path: &str, message: &str) -> Self {
        self.problems.push((path.to_string(), message.to_string()));
        self
    }

    /// Its reader waits until `other` was opened.
    pub(super) fn gated(mut self, rel: &str, bytes: Vec<u8>, other: &str) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.wait_for = Some(other.to_string());
        self.entries.push(entry);
        self
    }

    fn path(rel: &str) -> String {
        format!("/fake/{rel}")
    }
}

impl RemoteContent for FakeRemote {
    fn display_label(&self) -> String {
        self.label.clone()
    }

    fn list_entries(
        &self,
        _cancel: &AtomicBool,
        on_found: &(dyn Fn(u64) + Sync),
    ) -> SelectionListing {
        self.stats.listings.fetch_add(1, Ordering::SeqCst);
        let entries: Vec<ListedEntry> = self
            .entries
            .iter()
            .map(|entry| ListedEntry {
                rel: entry.rel.clone(),
                path: Self::path(&entry.rel),
                id: None,
                size: if entry.size_known {
                    entry.bytes.len() as u64
                } else {
                    0
                },
                size_known: entry.size_known,
                mtime_ms: entry.mtime_ms,
                is_dir: entry.is_dir,
            })
            .collect();
        on_found(entries.len() as u64);
        SelectionListing {
            entries,
            problems: self.problems.clone(),
            omitted: 0,
            complete: true,
        }
    }

    fn open_entry(&self, entry: &ListedEntry) -> io::Result<Box<dyn Read + Send>> {
        let fake = self
            .entries
            .iter()
            .find(|fake| Self::path(&fake.rel) == entry.path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unbekannte Testdatei"))?;
        self.stats.open(&fake.rel);
        Ok(Box::new(FakeReader {
            entry: fake.clone(),
            position: 0,
            stats: self.stats.clone(),
        }))
    }

    fn connection_flow(&self) -> Arc<Flow> {
        // A key of its own: learned limits of other tests do not leak in.
        flow(
            format!("transfer_engine_task {}", self.label),
            Some(self.ceiling),
        )
    }
}

struct FakeReader {
    entry: FakeEntry,
    position: usize,
    stats: Arc<Stats>,
}

impl Read for FakeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if let Some(other) = self.entry.wait_for.take() {
            self.stats.wait_opened(&other);
        }
        let limit = self.entry.fail_after.unwrap_or(usize::MAX);
        if self.position >= limit {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "Verbindung getrennt",
            ));
        }
        let end = self
            .entry
            .bytes
            .len()
            .min(limit)
            .min(self.position + out.len());
        let count = end - self.position;
        out[..count].copy_from_slice(&self.entry.bytes[self.position..end]);
        self.position = end;
        if count == 0 && !out.is_empty() {
            self.stats
                .finished
                .lock()
                .unwrap()
                .insert(self.entry.rel.clone());
        }
        Ok(count)
    }
}

impl Drop for FakeReader {
    fn drop(&mut self) {
        self.stats.open_now.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A budget of a few files, to watch the prefetch stay inside it.
pub(super) struct TestBudget {
    capacity: u64,
    used: Mutex<u64>,
    max_used: AtomicU64,
    freed: Condvar,
}

impl TestBudget {
    pub(super) fn leaked(capacity: u64) -> &'static Self {
        Box::leak(Box::new(Self {
            capacity,
            used: Mutex::new(0),
            max_used: AtomicU64::new(0),
            freed: Condvar::new(),
        }))
    }

    pub(super) fn used(&self) -> u64 {
        *self.used.lock().unwrap()
    }

    pub(super) fn max_used(&self) -> u64 {
        self.max_used.load(Ordering::SeqCst)
    }
}

pub(super) struct TestMemory(pub(super) &'static TestBudget);

struct TestHeld {
    budget: &'static TestBudget,
    bytes: u64,
}

impl Drop for TestHeld {
    fn drop(&mut self) {
        *self.budget.used.lock().unwrap() -= self.bytes;
        self.budget.freed.notify_all();
    }
}

impl Memory for TestMemory {
    fn capacity(&self) -> u64 {
        self.0.capacity
    }

    fn reserve(&self, bytes: u64, cancel: &AtomicBool) -> Option<Held> {
        let budget = self.0;
        let bytes = bytes.min(budget.capacity);
        let mut used = budget.used.lock().unwrap();
        loop {
            if cancel.load(Ordering::Acquire) {
                return None;
            }
            if *used + bytes <= budget.capacity {
                *used += bytes;
                budget.max_used.fetch_max(*used, Ordering::SeqCst);
                return Some(Box::new(TestHeld { budget, bytes }));
            }
            used = budget
                .freed
                .wait_timeout(used, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }
}

/// One descriptor entry as Explorer reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Described {
    pub(super) name: String,
    pub(super) flags: u32,
    pub(super) attributes: u32,
    pub(super) size: u64,
    pub(super) write_time: u64,
}

pub(super) fn request(format: u16, tymed: TYMED, lindex: i32) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex,
        tymed: tymed.0 as u32,
    }
}

pub(super) fn descriptor(object: &IDataObject, formats: &Formats) -> Result<Vec<Described>> {
    let mut medium: STGMEDIUM =
        unsafe { object.GetData(&request(formats.descriptor, TYMED_HGLOBAL, -1))? };
    let described = unsafe { parse(medium.u.hGlobal) };
    unsafe { ReleaseStgMedium(&mut medium) };
    Ok(described)
}

unsafe fn parse(handle: HGLOBAL) -> Vec<Described> {
    const ENTRY: usize = std::mem::size_of::<FILEDESCRIPTORW>();
    let size = GlobalSize(handle);
    let base = GlobalLock(handle).cast::<u8>();
    assert!(!base.is_null(), "descriptor block locks");
    let count = std::ptr::read_unaligned(base.cast::<u32>()) as usize;
    assert!(
        size >= 4 + count.max(1) * ENTRY,
        "descriptor block holds every entry"
    );
    let described = (0..count)
        .map(|index| {
            let entry =
                std::ptr::read_unaligned(base.add(4 + index * ENTRY).cast::<FILEDESCRIPTORW>());
            let units = entry.cFileName;
            let length = units
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(units.len());
            let written = entry.ftLastWriteTime;
            Described {
                name: String::from_utf16(&units[..length]).expect("valid UTF-16 name"),
                flags: entry.dwFlags,
                attributes: entry.dwFileAttributes,
                size: (u64::from(entry.nFileSizeHigh) << 32) | u64::from(entry.nFileSizeLow),
                write_time: (u64::from(written.dwHighDateTime) << 32)
                    | u64::from(written.dwLowDateTime),
            }
        })
        .collect();
    let _ = GlobalUnlock(handle);
    described
}

pub(super) fn contents(object: &IDataObject, formats: &Formats, index: i32) -> Result<IStream> {
    let mut medium: STGMEDIUM =
        unsafe { object.GetData(&request(formats.contents, TYMED_ISTREAM, index))? };
    let stream = unsafe { (*medium.u.pstm).clone() };
    unsafe { ReleaseStgMedium(&mut medium) };
    stream.ok_or_else(|| Error::from(E_UNEXPECTED))
}

/// Reads until the short read that ends the stream, in odd-sized chunks.
pub(super) fn read_all(stream: &IStream) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 7_919];
    loop {
        let mut read = 0u32;
        let status = unsafe {
            stream.Read(
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                Some(&mut read),
            )
        };
        status.ok()?;
        data.extend_from_slice(&chunk[..read as usize]);
        if status == S_FALSE {
            return Ok(data);
        }
    }
}

pub(super) fn read_exact(stream: &IStream, count: usize) -> Vec<u8> {
    let mut data = vec![0u8; count];
    let mut read = 0u32;
    let status = unsafe { stream.Read(data.as_mut_ptr().cast(), count as u32, Some(&mut read)) };
    assert!(status.is_ok(), "read failed: {status:?}");
    data.truncate(read as usize);
    data
}

/// The newest transfer-list entry with `label`.
pub(super) fn external(label: &str) -> ExternalSnapshot {
    crate::transfer::external_snapshots()
        .into_iter()
        .rev()
        .find(|snapshot| snapshot.label == label)
        .expect("the hand-off appears in the transfer list")
}
