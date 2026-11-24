//! The end of the one picture operation, awaited with the session's waker.
//! `poll` looks at the operation with a no-op waker (futures-util
//! `noop_waker_ref`: a waker that "does nothing when `wake()` is called"),
//! so its end alone never wakes the session; `finished` lets the session
//! wait for it when a capture tick found the operation still running.
//!
//! Credit when it frees (plan 2026-11-14 „Bildkette“ BK3b, B-27): with a
//! stage that paces itself (the host process) the session waits for the end
//! of each picture operation (`chained`) and starts the next one at once
//! when the last one got the stage's answer or changed state
//! (`begin_chained`), so the host process delivers in its own cadence and
//! the session loop's tick does not round the frame rate. When pacer or
//! sender budget refused the credit, an acknowledgement that frees the
//! queue starts the next one (`credit_freed`); the tick stays the backstop
//! (the sender budget is shared with the peer's other sessions, whose
//! acknowledgements this session does not see).
use super::{PipelineActor, Receipt, WorkCompletion};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_pipeline::{Pipeline, PipelineError, Produced};
use crate::screen::core::wire_control::FrameAck;
use std::io;

/// What the chain of picture operations knows about the last one.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Chain {
    /// The stage paces itself.
    pub self_paced: bool,
    /// The last operation got the stage's answer or changed state.
    pub ready: bool,
    /// An acknowledgement freed the queue since then.
    pub freed: bool,
}

impl Chain {
    /// The next operation may start without a tick: the stage paces itself
    /// and its credit is free (answered, or freed by an acknowledgement).
    pub(super) fn starts(self) -> bool {
        self.self_paced && (self.ready || self.freed)
    }
    /// An acknowledgement freed a refused credit while nothing runs.
    pub(super) fn freed_idle(self, running: bool) -> bool {
        self.self_paced && self.freed && !running
    }
}

impl PipelineActor {
    /// Takes the result of the ended picture operation.
    pub(super) fn settle(
        &mut self,
        output: Result<(Pipeline, WorkCompletion), tokio::task::JoinError>,
    ) {
        match output {
            Ok((pipeline, mut completion)) => {
                let overtaken = !self.work.current(self.job_epoch);
                if overtaken {
                    completion = match completion {
                        WorkCompletion::Tick(_) => WorkCompletion::Tick(Ok(Produced::Reset)),
                        WorkCompletion::Snapshot(_) => WorkCompletion::Snapshot(Err(
                            PipelineError::capture(io::Error::from(io::ErrorKind::Interrupted)),
                        )),
                    };
                }
                self.pipeline = Some(pipeline);
                let cancelled = match &completion {
                    WorkCompletion::Snapshot(Err(error)) => {
                        error.error.kind() == io::ErrorKind::Interrupted
                    }
                    _ => false,
                };
                if cancelled && overtaken && !self.stopped && self.pending.snapshot.is_none() {
                    // An interrupt (a route change, a key-frame request, a new
                    // source or mode) overtook the snapshot the viewer asked
                    // for. It runs again with the next picture operation; it
                    // would otherwise end without a picture and without an
                    // `ERROR`, and the viewer would wait for it for good.
                    self.pending.snapshot = self.job_snapshot.take();
                }
                self.job_snapshot = None;
                if !cancelled {
                    self.completion = Some(completion);
                }
            }
            Err(error) => {
                self.job_snapshot = None;
                self.stopped = true;
                self.blocked = true;
                self.completion = Some(WorkCompletion::Tick(Err(PipelineError::capture(
                    io::Error::other(error),
                ))));
            }
        }
    }

    /// „Höchste Datenrate“ of the viewer (`RATE_CEILING`, plan 2026-11-14
    /// „Bildkette“ B-09): applied by the next `apply`, without an interrupt
    /// of the running picture operation.
    pub fn set_rate_ceiling(&mut self, ceiling: Option<u64>) {
        // A picture being written holds the new ceiling at once (Codex 5/12).
        self.work.set_live_ceiling(ceiling);
        self.pending.ceiling = Some(ceiling);
        self.poll();
    }

    /// `SET_DECODERS` (plan „Bildkette“ P1): the pairing is negotiated
    /// again by the next `apply`, after the running picture operation. Only
    /// a changed pairing interrupts the picture stream there
    /// (`Pipeline::set_decoders`); an unchanged one leaves it alone.
    pub fn set_decoders(&mut self, set: crate::screen::core::set_decoders::SetDecoders) {
        self.pending.decoders = Some(set);
        self.poll();
    }

    pub(super) fn apply_decoders(&mut self, p: &mut Pipeline) {
        if self.pending.decoders.is_none() {
            return;
        }
        let due = self
            .pending
            .decoders_at
            .is_none_or(|at| at.elapsed() >= std::time::Duration::from_millis(250));
        if due {
            if let Some(set) = self.pending.decoders.take() {
                if p.set_decoders(&set) {
                    self.pending.decoders_at = Some(std::time::Instant::now());
                }
            }
        }
    }

    /// `FRAME_ACK`/`PREVIEW_ACK`; one that arrives while a picture
    /// operation runs waits for it with its arrival time.
    pub fn on_ack(&mut self, ack: FrameAck) -> bool {
        self.poll();
        if let Some(p) = self.pipeline.as_mut() {
            let known = p.on_ack(ack);
            if known {
                self.known_ack = Some(ack.frame_id);
                self.facts.chain.freed = true;
            }
            self.refresh();
            known
        } else {
            self.receipt(Receipt::Ack(ack, now_us()));
            false
        }
    }

    pub fn on_presented(&mut self, gop: u64, frame: u64) -> bool {
        self.poll();
        if let Some(p) = self.pipeline.as_mut() {
            let accepted = p.on_presented(gop, frame);
            self.refresh();
            accepted
        } else {
            self.receipt(Receipt::Presented(gop, frame));
            false
        }
    }

    /// The stage paces itself: credit goes when it frees (BK3b).
    pub fn self_paced(&self) -> bool {
        self.facts.chain.self_paced
    }

    /// The session waits for the end of the running operation to start the
    /// next one at once.
    pub fn chained(&self) -> bool {
        self.facts.chain.self_paced && self.job.is_some()
    }

    /// An acknowledgement freed the queue after a refused credit; no
    /// operation runs.
    pub fn credit_freed(&self) -> bool {
        self.facts.chain.freed_idle(self.job.is_some())
    }

    /// The next picture operation of a self-paced stage, when the last one
    /// got its answer or an acknowledgement freed the queue since; else
    /// nothing (the tick or the next acknowledgement starts it).
    pub fn begin_chained(&mut self) -> Result<(), PipelineError> {
        self.poll();
        if !self.facts.chain.starts() {
            return Ok(());
        }
        self.facts.chain.freed = false;
        self.begin_tick()
    }

    /// Picture operations started so far.
    pub fn work_started(&self) -> u64 {
        self.started
    }

    /// Whether a picture operation runs.
    pub fn working(&self) -> bool {
        self.job.is_some()
    }

    /// Resolves once no picture operation runs any more and the result of
    /// the last one is taken in. Cancel-safe: the `JoinHandle` stays in
    /// place until it is ready (tokio 1.52.3 `JoinHandle::poll` stores the
    /// waker "and is notified once the task does complete").
    pub async fn finished(&mut self) {
        if let Some(job) = self.job.as_mut() {
            let output = job.await;
            self.job = None;
            self.settle(output);
        }
        self.poll();
    }
}

#[cfg(test)]
#[path = "bildkette_chain_credit_task_tests.rs"]
mod credit_tests;

/// The pipeline ends off the session's runtime thread, also the one a
/// running picture operation still holds (the actor's drop).
fn dispose(pipeline: Pipeline) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let _ = runtime.spawn_blocking(move || drop(pipeline));
    } else {
        drop(pipeline);
    }
}
impl Drop for PipelineActor {
    fn drop(&mut self) {
        self.work.interrupt();
        if let Some(pipeline) = self.pipeline.take() {
            dispose(pipeline);
        }
        if let Some(job) = self.job.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let _ = runtime.spawn(async move {
                    if let Ok((pipeline, _)) = job.await {
                        dispose(pipeline);
                    }
                });
            }
        }
    }
}
