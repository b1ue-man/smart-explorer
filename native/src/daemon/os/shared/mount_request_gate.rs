use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const METADATA_GATE_TIMEOUT: Duration = Duration::from_secs(10);
/// Transfers queue behind long whole-file fetches on backends that allow one
/// in-flight request. Every waiting callback occupies a Dokany dispatcher
/// thread, so an unbounded wait lets one slow transfer starve the entire
/// drive — including fully cached metadata. A bounded wait fails the queued
/// transfer with a retryable timeout instead and keeps the drive responsive.
/// The budget is short: even bounded waiters hold dispatcher threads, so it
/// must drain a routine queue but never pin the pool for minutes behind one
/// long download.
const TRANSFER_GATE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_METADATA_PRIORITY_BURST: usize = 8;

/// Bounds the mount host's in-flight requests before they enter the agent
/// protocol. A permit stays attached to streamed readers/writers until their
/// request is closed. This counts local admission, not remote worker lifetime:
/// returning a client timeout does not cancel a synchronous backend operation.
pub(super) struct MountRequestGate {
    limit: usize,
    state: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    active: usize,
    transfer_waiters: usize,
    metadata_waiters: usize,
    metadata_burst: usize,
    metadata_served_at: Option<Instant>,
    transfers: WaitQueue,
    metadata: WaitQueue,
}

#[derive(Clone, Copy)]
enum RequestClass {
    Transfer,
    Metadata,
}

const WAITING: u8 = 0;
const GRANTED: u8 = 1;
const CANCELED: u8 = 2;
const ABORTED: u8 = 3;

struct Waiter {
    wake: Condvar,
    // Atomic only for interior mutability through Arc. All reads/writes and
    // predicate checks occur under this waiter's one owning gate mutex.
    status: AtomicU8,
    initial_deadline: Instant,
    progress_timeout: Option<Duration>,
}

impl Waiter {
    fn deadline(&self, served_at: Option<Instant>) -> Instant {
        match (self.progress_timeout, served_at) {
            (Some(timeout), Some(served_at)) => served_at
                .checked_add(timeout)
                .unwrap_or(self.initial_deadline)
                .max(self.initial_deadline),
            _ => self.initial_deadline,
        }
    }

    fn status(&self) -> u8 {
        self.status.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
struct WaitQueue {
    entries: VecDeque<Arc<Waiter>>,
    canceled: usize,
}

impl WaitQueue {
    fn prune_canceled(&mut self) {
        while self.entries.front().is_some_and(|waiter| waiter.status() == CANCELED) {
            self.entries.pop_front();
            self.canceled -= 1;
        }
        // No linear removal per timeout. A full pass occurs only when canceled
        // entries pay for at least half its work; retained tombstones stay below
        // the live count or 32. FIFO order of remaining requests is unchanged.
        if self.canceled >= 32 && self.canceled >= self.entries.len() / 2 {
            self.entries.retain(|waiter| waiter.status() != CANCELED);
            self.canceled = 0;
        }
        if self.entries.is_empty() {
            self.entries = VecDeque::new();
        }
    }

    fn pop(&mut self) -> Option<Arc<Waiter>> {
        self.prune_canceled();
        let waiter = self.entries.pop_front();
        self.prune_canceled();
        waiter
    }
}

impl GateState {
    fn queue(&mut self, class: RequestClass) -> &mut WaitQueue {
        match class {
            RequestClass::Transfer => &mut self.transfers,
            RequestClass::Metadata => &mut self.metadata,
        }
    }

    fn waiter_count(&mut self, class: RequestClass) -> &mut usize {
        match class {
            RequestClass::Transfer => &mut self.transfer_waiters,
            RequestClass::Metadata => &mut self.metadata_waiters,
        }
    }

    fn cancel(&mut self, class: RequestClass, waiter: &Waiter) {
        waiter.status.store(CANCELED, Ordering::Relaxed);
        *self.waiter_count(class) -= 1;
        let queue = self.queue(class);
        queue.canceled += 1;
        queue.prune_canceled();
    }

    fn reserve(&mut self, class: RequestClass, now: Instant) {
        self.active += 1;
        match class {
            RequestClass::Metadata => {
                self.metadata_burst = self.metadata_burst.saturating_add(1);
                self.metadata_served_at = Some(now);
            }
            RequestClass::Transfer => self.metadata_burst = 0,
        }
    }

    fn abort_waiters(&mut self) {
        for queue in [&mut self.transfers, &mut self.metadata] {
            for waiter in queue.entries.drain(..) {
                if waiter.status() == WAITING {
                    waiter.status.store(ABORTED, Ordering::Relaxed);
                    waiter.wake.notify_one();
                }
            }
            queue.canceled = 0;
            queue.entries = VecDeque::new();
        }
        self.transfer_waiters = 0;
        self.metadata_waiters = 0;
    }

    fn dispatch(&mut self, limit: usize) {
        while self.active < limit {
            let class = if self.metadata_waiters > 0 && metadata_can_enter(self, limit) {
                RequestClass::Metadata
            } else if self.transfer_waiters > 0 && transfer_can_enter(self, limit) {
                RequestClass::Transfer
            } else {
                break;
            };
            let Some(waiter) = self.queue(class).pop() else { break };
            *self.waiter_count(class) -= 1;
            let now = Instant::now();
            if now >= waiter.deadline(self.metadata_served_at) {
                waiter.status.store(CANCELED, Ordering::Relaxed);
            } else {
                // Reservation precedes notification: a later caller cannot
                // take this slot while the selected waiter is still waking.
                self.reserve(class, now);
                waiter.status.store(GRANTED, Ordering::Relaxed);
            }
            waiter.wake.notify_one();
        }
    }
}

impl MountRequestGate {
    pub(super) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: limit.clamp(1, 8),
            state: Mutex::new(GateState::default()),
        })
    }

    pub(super) fn enter(self: &Arc<Self>) -> io::Result<MountRequestPermit> {
        self.enter_until(Instant::now() + TRANSFER_GATE_TIMEOUT)
    }

    fn enter_until(self: &Arc<Self>, deadline: Instant) -> io::Result<MountRequestPermit> {
        self.enter_queued(RequestClass::Transfer, deadline, None)
    }

    /// Metadata may sit behind a long-lived streamed reader when a backend
    /// advertises only one in-flight request. Bound lack of metadata service,
    /// not a healthy FIFO backlog's total age. Progress wakes only its selected
    /// waiter; the others reconsider their deadline when their own timer wakes.
    pub(super) fn enter_metadata(self: &Arc<Self>) -> io::Result<MountRequestPermit> {
        self.enter_metadata_until(Instant::now() + METADATA_GATE_TIMEOUT)
    }

    fn enter_metadata_until(self: &Arc<Self>, deadline: Instant) -> io::Result<MountRequestPermit> {
        let timeout = deadline.saturating_duration_since(Instant::now());
        self.enter_queued(RequestClass::Metadata, deadline, Some(timeout))
    }

    fn enter_queued(
        self: &Arc<Self>,
        class: RequestClass,
        deadline: Instant,
        progress_timeout: Option<Duration>,
    ) -> io::Result<MountRequestPermit> {
        let waiter = Waiter {
            wake: Condvar::new(),
            status: AtomicU8::new(WAITING),
            initial_deadline: deadline,
            progress_timeout,
        };
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                poisoned.into_inner().abort_waiters();
                return Err(unavailable());
            }
        };
        state.dispatch(self.limit);
        let now = Instant::now();
        if now >= waiter.deadline(state.metadata_served_at) {
            return Err(queue_timeout(class));
        }
        let can_enter = match class {
            RequestClass::Transfer => state.transfer_waiters == 0
                && transfer_can_enter(&state, self.limit),
            RequestClass::Metadata => state.metadata_waiters == 0
                && metadata_can_enter(&state, self.limit),
        };
        if can_enter {
            // Uncontended calls allocate no queue entry or waiter Arc.
            // Queued predecessors have already reserved any available slots.
            state.reserve(class, now);
            return Ok(MountRequestPermit { gate: Arc::clone(self) });
        }
        let waiter = Arc::new(waiter);
        state.queue(class).entries.push_back(Arc::clone(&waiter));
        *state.waiter_count(class) += 1;
        state.dispatch(self.limit);
        loop {
            match waiter.status() {
                GRANTED => return Ok(MountRequestPermit { gate: Arc::clone(self) }),
                CANCELED => return Err(queue_timeout(class)),
                ABORTED => return Err(unavailable()),
                _ => {}
            }
            let deadline = waiter.deadline(state.metadata_served_at);
            let now = Instant::now();
            if now >= deadline {
                state.cancel(class, &waiter);
                state.dispatch(self.limit);
                return Err(queue_timeout(class));
            }
            state = match waiter.wake.wait_timeout(state, deadline - now) {
                Ok((state, _)) => state,
                Err(poisoned) => {
                    let (mut state, _) = poisoned.into_inner();
                    if waiter.status() == GRANTED {
                        state.active = state.active.saturating_sub(1);
                    }
                    state.abort_waiters();
                    return Err(unavailable());
                }
            };
            // Both status and progress are checked again under the same mutex,
            // including after spurious wakes and races with a timeout/grant.
        }
    }
}

fn unavailable() -> io::Error {
    io::Error::other("mounted-drive backend concurrency state is unavailable")
}

fn queue_timeout(class: RequestClass) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        match class {
            RequestClass::Transfer => "mounted-drive transfer waited too long for the remote backend",
            RequestClass::Metadata => {
                "mounted-drive metadata queue made no service progress for too long"
            }
        },
    )
}

fn transfer_can_enter(state: &GateState, limit: usize) -> bool {
    state.active < limit
        && (state.metadata_waiters == 0 || state.metadata_burst >= MAX_METADATA_PRIORITY_BURST)
}

fn metadata_can_enter(state: &GateState, limit: usize) -> bool {
    state.active < limit
        && (state.transfer_waiters == 0 || state.metadata_burst < MAX_METADATA_PRIORITY_BURST)
}

pub(super) struct MountRequestPermit {
    gate: Arc<MountRequestGate>,
}

impl Drop for MountRequestPermit {
    fn drop(&mut self) {
        let mut state = match self.gate.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.active = state.active.saturating_sub(1);
                state.abort_waiters();
                return;
            }
        };
        state.active = state.active.saturating_sub(1);
        state.dispatch(self.gate.limit);
    }
}

#[cfg(test)]
mod task_tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn remote_drive_task_metadata_waiter_has_priority_over_a_new_transfer() -> io::Result<()> {
        let gate = MountRequestGate::new(1);
        let occupied = gate.enter()?;
        let (send, receive) = mpsc::channel();

        let transfer_gate = Arc::clone(&gate);
        let transfer_send = send.clone();
        let transfer = std::thread::spawn(move || -> io::Result<()> {
            let _permit = transfer_gate.enter()?;
            transfer_send
                .send("transfer")
                .map_err(|_| io::Error::other("test receiver closed"))
        });
        let metadata_gate = Arc::clone(&gate);
        let metadata = std::thread::spawn(move || -> io::Result<()> {
            let _permit = metadata_gate.enter_metadata()?;
            send.send("metadata")
                .map_err(|_| io::Error::other("test receiver closed"))
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let (transfer_waiters, metadata_waiters) = {
                let state = gate
                    .state
                    .lock()
                    .map_err(|_| io::Error::other("test gate poisoned"))?;
                (state.transfer_waiters, state.metadata_waiters)
            };
            if transfer_waiters == 1 && metadata_waiters == 1 {
                break;
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "transfer and metadata waiters did not both queue",
                ));
            }
            std::thread::yield_now();
        }
        drop(occupied);
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(1)).unwrap(),
            "metadata"
        );
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(1)).unwrap(),
            "transfer"
        );
        metadata.join().unwrap()?;
        transfer.join().unwrap()?;
        Ok(())
    }

    #[test]
    fn remote_drive_task_transfer_gate_times_out_instead_of_starving_the_drive() -> io::Result<()> {
        let gate = MountRequestGate::new(1);
        let _occupied = gate.enter()?;
        let started = Instant::now();
        let error = gate
            .enter_until(started + Duration::from_millis(25))
            .err()
            .ok_or_else(|| io::Error::other("transfer wait unexpectedly succeeded"))?;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        // The failed waiter must not leave a stale counter that would block
        // later metadata priority decisions.
        let state = gate
            .state
            .lock()
            .map_err(|_| io::Error::other("test gate poisoned"))?;
        assert_eq!(state.transfer_waiters, 0);
        Ok(())
    }

    #[test]
    fn remote_drive_task_metadata_gate_uses_an_absolute_deadline() -> io::Result<()> {
        let gate = MountRequestGate::new(1);
        let _occupied = gate.enter()?;
        let started = Instant::now();
        let error = gate
            .enter_metadata_until(started + Duration::from_millis(25))
            .err()
            .ok_or_else(|| io::Error::other("metadata wait unexpectedly succeeded"))?;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn remote_drive_task_metadata_priority_is_a_bounded_burst() {
        let state = GateState {
            active: 0,
            transfer_waiters: 1,
            metadata_waiters: 1,
            metadata_burst: MAX_METADATA_PRIORITY_BURST,
            ..GateState::default()
        };
        assert!(transfer_can_enter(&state, 1));
        assert!(!metadata_can_enter(&state, 1));
    }
}
