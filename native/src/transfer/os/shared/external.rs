//! Transfers another program performs from our data, for example Explorer
//! copying virtual files it pasted from our clipboard. They appear in the
//! transfer list next to our own transfers so every copy stays traceable.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Instant;

/// Counters the providing side updates while the other program reads.
pub struct ExternalTransfer {
    id: u64,
    label: String,
    started: Instant,
    files_total: AtomicU64,
    files_done: AtomicU64,
    bytes_done: AtomicU64,
    errors: AtomicU64,
    finished: AtomicBool,
    note: Mutex<Option<String>>,
}

/// A point-in-time copy of an external transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSnapshot {
    pub id: u64,
    pub label: String,
    pub files_total: u64,
    pub files_done: u64,
    pub bytes_done: u64,
    pub errors: u64,
    pub elapsed_ms: u64,
    pub finished: bool,
    pub note: Option<String>,
}

impl ExternalTransfer {
    pub fn set_files_total(&self, total: u64) {
        self.files_total.store(total, Ordering::Release);
    }

    pub fn add_bytes(&self, bytes: u64) {
        self.bytes_done.fetch_add(bytes, Ordering::AcqRel);
    }

    pub fn file_done(&self) {
        self.files_done.fetch_add(1, Ordering::AcqRel);
    }

    pub fn error(&self) {
        self.errors.fetch_add(1, Ordering::AcqRel);
    }

    /// A short explanation shown with the entry (listing progress, entries
    /// the other program cannot receive, the last error).
    pub fn set_note(&self, note: impl Into<String>) {
        *self
            .note
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(note.into());
    }

    pub fn finish(&self) {
        self.finished.store(true, Ordering::Release);
    }

    fn snapshot(&self) -> ExternalSnapshot {
        ExternalSnapshot {
            id: self.id,
            label: self.label.clone(),
            files_total: self.files_total.load(Ordering::Acquire),
            files_done: self.files_done.load(Ordering::Acquire),
            bytes_done: self.bytes_done.load(Ordering::Acquire),
            errors: self.errors.load(Ordering::Acquire),
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            finished: self.finished.load(Ordering::Acquire),
            note: self
                .note
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        }
    }
}

struct Registry {
    next_id: u64,
    live: Vec<Weak<ExternalTransfer>>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            next_id: 1,
            live: Vec::new(),
        })
    })
}

/// Starts tracking an external transfer; it is listed while the returned
/// handle (or a clone) is alive.
pub fn register_external(label: impl Into<String>) -> Arc<ExternalTransfer> {
    let mut registry = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let transfer = Arc::new(ExternalTransfer {
        id: registry.next_id,
        label: label.into(),
        started: Instant::now(),
        files_total: AtomicU64::new(0),
        files_done: AtomicU64::new(0),
        bytes_done: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        finished: AtomicBool::new(false),
        note: Mutex::new(None),
    });
    registry.next_id += 1;
    registry.live.push(Arc::downgrade(&transfer));
    transfer
}

/// Every external transfer still tracked, oldest first.
pub fn external_snapshots() -> Vec<ExternalSnapshot> {
    let mut registry = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.live.retain(|weak| weak.strong_count() > 0);
    registry
        .live
        .iter()
        .filter_map(Weak::upgrade)
        .map(|transfer| transfer.snapshot())
        .collect()
}
