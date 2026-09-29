//! Shared state of one remote hand-off to Explorer (a clipboard object or a
//! drag): the connection's content, the file list (listed once, on
//! Explorer's first request), prefetching, progress and the async mode.
use super::catalog::{allocate_global, Catalog, DescriptorAlloc};
use super::fetch::{BufferPlan, Fetch, FetchHandle};
use super::prefetch::{Prefetcher, IDLE_RELEASE};
use super::producer::spawn_producer;
use super::session::Sessions;
use super::signal::{Signal, WAIT_SLICE};
use crate::transfer::{Flow, ListedEntry, SelectionListing};
use std::any::Any;
use std::io::{self, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_OUTOFMEMORY, E_UNEXPECTED};

/// Flow job ids of hand-offs start above any transfer number, so the flow
/// serves Explorer's reads in turn with the app's own jobs.
const EXTERNAL_JOBS: u64 = 1 << 48;
const LISTING_FAILED: &str = "Die Auflistung ist unerwartet abgebrochen";

/// What is handed over: a remote selection (`SelectionSource` in the app).
pub(super) trait RemoteContent: Send + Sync {
    fn display_label(&self) -> String;
    fn list_entries(
        &self,
        cancel: &AtomicBool,
        on_found: &(dyn Fn(u64) + Sync),
    ) -> SelectionListing;
    fn open_entry(&self, entry: &ListedEntry) -> io::Result<Box<dyn Read + Send>>;
    /// Reads from byte `offset` on (a broken read continues where it broke).
    fn open_entry_at(&self, entry: &ListedEntry, offset: u64) -> io::Result<Box<dyn Read + Send>>;
    fn connection_flow(&self) -> Arc<Flow>;
}

/// `reader` moved to byte `offset` by reading past the bytes before it, for
/// connections that cannot start mid-file.
pub(super) fn skip_to(
    mut reader: Box<dyn Read + Send>,
    offset: u64,
) -> io::Result<Box<dyn Read + Send>> {
    // A shorter file just ends early; the reader then yields nothing.
    io::copy(&mut reader.by_ref().take(offset), &mut io::sink())?;
    Ok(reader)
}

/// Memory held from a budget until dropped.
pub(super) type Held = Box<dyn Any + Send + Sync>;

pub(super) trait Memory: Send + Sync {
    fn capacity(&self) -> u64;
    /// Waits until `bytes` fit; `None` once `cancel` is set.
    fn reserve(&self, bytes: u64, cancel: &AtomicBool) -> Option<Held>;
    /// Reserves only if `bytes` fit right now.
    fn try_reserve(&self, bytes: u64) -> Option<Held>;
}

/// The process-wide transfer memory budget.
pub(super) struct SharedBudget;

impl Memory for SharedBudget {
    fn capacity(&self) -> u64 {
        crate::transfer::memory_budget()
    }

    fn reserve(&self, bytes: u64, cancel: &AtomicBool) -> Option<Held> {
        crate::transfer::reserve_memory(bytes, cancel).map(|held| Box::new(held) as Held)
    }

    fn try_reserve(&self, bytes: u64) -> Option<Held> {
        crate::transfer::try_reserve_memory(bytes).map(|held| Box::new(held) as Held)
    }
}

pub(super) struct Config {
    pub(super) memory: Arc<dyn Memory>,
    pub(super) alloc: DescriptorAlloc,
    /// How long prefetched files wait for a quiet Explorer.
    pub(super) prefetch_idle: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            memory: Arc::new(SharedBudget),
            alloc: allocate_global,
            prefetch_idle: IDLE_RELEASE,
        }
    }
}

enum Listing {
    Idle,
    Running,
    Ready(Arc<Catalog>),
}

pub(super) struct Handoff {
    pub(super) source: Arc<dyn RemoteContent>,
    pub(super) flow: Arc<Flow>,
    pub(super) memory: Arc<dyn Memory>,
    pub(super) alloc: DescriptorAlloc,
    pub(super) prefetch_job: u64,
    demand_job: u64,
    /// Set when the last object is released: listing, prefetch and fetches stop.
    pub(super) closed: AtomicBool,
    pub(super) prefetch: Prefetcher,
    pub(super) sessions: Sessions,
    listing: Mutex<Listing>,
    listed: Signal,
    asynchronous: AtomicBool,
    operating: AtomicBool,
}

impl Handoff {
    pub(super) fn new(source: Arc<dyn RemoteContent>, config: Config) -> Result<Arc<Self>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Prefetch and Explorer's current file are separate jobs, so the flow
        // alternates between them and the requested file never queues behind
        // the whole prefetch window.
        let prefetch_job = EXTERNAL_JOBS + 2 * NEXT.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(Self {
            flow: source.connection_flow(),
            sessions: Sessions::new(source.display_label()),
            source,
            memory: config.memory,
            alloc: config.alloc,
            prefetch_job,
            demand_job: prefetch_job + 1,
            closed: AtomicBool::new(false),
            prefetch: Prefetcher::new(config.prefetch_idle),
            listing: Mutex::new(Listing::Idle),
            listed: Signal::new(true)?,
            // Explorer may copy in the background (IDataObjectAsyncCapability).
            asynchronous: AtomicBool::new(true),
            operating: AtomicBool::new(false),
        }))
    }

    fn lock_listing(&self) -> MutexGuard<'_, Listing> {
        self.listing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Explorer asks for the file list (a paste or drop begins).
    pub(super) fn begin_paste(&self) {
        if self.sessions.begin() {
            self.prefetch.reset();
        }
    }

    /// The file list, listed once on first use; waits COM-safely meanwhile.
    pub(super) fn catalog(self: &Arc<Self>) -> Result<Arc<Catalog>> {
        loop {
            let start = {
                let mut listing = self.lock_listing();
                if let Listing::Ready(catalog) = &*listing {
                    return Ok(catalog.clone());
                }
                let idle = matches!(*listing, Listing::Idle);
                if idle {
                    *listing = Listing::Running;
                }
                idle
            };
            if start {
                self.start_listing()?;
                continue;
            }
            if self.closed.load(Ordering::Acquire) {
                return Err(Error::new(E_UNEXPECTED, "Die Übergabe wurde beendet"));
            }
            self.listed.wait(WAIT_SLICE);
        }
    }

    fn start_listing(self: &Arc<Self>) -> Result<()> {
        let handoff = self.clone();
        let spawned = std::thread::Builder::new()
            .name("remote-listing".into())
            .spawn(move || handoff.run_listing());
        if let Err(error) = spawned {
            *self.lock_listing() = Listing::Idle;
            return Err(Error::new(
                E_OUTOFMEMORY,
                format!("Auflistung konnte nicht starten: {error}"),
            ));
        }
        Ok(())
    }

    fn run_listing(&self) {
        let listed = catch_unwind(AssertUnwindSafe(|| {
            self.source
                .list_entries(&self.closed, &|found| self.sessions.found(found))
        }));
        // A panic still answers Explorer: with an error instead of a list.
        let listing = listed.unwrap_or_else(|_| SelectionListing {
            problems: vec![(String::new(), LISTING_FAILED.to_string())],
            complete: false,
            ..SelectionListing::default()
        });
        let catalog = Arc::new(Catalog::from_listing(listing));
        self.sessions.listed(&catalog);
        *self.lock_listing() = Listing::Ready(catalog);
        self.listed.notify();
    }

    /// The fetch serving Explorer's request for a file (`None` for folders):
    /// the prefetched one, or a new one started now.
    pub(super) fn open_fetch(
        self: &Arc<Self>,
        catalog: &Arc<Catalog>,
        index: usize,
        entry: &ListedEntry,
    ) -> Result<Option<FetchHandle>> {
        if entry.is_dir {
            return Ok(None);
        }
        let prefetched = self.prefetch.take(index);
        self.prefetch.ensure_running(self, catalog);
        match prefetched {
            Some(handle) => Ok(Some(handle)),
            None => self.demand_fetch(entry, 0).map(Some),
        }
    }

    /// Fetches `entry` from byte `start` on for a waiting reader.
    pub(super) fn demand_fetch(
        self: &Arc<Self>,
        entry: &ListedEntry,
        start: u64,
    ) -> Result<FetchHandle> {
        let plan = BufferPlan::from_offset(entry, start);
        let fetch = Fetch::new(entry.clone(), start, plan)?;
        let handle = FetchHandle::new(fetch.clone());
        spawn_producer(self.clone(), fetch, self.demand_job, None);
        Ok(handle)
    }

    pub(super) fn set_async(&self, enabled: bool) {
        self.asynchronous.store(enabled, Ordering::Release);
    }

    pub(super) fn is_async(&self) -> bool {
        self.asynchronous.load(Ordering::Acquire)
    }

    pub(super) fn operation_started(&self) {
        self.operating.store(true, Ordering::Release);
    }

    pub(super) fn in_operation(&self) -> bool {
        self.operating.load(Ordering::Acquire)
    }

    /// Explorer finished its background copy: nothing more is read in this
    /// paste, so waiting prefetches give their memory back.
    pub(super) fn operation_ended(&self) {
        self.operating.store(false, Ordering::Release);
        self.prefetch.release();
        self.sessions.end();
    }

    /// The last object was released: stop everything still running.
    pub(super) fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.prefetch.close();
        self.listed.notify();
        self.sessions.end();
    }
}
