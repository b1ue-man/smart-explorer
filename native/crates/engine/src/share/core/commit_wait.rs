//! Waiting for the host's answer to a commit (user rule 2026-11-17: no
//! fixed total time on an operation that may still be working).
//!
//! The host flushes, promotes or syncs before it answers the end of a
//! write (`WriteDone`), of a staged range, a `FinishStage`, a
//! `SyncFilesystem` or the commit of a batch. On a slow medium or a remote
//! export root that takes longer than any fixed limit while nothing moves
//! on the connection; a 60 s limit failed such writes although the host
//! was still writing. The wait therefore has no time limit. It ends with
//! the answer; when the stream or the connection ends (the host closes or
//! resets it, or stops answering QUIC keepalives, so that the transport's
//! idle timeout closes the connection); when the route goes offline
//! (`route_protocol::run_with_route`); or when the transfer this thread
//! works for is cancelled (`source_media::with_cancellation`). A wait that
//! ends without the answer leaves the commit unconfirmed exactly as the
//! former timeout did: callers keep it uncertain and never send it again
//! as if it had not run.
use std::future::Future;
use std::io;
use std::time::{Duration, Instant};

use super::wire::FsRequest;

/// How often a wait looks at the cancellation of its transfer.
const CANCEL_POLL: Duration = Duration::from_millis(200);

/// `future` (the exchange that ends with the host's answer) without a time
/// limit, bound to the connection, the route and the transfer's
/// cancellation. Route pauses hold the future as every Share operation.
pub(super) async fn run_until_answer<T, F>(operation: &str, future: F) -> io::Result<T>
where
    F: Future<Output = io::Result<T>>,
{
    // `Duration::MAX`: tokio's timer turns the overflowing deadline into its
    // far future (tokio 1.52.3, `time::timeout`, `time::sleep`).
    let answer = super::route_protocol::timeout_for(Duration::MAX, future);
    tokio::pin!(answer);
    let mut poll = tokio::time::interval(CANCEL_POLL);
    loop {
        tokio::select! {
            biased;
            result = &mut answer => return result.and_then(|answer| answer),
            _ = poll.tick() => {
                if crate::source_media::context_canceled() {
                    return Err(canceled(operation));
                }
            }
        }
    }
}

fn canceled(operation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        format!("{operation}: abgebrochen; ob der Host abgeschlossen hat, ist offen"),
    )
}

/// Requests whose answer waits for the host's commit work (flush, sync,
/// promotion) and therefore has no time limit (`run_until_answer`).
pub(super) fn waits_for_host_commit(request: &FsRequest) -> bool {
    matches!(
        request,
        FsRequest::WriteDone | FsRequest::FinishStage(_) | FsRequest::SyncFilesystem(_)
    )
}

/// Asking for the outcome of a batch commit whose answer did not arrive
/// (`PutBatchStatus`, K18e). A host that answers "pending" is still
/// committing: every such answer starts the stall window again. Only no
/// such sign of life for a whole window ends the asking, and the outcome
/// then stays unknown; it is never sent again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct OutcomeWait {
    last_sign: Instant,
}

impl OutcomeWait {
    /// One operation deadline without a sign of life (`PEER_OP_TIMEOUT`).
    pub(super) const STALL: Duration = super::io_deadline::PEER_OP_TIMEOUT;

    pub(super) fn new(now: Instant) -> Self {
        Self { last_sign: now }
    }

    /// The host answered "pending" at `now`.
    pub(super) fn pending(&mut self, now: Instant) {
        self.last_sign = now;
    }

    /// Whether to ask again at `now`.
    pub(super) fn ask_again(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_sign) < Self::STALL
    }
}

#[cfg(test)]
#[path = "commit_wait_tests.rs"]
mod tests;
