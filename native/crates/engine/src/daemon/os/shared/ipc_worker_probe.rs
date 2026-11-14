//! The outcome of one probe of the running worker, and the two pure readings
//! of it: the state the callers of `probe_worker` act on, and whether a worker
//! answers at all (M30, E55). The handshake that produces the outcome is
//! `probe_outcome` in `ipc_client.rs`; this file does no I/O.

use super::WorkerProbe;

#[cfg(test)]
#[path = "update_worker_task_tests.rs"]
mod update_worker_task_tests;

/// What one probe of the worker's IPC learned, before a caller names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProbeOutcome {
    /// A `Pong` of this version that serves requests.
    Ready { generation: String },
    /// A `Pong` of this version that is still initializing.
    Starting { generation: String },
    /// A `Pong` of this version that is being replaced.
    Retiring,
    /// A `Pong` of another version, or one without a valid generation.
    OtherPong,
    /// The process at the address did not prove that it holds the token.
    Unproven,
    /// No connection, or no answer to the proof in time.
    Unreachable,
    /// No token, no address, a failed `Ping`, or a reply that is not a `Pong`.
    Missing,
}

/// The state `probe_worker` reports to its callers.
pub(super) fn to_worker_probe(outcome: ProbeOutcome) -> WorkerProbe {
    match outcome {
        ProbeOutcome::Ready { generation } => WorkerProbe::Ready { generation },
        ProbeOutcome::Starting { generation } => WorkerProbe::Starting { generation },
        ProbeOutcome::Retiring => WorkerProbe::Retiring,
        ProbeOutcome::OtherPong | ProbeOutcome::Unproven => WorkerProbe::Stale,
        ProbeOutcome::Unreachable | ProbeOutcome::Missing => WorkerProbe::Missing,
    }
}

/// Whether a worker answers: a `Pong` after a passed proof, of any version (E55).
pub(super) fn answers(outcome: &ProbeOutcome) -> bool {
    match outcome {
        ProbeOutcome::Ready { .. }
        | ProbeOutcome::Starting { .. }
        | ProbeOutcome::Retiring
        | ProbeOutcome::OtherPong => true,
        ProbeOutcome::Unproven | ProbeOutcome::Unreachable | ProbeOutcome::Missing => false,
    }
}
