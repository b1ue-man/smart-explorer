//! The producer of one fetch: memory first, then a permit of the
//! connection's flow (K2), then the file opened at the fetch's offset and
//! read into the buffer. It never waits for Explorer while holding the
//! permit, and a panic ends the fetch with an error instead of leaving
//! Explorer waiting.
use super::fetch::{Fetch, FetchError};
use super::handoff::{Handoff, Held};
use crate::transfer::{classify_error, FlowPermit, OpOutcome};
use std::io::{self, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::Arc;

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
        .spawn(move || {
            let run = catch_unwind(AssertUnwindSafe(|| {
                produce(&handoff, &worker, job, grant);
            }));
            if run.is_err() {
                worker.end(Some(FetchError::internal()));
            }
        });
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
            let Some(held) = reserve(handoff, fetch) else {
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
    let mut reader = match handoff.source.open_entry_at(&fetch.entry, fetch.start) {
        Ok(reader) => reader,
        Err(error) => return finish(fetch, permit, Ending::Failed(error)),
    };
    let ending = pump(handoff, fetch, job, &mut *reader, &mut permit);
    // Close the connection's stream before its permit goes back.
    drop(reader);
    finish(fetch, permit, ending);
}

/// Memory for a file Explorer is waiting for: at once when it fits,
/// otherwise after the prefetched files (read only later) gave theirs back.
fn reserve(handoff: &Handoff, fetch: &Fetch) -> Option<Held> {
    let bytes = fetch.plan.reserve();
    if let Some(held) = handoff.memory.try_reserve(bytes) {
        return Some(held);
    }
    let _yielded = handoff.prefetch.yield_memory();
    handoff.memory.reserve(bytes, &fetch.cancel)
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
