//! The end of the one picture operation, awaited with the session's waker.
//! `poll` looks at the operation with a no-op waker (futures-util
//! `noop_waker_ref`: a waker that "does nothing when `wake()` is called"),
//! so its end alone never wakes the session; `finished` lets the session
//! wait for it when a capture tick found the operation still running.
use super::{PipelineActor, WorkCompletion};
use crate::screen::core::host_pipeline::{Pipeline, PipelineError, Produced};
use std::io;

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

    /// Picture operations started so far.
    pub(crate) fn work_started(&self) -> u64 {
        self.started
    }

    /// Whether a picture operation runs.
    pub(crate) fn working(&self) -> bool {
        self.job.is_some()
    }

    /// Resolves once no picture operation runs any more and the result of
    /// the last one is taken in. Cancel-safe: the `JoinHandle` stays in
    /// place until it is ready (tokio 1.52.3 `JoinHandle::poll` stores the
    /// waker "and is notified once the task does complete").
    pub(crate) async fn finished(&mut self) {
        if let Some(job) = self.job.as_mut() {
            let output = job.await;
            self.job = None;
            self.settle(output);
        }
        self.poll();
    }
}
