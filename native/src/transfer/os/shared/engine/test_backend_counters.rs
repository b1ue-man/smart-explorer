//! What the fake remote counts and offers beyond its plain reads and writes:
//! the faults of a single read, the requests it saw per kind, and its range
//! reads (split from `test_backend.rs`).
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Mutex;

/// A read that fails: before any byte, or after `after` bytes.
#[derive(Clone, Copy, Debug)]
pub(in super::super) struct ReadFault {
    pub after: Option<usize>,
    pub kind: io::ErrorKind,
}

#[derive(Default)]
pub(in super::super) struct Counters {
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
pub(in super::super) struct Ranges {
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
