use std::io;
use std::sync::{Arc, Condvar, Mutex};

/// Bounds the mount host's in-flight requests before they enter the agent
/// protocol. A permit stays attached to streamed readers/writers until their
/// request is closed, so the daemon never has to reject request number N+1.
pub(super) struct MountRequestGate {
    limit: usize,
    active: Mutex<usize>,
    wake: Condvar,
}

impl MountRequestGate {
    pub(super) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: limit.clamp(1, 8),
            active: Mutex::new(0),
            wake: Condvar::new(),
        })
    }

    pub(super) fn enter(self: &Arc<Self>) -> io::Result<MountRequestPermit> {
        let mut active = self.active.lock().map_err(|_| {
            io::Error::other("mounted-drive backend concurrency state is unavailable")
        })?;
        while *active >= self.limit {
            active = self.wake.wait(active).map_err(|_| {
                io::Error::other("mounted-drive backend concurrency state is unavailable")
            })?;
        }
        *active += 1;
        Ok(MountRequestPermit {
            gate: Arc::clone(self),
        })
    }
}

pub(super) struct MountRequestPermit {
    gate: Arc<MountRequestGate>,
}

impl Drop for MountRequestPermit {
    fn drop(&mut self) {
        let mut active = match self.gate.active.lock() {
            Ok(active) => active,
            Err(poisoned) => poisoned.into_inner(),
        };
        *active = active.saturating_sub(1);
        self.gate.wake.notify_one();
    }
}
