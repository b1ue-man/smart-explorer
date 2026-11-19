//! Input of the host process on a thread of its own (plan 2026-11-14
//! „Bildkette“ B-28). A router thread takes every request the dispatcher
//! decoded: `INPUT` and `RELEASE_INPUT` go to the input thread, `STREAM`
//! orders to the picture stream, everything else to the request loop; so
//! input never queues behind a control request, a still image or a
//! picture. The input thread injects with the source while the worker lock
//! is free. While the capture call or a control request holds it, input
//! that needs no source goes at once: through the source's own input
//! endpoint (the portal session on Wayland), and a release through the
//! retained release endpoint or the backend; other input waits only for
//! that capture call, never for scaling, conversion or encoding. Input is
//! answered without the reference envelope (the daemon takes both forms);
//! its events are never logged.
use super::*;
use crate::screen::core::host_backend::ScreenInputSink;
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::worker_wire::BoundInput;
use std::sync::{mpsc, Mutex, PoisonError, TryLockError};
use std::thread::JoinHandle;

enum Job {
    Input(BoundInput),
    Release(Vec<InputEvent>),
}

/// What input needs of the worker without its lock, as the request loop
/// last saw it.
#[derive(Default)]
struct Facts {
    control: bool,
    family: bool,
    /// The source's own input endpoint.
    sink: Option<Arc<dyn ScreenInputSink>>,
    release_sink: Option<Arc<dyn ScreenInputSink>>,
    backend: Option<Arc<dyn ScreenHostBackend>>,
    legacy: bool,
}

/// The input thread of one host process.
pub(super) struct InputLane {
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    facts: Arc<Mutex<Facts>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl InputLane {
    pub(super) fn start(
        worker: Arc<Mutex<Worker>>,
        output: dispatch::Output,
        remote: Arc<dispatch::RemoteAuthority>,
    ) -> io::Result<Arc<Self>> {
        let (jobs, queue) = mpsc::channel::<Job>();
        let facts = Arc::new(Mutex::new(Facts::default()));
        let known = facts.clone();
        let thread = std::thread::Builder::new()
            .name("screen-worker-input".into())
            .spawn(move || {
                // Input of the viewer is the most urgent work of the process.
                crate::screen::os::worker_thread::raise_priority();
                loop {
                    let job = match queue.recv_timeout(REFUSAL_CHECK) {
                        Ok(job) => Some(job),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let written = match job {
                        Some(job) => {
                            let reply = answer(&worker, &known, &remote, job);
                            output
                                .lock()
                                .map_err(|_| io::Error::other("worker output poisoned"))
                                .and_then(|mut output| send_reply(&mut *output, &reply))
                                // Codex 4/17: also while input keeps coming
                                // (no idle 50 ms), a refusal goes at once.
                                .and_then(|()| report_refusal(&known, &output))
                        }
                        None => report_refusal(&known, &output),
                    };
                    if let Err(error) = written {
                        eprintln!("screen host process input: {error}");
                        break;
                    }
                }
            })?;
        Ok(Arc::new(Self {
            jobs: Mutex::new(Some(jobs)),
            facts,
            thread: Mutex::new(Some(thread)),
        }))
    }

    fn submit(&self, job: Job) {
        let jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(jobs) = jobs.as_ref() {
            let _ = jobs.send(job);
        }
    }

    /// Takes what input needs of `worker` after a request changed it.
    pub(super) fn publish(&self, worker: &Worker) {
        let facts = Facts {
            control: worker.control,
            family: worker.family.is_some(),
            sink: worker
                .source
                .as_ref()
                .and_then(|source| source.input_sink()),
            release_sink: worker.release_sink.clone(),
            backend: Some(worker.backend.clone()),
            legacy: worker
                .source
                .as_ref()
                .is_some_and(|source| source.legacy_input_invalid()),
        };
        *self.facts.lock().unwrap_or_else(PoisonError::into_inner) = facts;
    }

    /// Ends the input thread after the input it already has.
    pub(super) fn stop(&self) {
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

/// How often the input thread looks for input its source's sink refused
/// after the answer (review 2026-11-19).
const REFUSAL_CHECK: std::time::Duration = std::time::Duration::from_millis(50);

/// Input the source's sink refused after its answer (the Wayland portal,
/// `ScreenInputSink::take_refusal`) goes to the daemon unasked
/// (`INPUT_REFUSED`), so its session reports it without waiting for the
/// next `INPUT`; the answer to that `INPUT` takes it when it comes first.
fn report_refusal(facts: &Mutex<Facts>, output: &dispatch::Output) -> io::Result<()> {
    let refused = {
        let known = facts.lock().unwrap_or_else(PoisonError::into_inner);
        known
            .sink
            .as_ref()
            .or(known.release_sink.as_ref())
            .and_then(|sink| sink.take_refusal())
    };
    let Some(block) = refused else {
        return Ok(());
    };
    let mut output = output
        .lock()
        .map_err(|_| io::Error::other("worker output poisoned"))?;
    crate::screen::core::worker_wire::send_input_refused(&mut *output, block)
}

/// The answer to one input job.
fn answer(
    worker: &Mutex<Worker>,
    facts: &Mutex<Facts>,
    remote: &dispatch::RemoteAuthority,
    job: Job,
) -> WorkerReply {
    let mut guard = match worker.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    };
    if guard.is_none() {
        let known = facts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(reply) = without_source(&known, remote, &job) {
            return reply;
        }
    }
    let mut worker = match guard.take() {
        Some(guard) => guard,
        None => worker.lock().unwrap_or_else(PoisonError::into_inner),
    };
    match job {
        Job::Input(input) => worker.input(input),
        Job::Release(events) => worker.release(events),
    }
}

/// Input that needs neither the source nor the family, answered while the
/// worker lock is taken; `None` when the job needs the worker.
fn without_source(
    facts: &Facts,
    remote: &dispatch::RemoteAuthority,
    job: &Job,
) -> Option<WorkerReply> {
    match job {
        Job::Input(input) => {
            let sink = facts.sink.as_ref()?;
            let blocked = if !facts.control || facts.family || remote.is_closed() {
                Some(ControlBlock::ControlUnavailable)
            } else {
                sink.inject(&input.events, input.reference_generation, input.presented)
                    .err()
            };
            Some(WorkerReply::InputDone {
                blocked,
                legacy_reconnect: facts.legacy || sink.legacy_input_invalid(),
            })
        }
        Job::Release(events) => {
            if facts.family {
                return None;
            }
            let blocked = if !release_only(events) || !facts.control {
                Some(ControlBlock::ControlUnavailable)
            } else if let Some(sink) = facts.release_sink.as_ref() {
                sink.release(events).err()
            } else {
                facts.backend.as_ref()?.release_input(events).err()
            };
            Some(WorkerReply::InputDone {
                blocked,
                legacy_reconnect: facts.legacy,
            })
        }
    }
}

/// Routes the dispatcher's requests: input to `lane`, orders to `stream`,
/// the rest to the returned receiver (the request loop). Ends after `CLOSE`,
/// a broken request or the end of the input.
pub(super) fn route(
    requests: mpsc::Receiver<io::Result<WorkerRequest>>,
    lane: Arc<InputLane>,
    stream: Arc<super::clock::Stream>,
) -> io::Result<(mpsc::Receiver<io::Result<WorkerRequest>>, JoinHandle<()>)> {
    let (forward, routed) = mpsc::channel();
    let router = std::thread::Builder::new()
        .name("screen-worker-router".into())
        .spawn(move || {
            for request in requests {
                let request = match request {
                    Ok(WorkerRequest::Input(input)) => {
                        lane.submit(Job::Input(input));
                        continue;
                    }
                    Ok(WorkerRequest::Release(events)) => {
                        lane.submit(Job::Release(events));
                        continue;
                    }
                    Ok(WorkerRequest::Stream(order)) => {
                        stream.order(order);
                        continue;
                    }
                    other => other,
                };
                let last = matches!(request, Ok(WorkerRequest::Close) | Err(_));
                if forward.send(request).is_err() || last {
                    break;
                }
            }
        })?;
    Ok((routed, router))
}
