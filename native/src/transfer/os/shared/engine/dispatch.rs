//! The dispatcher of one job: starts a worker while files wait and the flows
//! have a free permit, reports progress every ~150 ms and ends when the queue
//! is drained and the last worker is gone. A thread the process refuses to
//! start is overload, not a file error: the flows halve and learn the number
//! of operations that ran as their bound, and the dispatcher tries again
//! shortly (U44-6 point 7).
use super::{discovery, roots, worker, Engine, MIN_WAIT, PROGRESS_INTERVAL};
use crate::transfer::TransferMsg;
use std::io;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Refused starts in a row, with no worker left, that end the job.
const REFUSALS_THAT_END_THE_JOB: u32 = 3;

/// Tests make the next starts of this thread's dispatcher fail, as a process
/// at its thread limit does.
#[cfg(test)]
thread_local! {
    static REFUSED_STARTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn refuse_next_starts(count: u32) {
    REFUSED_STARTS.with(|refused| refused.set(count));
}

fn start_refused_by_test() -> bool {
    #[cfg(test)]
    {
        REFUSED_STARTS.with(|refused| {
            let left = refused.get();
            refused.set(left.saturating_sub(1));
            left > 0
        })
    }
    #[cfg(not(test))]
    {
        false
    }
}

impl Engine<'_> {
    pub(super) fn run(&self, plan: &roots::RootPlan) {
        std::thread::scope(|scope| {
            let walker = std::thread::Builder::new()
                .name("transfer-discovery".into())
                .spawn_scoped(scope, || discovery::discover(self, plan));
            if walker.is_err() {
                self.fatal(THREADS_REFUSED.to_string());
                self.stats.discovery_finished();
                self.queue.finish_producing();
            }
            let mut last_report: Option<Instant> = None;
            let mut refused = 0u32;
            loop {
                if self.user_cancel.load(Ordering::Acquire) && !self.stop.load(Ordering::Acquire) {
                    self.halt();
                }
                let queued = self.queue.view();
                let drained = queued.queued == 0 || self.stopped();
                if !queued.producing && drained && queued.workers == 0 {
                    break;
                }
                let busy = self.stats.running() >= queued.workers;
                let spawn = queued.queued > 0
                    && queued.idle == 0
                    && !self.stopped()
                    && (queued.workers == 0 || (busy && self.flows_have_spare()));
                if spawn {
                    let slot = self.queue.add_worker();
                    let started = if start_refused_by_test() {
                        drop(slot);
                        Err(io::Error::from(io::ErrorKind::WouldBlock))
                    } else {
                        std::thread::Builder::new()
                            .name("transfer-worker".into())
                            .spawn_scoped(scope, move || worker::run(self, slot))
                            .map(drop)
                    };
                    match started {
                        Ok(()) => refused = 0,
                        Err(_) => {
                            // The slot went with the closure: no worker.
                            refused = self.thread_refused(refused);
                            self.queue.wait_changed(MIN_WAIT);
                        }
                    }
                    continue;
                }
                let due = last_report.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL);
                if due {
                    (self.report)(TransferMsg::Progress(self.progress(queued.queued)));
                    last_report = Some(Instant::now());
                }
                let waited = last_report.map_or(Duration::ZERO, |at| at.elapsed());
                self.queue
                    .wait_changed(PROGRESS_INTERVAL.saturating_sub(waited).max(MIN_WAIT));
            }
        });
    }

    /// A worker could not be started: overload for the job's flows (they
    /// learn the operations that ran as their bound); the job ends when it
    /// cannot start any worker at all. Returns the refusals in a row.
    fn thread_refused(&self, in_a_row: u32) -> u32 {
        let running = self.stats.running();
        self.op_flows.0.learn_ceiling(running);
        if let Some(flow) = &self.op_flows.1 {
            flow.learn_ceiling(running);
        }
        if self.queue.view().workers > 0 {
            return 0;
        }
        let refused = in_a_row + 1;
        if refused >= REFUSALS_THAT_END_THE_JOB {
            self.fatal(THREADS_REFUSED.to_string());
        }
        refused
    }
}

const THREADS_REFUSED: &str = "Der Prozess kann keine weiteren Threads starten (Systemgrenze)";
