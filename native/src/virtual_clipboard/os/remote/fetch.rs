//! One file's bytes on their way from the connection to Explorer: a bounded
//! buffer filled by a producer thread (under the connection's flow and the
//! memory budget) and drained by Explorer's stream reads.
use super::handoff::{Handoff, Held};
use super::signal::{Signal, WAIT_SLICE};
use crate::transfer::{classify_error, FlowPermit, ListedEntry, OpOutcome};
use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use windows::core::HRESULT;
use windows::Win32::Foundation::{E_ABORT, STG_E_MEDIUMFULL, STG_E_READFAULT};

/// Read-ahead per streamed file and the largest file prefetched whole: one
/// bandwidth-delay product at 1 Gbit/s and 130 ms (125 MB/s × 0.13 s). Up to
/// this size a file takes about as long to open as to transfer, so fetching
/// it in parallel ahead of Explorer pays; larger files stream, and this much
/// buffer bridges an equally long pause of Explorer (disk flush, virus scan)
/// without idling the connection. It also holds any Google export (10 MB
/// limit), the only entries whose size is unknown.
const READ_AHEAD: usize = 16 << 20;
/// One page: the smallest buffer, so a file that grew after the listing
/// keeps flowing without over-reserving memory for tiny files.
const MIN_BUFFER: usize = 4 << 10;
/// Bytes per network read: large enough that the lock and wake-up per chunk
/// (microseconds) stay below 1 % at 1 GB/s, small against the read-ahead so
/// the producer resumes as soon as Explorer made room.
const SCRATCH: usize = 256 << 10;

/// Buffer sizes for one entry; `reserve` bytes are held from the budget
/// while the fetch buffers anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BufferPlan {
    capacity: usize,
    scratch: usize,
    initial: usize,
}

impl BufferPlan {
    pub(super) fn for_entry(entry: &ListedEntry) -> Self {
        let size = usize::try_from(entry.size).unwrap_or(usize::MAX);
        let (capacity, initial) = if entry.size_known {
            let capacity = size.clamp(MIN_BUFFER, READ_AHEAD);
            (capacity, capacity.min(size))
        } else {
            (READ_AHEAD, 0)
        };
        Self {
            capacity,
            scratch: SCRATCH.min(capacity),
            initial,
        }
    }

    /// Whole-file prefetch: small files and exports of unknown size.
    pub(super) fn prefetchable(entry: &ListedEntry) -> bool {
        !entry.is_dir && (!entry.size_known || entry.size <= READ_AHEAD as u64)
    }

    pub(super) fn reserve(&self) -> u64 {
        (self.capacity + self.scratch) as u64
    }
}

#[derive(Clone, Debug)]
pub(super) struct FetchError {
    pub(super) code: HRESULT,
    pub(super) message: String,
    /// The connection asked to slow down (retrying later can succeed).
    overload: bool,
}

impl FetchError {
    fn from_io(error: &io::Error) -> Self {
        Self {
            code: error
                .raw_os_error()
                .map(|code| HRESULT::from_win32(code as u32))
                .unwrap_or(STG_E_READFAULT),
            message: error.to_string(),
            overload: classify_error(error) == OpOutcome::Overload,
        }
    }

    fn canceled() -> Self {
        Self {
            code: E_ABORT,
            message: "Übergabe abgebrochen".to_string(),
            overload: false,
        }
    }

    fn size_unknown() -> Self {
        Self {
            code: STG_E_MEDIUMFULL,
            message: "Größe unbekannt und zu groß zum Zwischenspeichern".to_string(),
            overload: false,
        }
    }
}

struct Buffered {
    ring: VecDeque<u8>,
    /// Bytes put into the ring so far.
    produced: u64,
    done: bool,
    error: Option<FetchError>,
    /// The budget reservation backing the buffered bytes.
    _held: Option<Held>,
}

pub(super) struct Fetch {
    pub(super) entry: ListedEntry,
    plan: BufferPlan,
    state: Mutex<Buffered>,
    room: Condvar,
    data: Signal,
    cancel: AtomicBool,
}

impl Fetch {
    pub(super) fn new(entry: ListedEntry, plan: BufferPlan) -> windows::core::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            entry,
            plan,
            state: Mutex::new(Buffered {
                ring: VecDeque::with_capacity(plan.initial),
                produced: 0,
                done: false,
                error: None,
                _held: None,
            }),
            room: Condvar::new(),
            data: Signal::new(false)?,
            cancel: AtomicBool::new(false),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Buffered> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.room.notify_all();
        self.data.notify();
    }

    pub(super) fn failed_with_overload(&self) -> bool {
        matches!(&self.lock().error, Some(error) if error.overload)
    }

    /// Fills `out` unless the file ends first; waits COM-safely for bytes.
    /// Returns the byte count and whether the file is fully read.
    pub(super) fn read(&self, out: &mut [u8]) -> Result<(usize, bool), FetchError> {
        let mut filled = 0;
        self.take(out.len(), |chunk| {
            out[filled..filled + chunk.len()].copy_from_slice(chunk);
            filled += chunk.len();
        })
    }

    /// Discards `count` bytes (a forward seek); fewer at the end of the file.
    pub(super) fn skip(&self, count: u64) -> Result<u64, FetchError> {
        let mut skipped = 0;
        while skipped < count {
            let step = usize::try_from(count - skipped).unwrap_or(usize::MAX);
            let (taken, ended) = self.take(step, |_| {})?;
            skipped += taken as u64;
            if ended || taken < step {
                break;
            }
        }
        Ok(skipped)
    }

    /// The file's full length, for sizes the listing could not know: waits
    /// until the producer reaches the end, which fits the buffer for exports.
    pub(super) fn total(&self) -> Result<u64, FetchError> {
        loop {
            {
                let state = self.lock();
                if state.done {
                    return Ok(state.produced);
                }
                if let Some(error) = &state.error {
                    return Err(error.clone());
                }
                if state.ring.len() >= self.plan.capacity {
                    return Err(FetchError::size_unknown());
                }
            }
            if self.cancel.load(Ordering::Acquire) {
                return Err(FetchError::canceled());
            }
            self.data.wait(WAIT_SLICE);
        }
    }

    fn take(&self, count: usize, mut sink: impl FnMut(&[u8])) -> Result<(usize, bool), FetchError> {
        let mut taken = 0;
        loop {
            let (ended, error) = {
                let mut state = self.lock();
                while taken < count {
                    let (front, _) = state.ring.as_slices();
                    let step = front.len().min(count - taken);
                    if step == 0 {
                        break;
                    }
                    sink(&front[..step]);
                    state.ring.drain(..step);
                    taken += step;
                }
                (state.done && state.ring.is_empty(), state.error.clone())
            };
            if taken > 0 {
                self.room.notify_all();
            }
            if taken == count {
                return Ok((taken, ended));
            }
            if let Some(error) = error {
                return Err(error);
            }
            if ended {
                return Ok((taken, true));
            }
            if self.cancel.load(Ordering::Acquire) {
                return Err(FetchError::canceled());
            }
            self.data.wait(WAIT_SLICE);
        }
    }

    /// Moves as much of `bytes` into the buffer as fits; returns the count.
    fn push(&self, bytes: &[u8]) -> usize {
        let pushed = {
            let mut state = self.lock();
            let room = self.plan.capacity.saturating_sub(state.ring.len());
            let pushed = room.min(bytes.len());
            state.ring.extend(&bytes[..pushed]);
            state.produced += pushed as u64;
            pushed
        };
        if pushed > 0 {
            self.data.notify();
        }
        pushed
    }

    /// Waits until Explorer drained half the buffer (fewer, larger resumes
    /// than refilling byte by byte); false once canceled.
    fn wait_for_room(&self) -> bool {
        let mut state = self.lock();
        loop {
            if self.cancel.load(Ordering::Acquire) {
                return false;
            }
            if state.ring.len() <= self.plan.capacity / 2 {
                return true;
            }
            state = match self.room.wait_timeout(state, WAIT_SLICE) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    fn keep(&self, held: Held) {
        self.lock()._held = Some(held);
    }

    fn end(&self, error: Option<FetchError>) {
        {
            let mut state = self.lock();
            match error {
                Some(error) => state.error = Some(error),
                None => state.done = true,
            }
        }
        self.data.notify();
    }
}

/// Ownership of a fetch by its consumer (a prefetch slot, then a stream).
/// Dropping it stops the producer and frees the buffer.
pub(super) struct FetchHandle(Arc<Fetch>);

impl FetchHandle {
    pub(super) fn new(fetch: Arc<Fetch>) -> Self {
        Self(fetch)
    }

    pub(super) fn shared(&self) -> Arc<Fetch> {
        self.0.clone()
    }

    pub(super) fn reserved(&self) -> u64 {
        self.0.plan.reserve()
    }
}

impl std::ops::Deref for FetchHandle {
    type Target = Fetch;

    fn deref(&self) -> &Fetch {
        &self.0
    }
}

impl Drop for FetchHandle {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Memory and a connection permit obtained before the producer starts.
pub(super) struct Grant {
    pub(super) held: Held,
    pub(super) permit: FlowPermit,
}

/// Starts the producer of `fetch`; without a grant it first waits for memory
/// and a permit itself (in that order, K2).
pub(super) fn spawn_producer(
    handoff: Arc<Handoff>,
    fetch: Arc<Fetch>,
    job: u64,
    grant: Option<Grant>,
) {
    let worker = fetch.clone();
    let spawned = std::thread::Builder::new()
        .name("remote-fetch".into())
        .spawn(move || produce(&handoff, &worker, job, grant));
    if let Err(error) = spawned {
        fetch.end(Some(FetchError::from_io(&error)));
    }
}

enum Ending {
    Done,
    Canceled,
    Failed(io::Error),
}

fn produce(handoff: &Handoff, fetch: &Fetch, job: u64, grant: Option<Grant>) {
    let (held, permit) = match grant {
        Some(Grant { held, permit }) => (held, permit),
        None => {
            let Some(held) = handoff.memory.reserve(fetch.plan.reserve(), &fetch.cancel) else {
                return;
            };
            let Some(permit) = handoff.flow.acquire_for(job, &fetch.cancel) else {
                return;
            };
            (held, permit)
        }
    };
    fetch.keep(held);
    let mut permit = Some(permit);
    if fetch.cancel.load(Ordering::Acquire) {
        // Explorer went past this prefetch before it started.
        return finish(fetch, permit, Ending::Canceled);
    }
    let mut reader = match handoff.source.open_entry(&fetch.entry) {
        Ok(reader) => reader,
        Err(error) => return finish(fetch, permit, Ending::Failed(error)),
    };
    let ending = pump(handoff, fetch, job, &mut *reader, &mut permit);
    // Close the connection's stream before its permit goes back.
    drop(reader);
    finish(fetch, permit, ending);
}

fn pump(
    handoff: &Handoff,
    fetch: &Fetch,
    job: u64,
    reader: &mut dyn Read,
    permit: &mut Option<FlowPermit>,
) -> Ending {
    let mut scratch = vec![0u8; fetch.plan.scratch];
    let (mut start, mut end) = (0, 0);
    loop {
        if fetch.cancel.load(Ordering::Acquire) {
            return Ending::Canceled;
        }
        if start < end {
            start += fetch.push(&scratch[start..end]);
            if start < end {
                // K2: wait for Explorer without holding a connection permit.
                if let Some(permit) = permit.take() {
                    permit.finish(OpOutcome::Done);
                }
                if !fetch.wait_for_room() {
                    return Ending::Canceled;
                }
            }
            continue;
        }
        if permit.is_none() {
            *permit = handoff.flow.acquire_for(job, &fetch.cancel);
            if permit.is_none() {
                return Ending::Canceled;
            }
        }
        // The scratch buffer is read even when the ring is full, so a file
        // that exactly fills it still sees its end and closes at once.
        match reader.read(&mut scratch) {
            Ok(0) => return Ending::Done,
            Ok(read) => {
                // Never trust a reader to stay inside the buffer it was given.
                let read = read.min(scratch.len());
                if let Some(permit) = permit.as_ref() {
                    permit.progress(read as u64);
                }
                (start, end) = (0, read);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Ending::Failed(error),
        }
    }
}

fn finish(fetch: &Fetch, permit: Option<FlowPermit>, ending: Ending) {
    match ending {
        Ending::Done => {
            fetch.end(None);
            if let Some(permit) = permit {
                permit.finish(OpOutcome::Done);
            }
        }
        Ending::Canceled => {
            if let Some(permit) = permit {
                permit.abandon();
            }
        }
        Ending::Failed(error) => {
            if let Some(permit) = permit {
                permit.finish(classify_error(&error));
            }
            fetch.end(Some(FetchError::from_io(&error)));
        }
    }
}
