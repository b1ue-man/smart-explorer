//! An in-memory remote for the hand-off tests: entries with gates, broken
//! connections and panics, readable from an offset or not, and counters of
//! what was listed, opened and read.
use super::handoff::{skip_to, RemoteContent};
use super::test_support::DEADLINE;
use crate::transfer::{flow, Flow, ListedEntry, SelectionListing};
use std::collections::HashSet;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct FakeEntry {
    rel: String,
    bytes: Vec<u8>,
    is_dir: bool,
    size_known: bool,
    mtime_ms: i64,
    /// Every reader fails once it reaches this file offset.
    fail_after: Option<usize>,
    /// Open number i fails once it reaches offset `fail_opens[i]`; later
    /// opens work.
    fail_opens: Vec<usize>,
    panic_on_read: bool,
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
            fail_opens: Vec::new(),
            panic_on_read: false,
            wait_for: None,
        }
    }
}

/// What the remote saw: listings, every open with its offset, fully read
/// and failed files, and how many readers were open at once.
#[derive(Default)]
pub(super) struct Stats {
    listings: AtomicUsize,
    open_now: AtomicUsize,
    max_open: AtomicUsize,
    opened: Mutex<Vec<(String, u64)>>,
    finished: Mutex<HashSet<String>>,
    failed: Mutex<HashSet<String>>,
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
        self.opens(rel) > 0
    }

    pub(super) fn opens(&self, rel: &str) -> usize {
        self.offsets(rel).len()
    }

    /// Start offsets of the opens of `rel`, in order.
    pub(super) fn offsets(&self, rel: &str) -> Vec<u64> {
        let opened = self.opened.lock().unwrap();
        opened
            .iter()
            .filter(|(name, _)| name == rel)
            .map(|(_, offset)| *offset)
            .collect()
    }

    pub(super) fn finished(&self, rel: &str) -> bool {
        self.finished.lock().unwrap().contains(rel)
    }

    pub(super) fn failed(&self, rel: &str) -> bool {
        self.failed.lock().unwrap().contains(rel)
    }

    /// Records an open; returns how many opens of `rel` came before.
    fn open(&self, rel: &str, offset: u64) -> usize {
        let now = self.open_now.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_open.fetch_max(now, Ordering::SeqCst);
        let mut opened = self.opened.lock().unwrap();
        let earlier = opened.iter().filter(|(name, _)| name == rel).count();
        opened.push((rel.to_string(), offset));
        drop(opened);
        self.changed.notify_all();
        earlier
    }

    /// Blocks a reader until `rel` was opened (bounded, so a broken
    /// prefetch fails the test instead of hanging it).
    fn wait_opened(&self, rel: &str) {
        let deadline = Instant::now() + DEADLINE;
        let mut opened = self.opened.lock().unwrap();
        while !opened.iter().any(|(name, _)| name == rel) && Instant::now() < deadline {
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
    seekable: bool,
    panic_on_list: bool,
    entries: Vec<FakeEntry>,
    problems: Vec<(String, String)>,
    pub(super) stats: Arc<Stats>,
}

impl FakeRemote {
    pub(super) fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            ceiling: 4,
            seekable: true,
            panic_on_list: false,
            entries: Vec::new(),
            problems: Vec::new(),
            stats: Arc::new(Stats::default()),
        }
    }

    pub(super) fn ceiling(mut self, ceiling: usize) -> Self {
        self.ceiling = ceiling;
        self
    }

    /// Like a connection that cannot start mid-file.
    pub(super) fn not_seekable(mut self) -> Self {
        self.seekable = false;
        self
    }

    pub(super) fn panicking_listing(mut self) -> Self {
        self.panic_on_list = true;
        self
    }

    fn with(mut self, entry: FakeEntry) -> Self {
        self.entries.push(entry);
        self
    }

    pub(super) fn dir(self, rel: &str) -> Self {
        let mut entry = FakeEntry::file(rel, Vec::new());
        entry.is_dir = true;
        self.with(entry)
    }

    pub(super) fn file(self, rel: &str, bytes: Vec<u8>, mtime_ms: i64) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.mtime_ms = mtime_ms;
        self.with(entry)
    }

    /// A provider export: its size is only known after reading it.
    pub(super) fn export(self, rel: &str, bytes: Vec<u8>) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.size_known = false;
        self.with(entry)
    }

    /// Every connection breaks at offset `after`.
    pub(super) fn failing(self, rel: &str, bytes: Vec<u8>, after: usize) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.fail_after = Some(after);
        self.with(entry)
    }

    /// The first connections break at the given offsets, one each.
    pub(super) fn failing_opens(self, rel: &str, bytes: Vec<u8>, offsets: &[usize]) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.fail_opens = offsets.to_vec();
        self.with(entry)
    }

    /// Reading it panics.
    pub(super) fn panicking(self, rel: &str, bytes: Vec<u8>) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.panic_on_read = true;
        self.with(entry)
    }

    /// Its reader waits until `other` was opened.
    pub(super) fn gated(self, rel: &str, bytes: Vec<u8>, other: &str) -> Self {
        let mut entry = FakeEntry::file(rel, bytes);
        entry.wait_for = Some(other.to_string());
        self.with(entry)
    }

    /// An entry the listing could not read.
    pub(super) fn problem(mut self, path: &str, message: &str) -> Self {
        self.problems.push((path.to_string(), message.to_string()));
        self
    }

    pub(super) fn path(rel: &str) -> String {
        format!("/fake/{rel}")
    }

    fn reader(&self, entry: &ListedEntry, offset: u64) -> io::Result<Box<dyn Read + Send>> {
        let fake = self
            .entries
            .iter()
            .find(|fake| Self::path(&fake.rel) == entry.path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unbekannte Testdatei"))?;
        let earlier = self.stats.open(&fake.rel, offset);
        let limit = fake
            .fail_after
            .or_else(|| fake.fail_opens.get(earlier).copied())
            .unwrap_or(usize::MAX);
        Ok(Box::new(FakeReader {
            entry: fake.clone(),
            position: usize::try_from(offset).unwrap_or(usize::MAX),
            limit,
            stats: self.stats.clone(),
        }))
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
        assert!(!self.panic_on_list, "Testpanik beim Auflisten");
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

    fn open_entry_at(&self, entry: &ListedEntry, offset: u64) -> io::Result<Box<dyn Read + Send>> {
        if self.seekable {
            self.reader(entry, offset)
        } else {
            skip_to(self.reader(entry, 0)?, offset)
        }
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
    limit: usize,
    stats: Arc<Stats>,
}

impl Read for FakeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if let Some(other) = self.entry.wait_for.take() {
            self.stats.wait_opened(&other);
        }
        assert!(!self.entry.panic_on_read, "Testpanik beim Lesen");
        if self.position >= self.limit {
            self.stats
                .failed
                .lock()
                .unwrap()
                .insert(self.entry.rel.clone());
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "Verbindung getrennt",
            ));
        }
        let length = self.entry.bytes.len();
        let start = self.position.min(length);
        let end = length.min(self.limit).min(start + out.len());
        let count = end - start;
        out[..count].copy_from_slice(&self.entry.bytes[start..end]);
        self.position = end.max(self.position);
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
