//! How many requests of one file a pipelined transfer keeps on the wire.
//!
//! Each request moves one chunk. Bandwidth × round trip decides how many
//! chunks must be in flight to fill the link, and neither is known up front,
//! so the depth grows from two by one per answered full chunk (doubling per
//! round trip, like a slow start) while answers still come about as fast as
//! the fastest one. Once answers take longer than twice the fastest one plus
//! the timer resolution, a queue as long as the unloaded round trip has
//! formed: in flight is about twice the bandwidth-delay product (BBR's cwnd
//! gain of 2), more would only delay other requests on the connection and a
//! cancel. A slow link therefore stays at a few chunks, a long fat one grows
//! up to `cap`. Every chunk beyond the first is read-ahead that sits in memory
//! until the caller takes it, so it is reserved from the transfer memory
//! budget before the depth grows (plan K7); without budget the depth stays.
//! Used by the SFTP channel pool and the SMB reader.
use crate::transfer::{try_reserve_memory, MemoryReservation};
use std::time::Duration;

/// Answers within this factor of the fastest one mean no queue has formed.
const QUEUE_FACTOR: u32 = 2;
/// Tokio's timer wheel resolves 1 ms; smaller latency differences are noise.
const TIMER_RESOLUTION: Duration = Duration::from_millis(1);
/// Two from the start: a small file's last chunk and the answer after it
/// (the SFTP end-of-file status) come back in one round trip.
const START_DEPTH: usize = 2;

pub(crate) struct Pipeline {
    chunk: u64,
    depth: usize,
    cap: usize,
    /// Requests beyond the first that the grants below pay for.
    reserved: usize,
    grants: Vec<MemoryReservation>,
    fastest: Option<Duration>,
}

impl Pipeline {
    /// `chunk` bytes per request, never more than `cap` requests at once.
    pub(crate) fn new(chunk: u64, cap: usize) -> Self {
        let mut pipeline = Self {
            chunk: chunk.max(1),
            depth: 1,
            cap: cap.max(1),
            reserved: 0,
            grants: Vec::new(),
            fastest: None,
        };
        pipeline.grow_to(START_DEPTH);
        pipeline
    }

    /// Requests that may be in flight now.
    pub(crate) fn depth(&self) -> usize {
        self.depth
    }

    /// One request was answered `latency` after it was sent; `full` when it
    /// moved a whole chunk (a short tail says nothing about the link).
    pub(crate) fn answered(&mut self, latency: Duration, full: bool) {
        let fastest = match self.fastest {
            Some(fastest) if fastest <= latency => fastest,
            _ => {
                self.fastest = Some(latency);
                latency
            }
        };
        let unqueued = fastest
            .saturating_mul(QUEUE_FACTOR)
            .saturating_add(TIMER_RESOLUTION);
        if full && latency <= unqueued {
            self.grow_to(self.depth + 1);
        }
    }

    fn grow_to(&mut self, target: usize) {
        let target = target.min(self.cap);
        if target <= self.depth {
            return;
        }
        let needed = target - 1;
        if needed > self.reserved {
            // Doubling grants: a long download takes a handful, not one per
            // chunk. `needed <= cap - 1`, so the room never underflows.
            let room = self.cap - 1 - self.reserved;
            let extra = (needed - self.reserved).max(self.reserved).min(room);
            let Some(grant) = try_reserve_memory(extra as u64 * self.chunk) else {
                return;
            };
            self.grants.push(grant);
            self.reserved += extra;
        }
        self.depth = target;
    }
}
