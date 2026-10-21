//! A large file transferred in parts under the flow's permits. The worker
//! that took the file owns it: it transfers parts itself and, while the flow
//! has a free permit, queues tickets (`Work::Segment`) that other workers
//! answer by taking parts of the same file (`help`). Each part is one
//! operation like a file: memory first, then a permit, bytes through a
//! `Meter`, the permit returned with the result. The owner never waits for a
//! part no worker has begun, so one permit is enough: the owner then
//! transfers all parts one after the other. Source and target are behind
//! `SegmentIo`; this module knows neither.
use super::super::flow::{acquire_pair, classify_error};
use super::super::flow_control::OpOutcome;
use super::super::memory::reserve_memory;
use super::super::segments::{Ledger, SegmentSizer};
use super::ops::{self, Meter, OpError};
use super::queue::Work;
use super::{lock, Engine};
use crate::vfs::Backend;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The owner looks at the state of running parts this often.
const WAIT_SLICE: Duration = Duration::from_millis(100);
/// What is known of a connection is forgotten this long after its last use
/// (like the flows); a refusal of ranges holds for as long.
const CONNECTION_MEMORY: Duration = Duration::from_secs(15 * 60);

/// How long a part took: until its first byte, and in all.
pub(super) struct SegmentTiming {
    pub first_byte_ms: u64,
    pub total_ms: u64,
}

pub(super) enum SegmentError {
    /// Only before the first byte of the first part: this way cannot read
    /// ranges, the file goes as one stream.
    Fallback,
    Failed(OpError),
}

/// Moves one part between the endpoints.
pub(super) trait SegmentIo: Send + Sync {
    /// Transfers exactly `[start, start + len)`. `buffer` is the worker's
    /// 256 KiB buffer; `meter` counts bytes for progress and flow; `stop` is
    /// looked at between blocks.
    fn run(
        &self,
        start: u64,
        len: u64,
        buffer: &mut [u8],
        meter: &Meter<'_>,
        stop: &AtomicBool,
    ) -> Result<SegmentTiming, SegmentError>;
}

/// One file in parts: shared by its owner and the helpers.
pub(super) struct Shared<'e> {
    id: AtomicU64,
    name: String,
    ledger: Mutex<Ledger>,
    sizer: Arc<Mutex<SegmentSizer>>,
    io: Box<dyn SegmentIo + 'e>,
    abort: Arc<AtomicBool>,
    first_error: Mutex<Option<OpError>>,
    fallback: AtomicBool,
    /// Bytes helpers reported through their own meters.
    moved: AtomicU64,
    changed: Condvar,
    ticket_pending: AtomicBool,
}

impl<'e> Shared<'e> {
    pub(super) fn new(
        name: &str,
        ledger: Ledger,
        sizer: Arc<Mutex<SegmentSizer>>,
        io: Box<dyn SegmentIo + 'e>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: AtomicU64::new(0),
            name: name.to_string(),
            ledger: Mutex::new(ledger),
            sizer,
            io,
            abort: Arc::new(AtomicBool::new(false)),
            first_error: Mutex::new(None),
            fallback: AtomicBool::new(false),
            moved: AtomicU64::new(0),
            changed: Condvar::new(),
            ticket_pending: AtomicBool::new(false),
        })
    }

    /// The safe start: every byte before it is written.
    pub(super) fn prefix(&self) -> u64 {
        lock(&self.ledger).prefix()
    }

    /// Bytes the helpers moved.
    pub(super) fn moved(&self) -> u64 {
        self.moved.load(Ordering::Acquire)
    }

    fn aborted(&self) -> bool {
        self.abort.load(Ordering::Acquire)
    }

    fn has_unclaimed(&self) -> bool {
        lock(&self.ledger).has_unclaimed()
    }

    /// The next part, measured by what this connection showed so far.
    fn claim(&self) -> Option<(u64, u64)> {
        if self.aborted() {
            return None;
        }
        let sizer = lock(&self.sizer);
        lock(&self.ledger).claim(&sizer)
    }

    /// Ends the attempt with `error` (the first one stays).
    pub(super) fn fail(&self, error: OpError) {
        lock(&self.first_error).get_or_insert(error);
        self.stop_all();
    }

    fn stop_all(&self) {
        self.abort.store(true, Ordering::Release);
        self.changed.notify_all();
    }

    /// Runs one claimed part and books the result. `None`: the part was cut
    /// off (abort, fallback) and says nothing about the connection.
    fn run_range(
        &self,
        start: u64,
        len: u64,
        buffer: &mut [u8],
        meter: &Meter<'_>,
    ) -> Option<OpOutcome> {
        let outcome = match self.io.run(start, len, buffer, meter, &self.abort) {
            Ok(timing) => {
                lock(&self.sizer).record(len, timing.first_byte_ms, timing.total_ms);
                lock(&self.ledger).complete(start);
                self.changed.notify_all();
                return Some(OpOutcome::Done);
            }
            Err(SegmentError::Fallback) => {
                self.fallback.store(true, Ordering::Release);
                None
            }
            // A part cut off by the abort of another part or by the stop of
            // the job is no failure of its own; the reason is recorded once.
            Err(SegmentError::Failed(error))
                if error.error.kind() == io::ErrorKind::Interrupted =>
            {
                None
            }
            Err(SegmentError::Failed(error)) => {
                let outcome = classify_error(&error.error);
                lock(&self.first_error).get_or_insert(error);
                Some(outcome)
            }
        };
        self.abort.store(true, Ordering::Release);
        lock(&self.ledger).release_failed(start);
        self.changed.notify_all();
        outcome
    }

    /// Waits until no part runs, calling `on_progress` every slice.
    fn wait_until_idle(&self, engine: &Engine<'_>, on_progress: &mut dyn FnMut()) {
        loop {
            let guard = lock(&self.ledger);
            if guard.in_flight_empty() {
                return;
            }
            if engine.stopped() {
                drop(guard);
                self.fail(OpError::canceled());
                continue;
            }
            let waited = self.changed.wait_timeout(guard, WAIT_SLICE);
            drop(waited.unwrap_or_else(|poisoned| poisoned.into_inner()));
            on_progress();
        }
    }
}

/// The files of one engine run that are transferred in parts, by id.
pub(super) struct Registry<'e> {
    jobs: Mutex<HashMap<u64, Arc<Shared<'e>>>>,
    next: AtomicU64,
}

/// An entry of the registry; the file leaves it when this is dropped (also
/// when the owner fails), so late tickets find nothing.
pub(super) struct Registered<'r, 'e> {
    registry: &'r Registry<'e>,
    id: u64,
}

impl Drop for Registered<'_, '_> {
    fn drop(&mut self) {
        lock(&self.registry.jobs).remove(&self.id);
    }
}

impl<'e> Registry<'e> {
    pub(super) fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }

    pub(super) fn insert(&self, shared: Arc<Shared<'e>>) -> Registered<'_, 'e> {
        let id = self.next.fetch_add(1, Ordering::AcqRel);
        shared.id.store(id, Ordering::Release);
        lock(&self.jobs).insert(id, shared);
        Registered { registry: self, id }
    }

    fn get(&self, id: u64) -> Option<Arc<Shared<'e>>> {
        lock(&self.jobs).get(&id).cloned()
    }

    /// The job stops: every part in the air ends at its next block, every
    /// helper waiting for memory or a permit gives up.
    pub(super) fn abort_all(&self) {
        for shared in lock(&self.jobs).values() {
            shared.stop_all();
        }
    }
}

/// Queues one ticket for a helper when the file has parts left, the flow has
/// a free permit and no ticket of this file is waiting: the helpers grow with
/// the permits the flow hands out.
fn offer_ticket(engine: &Engine<'_>, shared: &Shared<'_>) {
    if shared.aborted() || !shared.has_unclaimed() || !engine.flows_have_spare() {
        return;
    }
    if shared.ticket_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let job = shared.id.load(Ordering::Acquire);
    engine.queue.push_front(Work::Segment { job });
    engine.queue.notify();
}

/// The owner: transfers parts under the permit its worker holds, until none
/// is left, then waits for the parts other workers began. `Ok` when every
/// part is written. `on_progress` runs after each part and each wait slice.
pub(super) fn drive(
    engine: &Engine<'_>,
    shared: &Arc<Shared<'_>>,
    buffer: &mut [u8],
    meter: &Meter<'_>,
    on_progress: &mut dyn FnMut(),
) -> Result<(), SegmentError> {
    loop {
        if engine.stopped() {
            shared.fail(OpError::canceled());
        }
        if !shared.aborted() {
            offer_ticket(engine, shared);
            if let Some((start, len)) = shared.claim() {
                shared.run_range(start, len, buffer, meter);
                on_progress();
                continue;
            }
        }
        break;
    }
    shared.wait_until_idle(engine, on_progress);
    on_progress();
    shared.stop_all();
    if let Some(error) = lock(&shared.first_error).take() {
        return Err(SegmentError::Failed(error));
    }
    if shared.fallback.load(Ordering::Acquire) {
        return Err(SegmentError::Fallback);
    }
    if lock(&shared.ledger).all_done() {
        return Ok(());
    }
    Err(SegmentError::Failed(OpError::canceled()))
}

/// A helper: answers a ticket by taking parts of file `job` as long as the
/// file has parts left and the flow gives permits. Memory first, then the
/// permit (never the other way round), and nothing is held while waiting for
/// either besides what the order allows.
pub(super) fn help(engine: &Engine<'_>, job: u64, buffer: &mut [u8]) {
    let Some(abort) = engine.segments.get(job).map(|shared| {
        shared.ticket_pending.store(false, Ordering::Release);
        shared.abort.clone()
    }) else {
        return;
    };
    loop {
        if engine.stopped() || abort.load(Ordering::Acquire) {
            return;
        }
        // Not holding the file while waiting: the target must be free to be
        // closed and renamed the moment its owner is done.
        if !engine
            .segments
            .get(job)
            .is_some_and(|shared| shared.has_unclaimed())
        {
            return;
        }
        let Some(_reservation) = reserve_memory(ops::reservation(engine), &abort) else {
            return;
        };
        trace("memory");
        let Some(permits) = acquire_pair(
            &engine.op_flows.0,
            engine.op_flows.1.as_ref(),
            engine.job_id,
            &abort,
        ) else {
            return;
        };
        trace("permit");
        let claimed = engine
            .segments
            .get(job)
            .and_then(|shared| shared.claim().map(|part| (shared, part)));
        let Some((shared, (start, len))) = claimed else {
            permits.finish(OpOutcome::Failed);
            return;
        };
        offer_ticket(engine, &shared);
        let active = engine.stats.begin(&shared.name);
        engine.queue.notify();
        let meter = Meter::new(&permits, &engine.stats);
        let outcome = shared.run_range(start, len, buffer, &meter);
        shared.moved.fetch_add(meter.moved(), Ordering::AcqRel);
        permits.finish(outcome.unwrap_or(OpOutcome::Failed));
        trace("done");
        drop(active);
    }
}

#[cfg(test)]
pub(super) static TRACE: Mutex<Vec<(std::thread::ThreadId, &'static str)>> = Mutex::new(Vec::new());

/// Tests record the order of memory, permit and release of every helper.
fn trace(event: &'static str) {
    #[cfg(test)]
    lock(&TRACE).push((std::thread::current().id(), event));
    #[cfg(not(test))]
    let _ = event;
}

struct Connection {
    sizer: Arc<Mutex<SegmentSizer>>,
    no_ranges_until: Option<Instant>,
    used: Instant,
}

fn connections() -> &'static Mutex<HashMap<String, Connection>> {
    static CONNECTIONS: OnceLock<Mutex<HashMap<String, Connection>>> = OnceLock::new();
    CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_connection<R>(key: &str, f: impl FnOnce(&mut Connection) -> R) -> R {
    let mut known = lock(connections());
    let now = Instant::now();
    known.retain(|_, connection| now.duration_since(connection.used) < CONNECTION_MEMORY);
    let connection = known.entry(key.to_string()).or_insert_with(|| Connection {
        sizer: Arc::new(Mutex::new(SegmentSizer::default())),
        no_ranges_until: None,
        used: now,
    });
    connection.used = now;
    f(connection)
}

/// The part sizer of the connection `key` names (`Backend::flow_identity`):
/// what one file learned about set-up time and stream rate helps the next.
pub(super) fn sizer_of(key: &str) -> Arc<Mutex<SegmentSizer>> {
    with_connection(key, |connection| connection.sizer.clone())
}

/// Whether this connection told us (within 15 minutes) that it cannot read
/// ranges, so the file is not even tried in parts.
pub(super) fn no_ranges(backend: &dyn Backend, path: &str) -> bool {
    with_connection(&backend.flow_identity(path), |connection| {
        connection
            .no_ranges_until
            .is_some_and(|until| until > Instant::now())
    })
}

/// Remembers that this connection cannot read ranges.
pub(super) fn mark_no_ranges(backend: &dyn Backend, path: &str) {
    with_connection(&backend.flow_identity(path), |connection| {
        connection.no_ranges_until = Some(Instant::now() + CONNECTION_MEMORY);
    });
}
