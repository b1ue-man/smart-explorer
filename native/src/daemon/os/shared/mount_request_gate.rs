use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Metadata requests may overtake queued transfers this many times in a
/// row; then one transfer enters, so neither class starves the other.
const MAX_METADATA_PRIORITY_BURST: usize = 8;

/// Bounds the mount host's in-flight requests before they enter the agent
/// protocol. A permit stays attached to streamed readers/writers until their
/// request is closed. This counts local admission, not remote worker lifetime.
///
/// G4 (2026-10-28): a queued request waits until a slot frees; there is no
/// wall-clock deadline. The limit is the backend's transfer pool capacity
/// (`MountBackendCapabilities::from_backend`, ML8), so queueing here only
/// reflects real remote capacity. A dead connection is not detected by a
/// timer: every request in flight on it fails at the agent connection
/// (socket error or the connection's own liveness check), drops its permit
/// and hands the slot on, so queued requests fail there just as fast. A
/// poisoned gate aborts all waiters.
///
/// Without a deadline, streamed transfers (hydration, write-back, open
/// files) that hold every slot would stall listings and `stat` until one of
/// them ends. Where the backend has more than one slot, transfers therefore
/// never take the last one (`transfer_limit`): it stays for metadata, which
/// only reorders waiters otherwise and cannot pre-empt a running stream.
pub(super) struct MountRequestGate {
    limit: usize,
    state: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    active: usize,
    /// Of `active`, the transfer-class permits.
    transfers_active: usize,
    transfer_waiters: usize,
    metadata_waiters: usize,
    metadata_burst: usize,
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
const ABORTED: u8 = 3;

struct Waiter {
    wake: Condvar,
    // Atomic only for interior mutability through Arc. All reads/writes and
    // predicate checks occur under this waiter's one owning gate mutex.
    status: AtomicU8,
}

impl Waiter {
    fn status(&self) -> u8 {
        self.status.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
struct WaitQueue {
    entries: VecDeque<Arc<Waiter>>,
}

impl WaitQueue {
    fn pop(&mut self) -> Option<Arc<Waiter>> {
        let waiter = self.entries.pop_front();
        if self.entries.is_empty() {
            self.entries = VecDeque::new();
        }
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

    fn reserve(&mut self, class: RequestClass) {
        self.active += 1;
        match class {
            RequestClass::Metadata => {
                self.metadata_burst = self.metadata_burst.saturating_add(1);
            }
            RequestClass::Transfer => {
                self.transfers_active += 1;
                self.metadata_burst = 0;
            }
        }
    }

    fn release(&mut self, class: RequestClass) {
        self.active = self.active.saturating_sub(1);
        if let RequestClass::Transfer = class {
            self.transfers_active = self.transfers_active.saturating_sub(1);
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
            let Some(waiter) = self.queue(class).pop() else {
                break;
            };
            *self.waiter_count(class) -= 1;
            // Reservation precedes notification: a later caller cannot
            // take this slot while the selected waiter is still waking.
            self.reserve(class);
            waiter.status.store(GRANTED, Ordering::Relaxed);
            waiter.wake.notify_one();
        }
    }
}

impl MountRequestGate {
    /// `limit` is the backend's transfer pool capacity as announced in the
    /// mount capabilities (at least one slot).
    pub(super) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: limit.max(1),
            state: Mutex::new(GateState::default()),
        })
    }

    pub(super) fn enter(self: &Arc<Self>) -> io::Result<MountRequestPermit> {
        self.enter_queued(RequestClass::Transfer)
    }

    /// Metadata may sit behind long streamed transfers on a backend with few
    /// slots; it has priority for a bounded burst (`MAX_METADATA_PRIORITY_BURST`).
    pub(super) fn enter_metadata(self: &Arc<Self>) -> io::Result<MountRequestPermit> {
        self.enter_queued(RequestClass::Metadata)
    }

    fn enter_queued(self: &Arc<Self>, class: RequestClass) -> io::Result<MountRequestPermit> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                poisoned.into_inner().abort_waiters();
                return Err(unavailable());
            }
        };
        state.dispatch(self.limit);
        let can_enter = match class {
            RequestClass::Transfer => {
                state.transfer_waiters == 0 && transfer_can_enter(&state, self.limit)
            }
            RequestClass::Metadata => {
                state.metadata_waiters == 0 && metadata_can_enter(&state, self.limit)
            }
        };
        if can_enter {
            // Uncontended calls allocate no queue entry or waiter Arc.
            // Queued predecessors have already reserved any available slots.
            state.reserve(class);
            return Ok(MountRequestPermit {
                gate: Arc::clone(self),
                class,
            });
        }
        let waiter = Arc::new(Waiter {
            wake: Condvar::new(),
            status: AtomicU8::new(WAITING),
        });
        state.queue(class).entries.push_back(Arc::clone(&waiter));
        *state.waiter_count(class) += 1;
        state.dispatch(self.limit);
        loop {
            match waiter.status() {
                GRANTED => {
                    return Ok(MountRequestPermit {
                        gate: Arc::clone(self),
                        class,
                    })
                }
                ABORTED => return Err(unavailable()),
                _ => {}
            }
            state = match waiter.wake.wait(state) {
                Ok(state) => state,
                Err(poisoned) => {
                    let mut state = poisoned.into_inner();
                    if waiter.status() == GRANTED {
                        state.release(class);
                    }
                    state.abort_waiters();
                    return Err(unavailable());
                }
            };
            // The status is checked again under the same mutex, including
            // after spurious wakes.
        }
    }
}

fn unavailable() -> io::Error {
    io::Error::other("mounted-drive backend concurrency state is unavailable")
}

/// The slots transfers may hold at once: all but one, which stays free for
/// metadata; a single-slot backend has none to spare.
fn transfer_limit(limit: usize) -> usize {
    limit.saturating_sub(1).max(1)
}

fn transfer_has_room(state: &GateState, limit: usize) -> bool {
    state.active < limit && state.transfers_active < transfer_limit(limit)
}

fn transfer_can_enter(state: &GateState, limit: usize) -> bool {
    transfer_has_room(state, limit)
        && (state.metadata_waiters == 0 || state.metadata_burst >= MAX_METADATA_PRIORITY_BURST)
}

/// The burst bound only hands a slot to a transfer that can take it.
fn metadata_can_enter(state: &GateState, limit: usize) -> bool {
    state.active < limit
        && (state.transfer_waiters == 0
            || state.metadata_burst < MAX_METADATA_PRIORITY_BURST
            || !transfer_has_room(state, limit))
}

pub(super) struct MountRequestPermit {
    gate: Arc<MountRequestGate>,
    class: RequestClass,
}

impl Drop for MountRequestPermit {
    fn drop(&mut self) {
        let mut state = match self.gate.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.release(self.class);
                state.abort_waiters();
                return;
            }
        };
        state.release(self.class);
        state.dispatch(self.gate.limit);
    }
}

#[cfg(test)]
#[path = "mount_gate_bulk_task_tests.rs"]
mod bulk_task_tests;

#[cfg(test)]
mod task_tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn wait_for_waiters(
        gate: &MountRequestGate,
        transfers: usize,
        metadata: usize,
    ) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let (transfer_waiters, metadata_waiters) = {
                let state = gate
                    .state
                    .lock()
                    .map_err(|_| io::Error::other("test gate poisoned"))?;
                (state.transfer_waiters, state.metadata_waiters)
            };
            if transfer_waiters == transfers && metadata_waiters == metadata {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the expected waiters did not queue",
                ));
            }
            std::thread::yield_now();
        }
    }

    #[test]
    fn mount_vault_task_remote_drive_task_metadata_waiter_has_priority_over_a_new_transfer(
    ) -> io::Result<()> {
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
        wait_for_waiters(&gate, 1, 1)?;
        drop(occupied);
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(5)).unwrap(),
            "metadata"
        );
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(5)).unwrap(),
            "transfer"
        );
        metadata.join().unwrap()?;
        transfer.join().unwrap()?;
        Ok(())
    }

    /// G4: a transfer queued behind a long one is never failed by a clock;
    /// it enters as soon as the slot frees, however long that takes.
    #[test]
    fn speed_task_drive_gate_waits_without_deadline() -> io::Result<()> {
        let gate = MountRequestGate::new(1);
        let occupied = gate.enter()?;
        let waiting_gate = Arc::clone(&gate);
        let queued = std::thread::spawn(move || waiting_gate.enter().map(|_permit| ()));
        wait_for_waiters(&gate, 1, 0)?;
        // Longer than the former 15 s transfer budget is not needed to prove
        // the absence of a deadline: the waiter has no timer at all, so it
        // is still queued after any delay and enters only on release.
        std::thread::sleep(Duration::from_millis(50));
        assert!(!queued.is_finished(), "the queued transfer must still wait");
        drop(occupied);
        queued.join().unwrap()?;
        let state = gate
            .state
            .lock()
            .map_err(|_| io::Error::other("test gate poisoned"))?;
        assert_eq!(state.transfer_waiters, 0);
        assert_eq!(state.active, 0);
        Ok(())
    }

    /// ML8: the gate admits exactly the announced pool capacity at once:
    /// transfers all slots but one, metadata the last.
    #[test]
    fn speed_task_drive_gate_admits_full_pool_capacity() -> io::Result<()> {
        let gate = MountRequestGate::new(24);
        let mut permits = (0..23)
            .map(|_| gate.enter())
            .collect::<io::Result<Vec<_>>>()?;
        let waiting_gate = Arc::clone(&gate);
        let transfer = std::thread::spawn(move || waiting_gate.enter().map(|_permit| ()));
        wait_for_waiters(&gate, 1, 0)?;
        permits.push(gate.enter_metadata()?);
        {
            let state = gate
                .state
                .lock()
                .map_err(|_| io::Error::other("test gate poisoned"))?;
            assert_eq!(state.active, 24);
            assert_eq!(state.transfers_active, 23);
        }
        let waiting_gate = Arc::clone(&gate);
        let queued = std::thread::spawn(move || waiting_gate.enter_metadata().map(|_permit| ()));
        wait_for_waiters(&gate, 1, 1)?;
        drop(permits);
        queued.join().unwrap()?;
        transfer.join().unwrap()?;
        Ok(())
    }

    /// Streams that never end on their own (a hydration of a large file)
    /// hold every transfer slot; a listing still enters at once.
    #[test]
    fn speed_task_drive_gate_keeps_a_slot_for_metadata_behind_streams() -> io::Result<()> {
        let gate = MountRequestGate::new(4);
        let streams = (0..3)
            .map(|_| gate.enter())
            .collect::<io::Result<Vec<_>>>()?;
        let waiting_gate = Arc::clone(&gate);
        let fourth = std::thread::spawn(move || waiting_gate.enter().map(|_permit| ()));
        wait_for_waiters(&gate, 1, 0)?;
        // Not queued behind the streams or the waiting transfer.
        drop(gate.enter_metadata()?);
        drop(gate.enter_metadata()?);
        assert!(!fourth.is_finished(), "the last slot is not a transfer's");
        drop(streams);
        fourth.join().unwrap()?;
        // A single-slot backend has nothing to keep back.
        let single = MountRequestGate::new(1);
        drop(single.enter()?);
        Ok(())
    }

    #[test]
    fn mount_vault_task_remote_drive_task_metadata_priority_is_a_bounded_burst() {
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
