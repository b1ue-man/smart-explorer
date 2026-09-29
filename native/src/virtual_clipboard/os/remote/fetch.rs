//! One file's bytes on their way from the connection to Explorer: a bounded
//! buffer filled by a producer thread (`producer.rs`, under the connection's
//! flow and the memory budget) and drained by Explorer's stream reads.
use super::handoff::Held;
use super::signal::{Signal, WAIT_SLICE};
use crate::transfer::ListedEntry;
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use windows::core::HRESULT;
use windows::Win32::Foundation::{E_ABORT, E_UNEXPECTED, STG_E_MEDIUMFULL, STG_E_READFAULT};

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

/// Buffer sizes for one fetch; `reserve` bytes are held from the budget
/// while the fetch buffers anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BufferPlan {
    capacity: usize,
    pub(super) scratch: usize,
    initial: usize,
}

impl BufferPlan {
    pub(super) fn for_entry(entry: &ListedEntry) -> Self {
        Self::from_offset(entry, 0)
    }

    /// For the bytes from `start` to the end.
    pub(super) fn from_offset(entry: &ListedEntry, start: u64) -> Self {
        let rest = usize::try_from(entry.size.saturating_sub(start)).unwrap_or(usize::MAX);
        let (capacity, initial) = if entry.size_known {
            let capacity = rest.clamp(MIN_BUFFER, READ_AHEAD);
            (capacity, capacity.min(rest))
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
}

impl FetchError {
    pub(super) fn from_io(error: &io::Error) -> Self {
        Self {
            code: error
                .raw_os_error()
                .map(|code| HRESULT::from_win32(code as u32))
                .unwrap_or(STG_E_READFAULT),
            message: error.to_string(),
        }
    }

    /// A panic in the producer: no bytes will come, but Explorer gets an
    /// answer instead of waiting forever.
    pub(super) fn internal() -> Self {
        Self {
            code: E_UNEXPECTED,
            message: "Interner Fehler beim Lesen".to_string(),
        }
    }

    fn canceled() -> Self {
        Self {
            code: E_ABORT,
            message: "Übergabe abgebrochen".to_string(),
        }
    }

    fn size_unknown() -> Self {
        Self {
            code: STG_E_MEDIUMFULL,
            message: "Größe unbekannt und zu groß zum Zwischenspeichern".to_string(),
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
    /// File offset of the first byte this fetch delivers.
    pub(super) start: u64,
    pub(super) plan: BufferPlan,
    state: Mutex<Buffered>,
    room: Condvar,
    data: Signal,
    pub(super) cancel: AtomicBool,
}

/// What one take from the buffer yielded.
pub(super) struct Taken {
    pub(super) count: usize,
    /// The file ended; short counts without it mean an error is pending,
    /// delivered by the next take.
    pub(super) ended: bool,
}

impl Fetch {
    pub(super) fn new(
        entry: ListedEntry,
        start: u64,
        plan: BufferPlan,
    ) -> windows::core::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            entry,
            start,
            plan,
            state: Mutex::new(Buffered {
                // Allocated only once the reservation is held (`keep`, K7).
                ring: VecDeque::new(),
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

    fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
        self.room.notify_all();
        self.data.notify();
    }

    /// The fetch ended with an error (bytes before it may still wait).
    pub(super) fn has_error(&self) -> bool {
        self.lock().error.is_some()
    }

    /// Nothing more will come: an error and every byte before it taken.
    pub(super) fn failed(&self) -> bool {
        let state = self.lock();
        state.error.is_some() && state.ring.is_empty()
    }

    #[cfg(test)]
    pub(super) fn buffer_capacity(&self) -> usize {
        self.lock().ring.capacity()
    }

    /// Fills `out` unless the file ends or fails first; waits COM-safely.
    pub(super) fn read(&self, out: &mut [u8]) -> Result<Taken, FetchError> {
        let mut filled = 0;
        self.take(out.len(), |chunk| {
            out[filled..filled + chunk.len()].copy_from_slice(chunk);
            filled += chunk.len();
        })
    }

    /// Discards up to `count` bytes (a forward seek).
    pub(super) fn skip(&self, count: u64) -> Result<Taken, FetchError> {
        let step = usize::try_from(count).unwrap_or(usize::MAX);
        self.take(step, |_| {})
    }

    /// The file's full length, for sizes the listing could not know: waits
    /// until the producer reaches the end, which fits the buffer for exports.
    pub(super) fn total(&self) -> Result<u64, FetchError> {
        loop {
            {
                let state = self.lock();
                if state.done {
                    return Ok(self.start + state.produced);
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

    /// Takes up to `count` bytes. Bytes taken before an error are returned
    /// first; the error comes with the next take.
    fn take(&self, count: usize, mut sink: impl FnMut(&[u8])) -> Result<Taken, FetchError> {
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
            if taken == count || ended {
                return Ok(Taken {
                    count: taken,
                    ended,
                });
            }
            if let Some(error) = error {
                return match taken {
                    0 => Err(error),
                    _ => Ok(Taken {
                        count: taken,
                        ended: false,
                    }),
                };
            }
            if self.cancel.load(Ordering::Acquire) {
                return Err(FetchError::canceled());
            }
            self.data.wait(WAIT_SLICE);
        }
    }

    /// Moves as much of `bytes` into the buffer as fits; returns the count.
    pub(super) fn push(&self, bytes: &[u8]) -> usize {
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
    pub(super) fn wait_for_room(&self) -> bool {
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

    /// Keeps the reservation with the buffer and only now allocates it.
    pub(super) fn keep(&self, held: Held) {
        let mut state = self.lock();
        state._held = Some(held);
        let initial = self.plan.initial;
        state.ring.reserve_exact(initial);
    }

    /// Ends the fetch once: done, or failed with `error`.
    pub(super) fn end(&self, error: Option<FetchError>) {
        {
            let mut state = self.lock();
            if state.done || state.error.is_some() {
                return;
            }
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
        self.0.stop();
    }
}
