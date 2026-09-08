//! Fixed-interval deadlines in assignment order, with direct ID removal.
use std::collections::HashMap;
use std::io;
use std::time::Instant;

#[derive(Default)]
pub(super) struct ResetSchedule {
    pending: HashMap<u64, Links>,
    first: Option<u64>,
    last: Option<u64>,
    last_deadline: Option<Instant>,
}

struct Links {
    previous: Option<u64>,
    next: Option<u64>,
    deadline: Instant,
}

impl ResetSchedule {
    /// Called only under the supervisor mutex, after registration or a fully
    /// completed reset. A claimed reset is not pending until explicitly rearmed.
    pub(super) fn arm(&mut self, id: u64, now: Instant) -> io::Result<bool> {
        if self.pending.contains_key(&id) {
            return Err(io::Error::other("callback reset is already scheduled"));
        }
        let deadline = now
            .checked_add(super::RESET_INTERVAL)
            .ok_or_else(|| io::Error::other("callback reset deadline is out of range"))?;
        // Instant aims to be monotonic, but documents platform violations.
        // Preserve FIFO deadline order ourselves, including across empty queues.
        let deadline = self.last_deadline.map_or(deadline, |last| last.max(deadline));
        self.pending.try_reserve(1).map_err(|_| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                "callback reset schedule allocation failed",
            )
        })?;
        if let Some(last) = self.last {
            let tail = self.pending.get_mut(&last)
                .ok_or_else(|| io::Error::other("callback reset queue lost its tail"))?;
            tail.next = Some(id);
        }
        let head_changed = self.first.is_none();
        self.pending.insert(id, Links {
            previous: self.last,
            next: None,
            deadline,
        });
        if head_changed {
            self.first = Some(id);
        }
        self.last = Some(id);
        self.last_deadline = Some(deadline);
        Ok(head_changed)
    }

    pub(super) fn front(&self) -> Option<(u64, Instant)> {
        let id = self.first?;
        self.pending.get(&id).map(|links| (id, links.deadline))
    }

    /// Update neighboring IDs directly; capacity rebuilding happens only after
    /// geometric contraction. Removing an already claimed/completed request is
    /// a no-op; no stale deadline ticket remains after completion.
    pub(super) fn remove(&mut self, id: u64) -> bool {
        let Some(links) = self.pending.remove(&id) else {
            return false;
        };
        match links.previous {
            Some(previous) => {
                if let Some(previous) = self.pending.get_mut(&previous) {
                    previous.next = links.next;
                }
            }
            None => self.first = links.next,
        }
        match links.next {
            Some(next) => {
                if let Some(next) = self.pending.get_mut(&next) {
                    next.previous = links.previous;
                }
            }
            None => self.last = links.previous,
        }
        let capacity = self.pending.capacity();
        let live = self.pending.len();
        // Reclaim peak-burst allocation without a rebuild on each unlink.
        // Links store stable IDs, so rehashing cannot invalidate the FIFO.
        if capacity > 32 && capacity / 4 > live {
            self.pending.shrink_to(live.saturating_mul(2));
        }
        links.previous.is_none()
    }
}
