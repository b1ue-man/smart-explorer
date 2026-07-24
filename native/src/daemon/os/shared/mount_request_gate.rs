use std::io;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const METADATA_GATE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_METADATA_PRIORITY_BURST: usize = 8;

/// Bounds the mount host's in-flight requests before they enter the agent
/// protocol. A permit stays attached to streamed readers/writers until their
/// request is closed, so the daemon never has to reject request number N+1.
pub(super) struct MountRequestGate {
    limit: usize,
    state: Mutex<GateState>,
    wake: Condvar,
}

#[derive(Default)]
struct GateState {
    active: usize,
    transfer_waiters: usize,
    metadata_waiters: usize,
    metadata_burst: usize,
}

impl MountRequestGate {
    pub(super) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: limit.clamp(1, 8),
            state: Mutex::new(GateState::default()),
            wake: Condvar::new(),
        })
    }

    pub(super) fn enter(self: &Arc<Self>) -> io::Result<MountRequestPermit> {
        let mut state = self.state.lock().map_err(|_| {
            io::Error::other("mounted-drive backend concurrency state is unavailable")
        })?;
        state.transfer_waiters = state.transfer_waiters.saturating_add(1);
        while !transfer_can_enter(&state, self.limit) {
            state = self.wake.wait(state).map_err(|_| {
                io::Error::other("mounted-drive backend concurrency state is unavailable")
            })?;
        }
        state.transfer_waiters = state.transfer_waiters.saturating_sub(1);
        state.active += 1;
        state.metadata_burst = 0;
        self.wake.notify_all();
        Ok(MountRequestPermit {
            gate: Arc::clone(self),
        })
    }

    /// Metadata may sit behind a long-lived streamed reader when a backend
    /// advertises only one in-flight request. Prefer queued Explorer metadata
    /// over new transfers and fail it within a small absolute budget instead
    /// of letting Dokany accumulate callbacks for minutes.
    pub(super) fn enter_metadata(self: &Arc<Self>) -> io::Result<MountRequestPermit> {
        self.enter_metadata_until(Instant::now() + METADATA_GATE_TIMEOUT)
    }

    fn enter_metadata_until(self: &Arc<Self>, deadline: Instant) -> io::Result<MountRequestPermit> {
        let mut state = self.state.lock().map_err(|_| {
            io::Error::other("mounted-drive backend concurrency state is unavailable")
        })?;
        state.metadata_waiters = state.metadata_waiters.saturating_add(1);
        loop {
            if metadata_can_enter(&state, self.limit) {
                state.metadata_waiters = state.metadata_waiters.saturating_sub(1);
                state.active += 1;
                state.metadata_burst = state.metadata_burst.saturating_add(1);
                self.wake.notify_all();
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                state.metadata_waiters = state.metadata_waiters.saturating_sub(1);
                self.wake.notify_all();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "mounted-drive metadata waited too long for the remote backend",
                ));
            }
            let waited = self.wake.wait_timeout(state, deadline - now).map_err(|_| {
                io::Error::other("mounted-drive backend concurrency state is unavailable")
            })?;
            state = waited.0;
            if waited.1.timed_out() && state.active >= self.limit {
                state.metadata_waiters = state.metadata_waiters.saturating_sub(1);
                self.wake.notify_all();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "mounted-drive metadata waited too long for the remote backend",
                ));
            }
        }
        Ok(MountRequestPermit {
            gate: Arc::clone(self),
        })
    }
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
            Err(poisoned) => poisoned.into_inner(),
        };
        state.active = state.active.saturating_sub(1);
        self.gate.wake.notify_all();
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
        };
        assert!(transfer_can_enter(&state, 1));
        assert!(!metadata_can_enter(&state, 1));
    }
}
