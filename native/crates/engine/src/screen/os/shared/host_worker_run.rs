//! Worker lifetime and permanently active resource-control dispatcher. The
//! worker sits behind one lock that this loop (control requests and
//! `PRODUCE`), the capture stage of the picture stream (`host_worker_clock`)
//! and the input thread (`host_worker_input`) share; capture holds it only
//! for the capture call, encoding never.
use super::*;
use crate::screen::core::{capture_bytes::AdmittedBytes, capture_transport};
use crate::screen::os::host_worker_client::session_queries as queries;
#[cfg(target_os = "linux")]
use crate::screen::os::session_detect::active_session::process as session_host;
use std::sync::{mpsc, Mutex, PoisonError};

/// No other platform starts a host process in another account's session.
#[cfg(not(target_os = "linux"))]
mod session_host {
    pub fn seal_host() -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct Reference {
    pub(super) generation: u64,
    pub(super) regions: bool,
    pub(super) limits: FrameLimits,
    pub(super) family_layout_revision: Option<u64>,
}
impl Reference {
    pub(super) fn of(worker: &Worker) -> Self {
        Self {
            generation: worker
                .source
                .as_ref()
                .map_or(0, |source| source.reference_generation()),
            regions: worker
                .source
                .as_ref()
                .is_none_or(|source| source.supports_frame_regions()),
            limits: worker
                .source
                .as_ref()
                .map_or(FrameLimits::LEGACY, |source| source.frame_limits()),
            family_layout_revision: worker
                .source
                .as_ref()
                .and_then(|source| source.family_layout_revision()),
        }
    }
    pub(super) fn wrap(self, reply: WorkerReply) -> WorkerReply {
        WorkerReply::Reference {
            generation: self.generation,
            regions: self.regions,
            limits: self.limits,
            family_layout_revision: self.family_layout_revision,
            reply: Box::new(reply),
        }
    }
}

/// Both the original picture and the serialized transport outlive write's
/// output guard. Their drops can synchronously publish monotone reductions.
pub(super) struct PreparedReply {
    reply: WorkerReply,
    bytes: Option<AdmittedBytes>,
}
impl PreparedReply {
    pub(super) fn write(&self, output: &dispatch::Output) -> io::Result<()> {
        let mut output = output
            .lock()
            .map_err(|_| io::Error::other("worker output poisoned"))?;
        if let Some(bytes) = &self.bytes {
            capture_transport::send_prepared(&mut *output, bytes)
        } else {
            send_reply(&mut *output, &self.reply)
        }
    }
}
impl Worker {
    fn waiting_picture(&self, raw: bool) -> WorkerReply {
        let Some(source) = self.source.as_ref() else {
            return failed(ScreenErrorCode::CaptureFailed, "screen source unavailable");
        };
        WorkerReply::Frame(if raw {
            StageFrame::Paused(crate::screen::core::session_state::SessionState::Starting)
        } else {
            StageFrame::Unchanged {
                source_size: source.size(),
            }
        })
    }
    pub(super) fn prepare_reply(&mut self, request: WorkerRequest) -> io::Result<PreparedReply> {
        let raw = matches!(&request, WorkerRequest::Capture { .. });
        let picture_request = raw || matches!(&request, WorkerRequest::Produce(_));
        let before = Reference::of(self);
        let mut reply = self.handle(request);
        let after = Reference::of(self);
        if after.generation != self.reference_generation {
            self.reference_generation = after.generation;
            self.encoder = None;
        }
        if matches!(&reply, WorkerReply::CaptureControl(_)) {
            return Ok(PreparedReply { reply, bytes: None });
        }
        let mut grant = capture_transport::picture_grant(&reply)?;
        if picture_request && before.generation != after.generation {
            self.encoder = None;
            reply = match reply {
                // A source can concurrently report a concrete pause or deny
                // permission; retain those existing user-visible reasons.
                paused @ WorkerReply::Frame(StageFrame::Paused(_)) => paused,
                denied @ WorkerReply::Failed {
                    code: ScreenErrorCode::NoDisplay,
                    ..
                } => denied,
                _ => self.waiting_picture(raw),
            };
            grant = None;
        }
        // A picture is bound to the reference observed before its production.
        // A later callback may retire it, but can never relabel its pixels.
        // Layout metadata belongs to the capture just completed. Preserve its
        // revision even when the preceding frame had no published layout.
        let reference = if grant.is_some() {
            Reference {
                generation: before.generation,
                ..after
            }
        } else {
            after
        };
        reply = reference.wrap(reply);
        if let Some(grant) = grant {
            let id = self.remote.id_for(&grant)?;
            match capture_transport::prepare_owned(&reply, id, &grant)? {
                Some(bytes) => {
                    return Ok(PreparedReply {
                        reply,
                        bytes: Some(bytes),
                    })
                }
                None => {
                    // The last transport claim lost to source retirement.
                    // No bytes were allocated or written. Keep this worker.
                    self.encoder = None;
                    let reference = Reference::of(self);
                    self.reference_generation = reference.generation;
                    reply = reference.wrap(self.waiting_picture(raw));
                }
            }
        }
        Ok(PreparedReply { reply, bytes: None })
    }
}

/// Sends what changed of the pointer ahead of an answer (P6): nothing until
/// the daemon switched the overlay on.
pub(super) fn flush_cursor(output: &dispatch::Output) -> io::Result<()> {
    let cursor = crate::screen::core::cursor_overlay::worker();
    if !cursor.has_pending() {
        return Ok(());
    }
    let mut output = output
        .lock()
        .map_err(|_| io::Error::other("worker output poisoned"))?;
    cursor.flush(&mut *output)
}

pub(super) fn run() -> i32 {
    if let Err(error) = super::super::worker_lifetime::arm_worker() {
        eprintln!("screen host process: {error}");
        return 1;
    }
    // A host started by a root daemon in an X11 session: only the daemon
    // holds its pipes (E23, S3).
    if let Err(error) = session_host::seal_host() {
        eprintln!("screen host process: {error}");
        return 1;
    }
    // One query of a worker that follows this session (E47): answered as
    // this account, without a source and without the update-stop watch.
    if queries::requested() {
        let backend = super::super::platform_backend();
        return queries::serve(io::stdin(), io::stdout(), &*backend, || {
            codec_inventory::registry().encoder_offers()
        });
    }
    let _stop =
        match crate::daemon::spawn_update_stop_watch(crate::daemon::UPDATE_STOP_POLL, || {
            std::process::exit(crate::daemon::EXIT_UPDATE_STOP)
        }) {
            Ok(watch) => watch,
            Err(error) => {
                eprintln!("screen host process update-stop: {error}");
                return 1;
            }
        };
    // Merge W2: GPU pictures from the capture to the encoder (Linux: the
    // DMA-BUF offer of the PipeWire streams, BK8b).
    super::super::enable_gpu_pictures();
    let output: dispatch::Output = Arc::new(std::sync::Mutex::new(Box::new(io::stdout())));
    let remote = dispatch::RemoteAuthority::new(output.clone());
    let requests = match dispatch::start(io::stdin(), remote.clone()) {
        Ok(requests) => requests,
        Err(error) => {
            eprintln!("screen host process: {error}");
            return 1;
        }
    };
    let worker = Arc::new(Mutex::new(Worker {
        admitted: None,
        backend: super::super::platform_backend(),
        source: None,
        encoder: None,
        refused: Vec::new(),
        produced: 0,
        checked: Instant::now(),
        control: false,
        capture_active: true,
        reference_generation: 0,
        family: None,
        prepared: None,
        planned: None,
        capture_session: None,
        remote: remote.clone(),
        release_sink: None,
    }));
    let stream = super::clock::Stream::new(
        worker.clone(),
        output.clone(),
        remote.clone(),
        Arc::new(super::clock::Registry),
    );
    let lane = match super::input::InputLane::start(worker.clone(), output.clone(), remote) {
        Ok(lane) => lane,
        Err(error) => {
            eprintln!("screen host process: {error}");
            return 1;
        }
    };
    let (requests, router) = match super::input::route(requests, lane.clone(), stream.clone()) {
        Ok(routed) => routed,
        Err(error) => {
            eprintln!("screen host process: {error}");
            return 1;
        }
    };
    let code = serve(&worker, &stream, &lane, &output, requests);
    if code != 0 {
        return code;
    }
    // Stages, input and router end before the worker, its source and its
    // encoders are torn down; the Parent still retains unretired grants
    // until its wait confirms the actual exit.
    stream.stop();
    lane.stop();
    let _ = router.join();
    drop((stream, lane));
    drop(worker);
    0
}

/// Answers the requests the router leaves to this loop, one after another.
fn serve(
    worker: &Mutex<Worker>,
    stream: &super::clock::Stream,
    lane: &super::input::InputLane,
    output: &dispatch::Output,
    requests: mpsc::Receiver<io::Result<WorkerRequest>>,
) -> i32 {
    for request in requests {
        let request = match request {
            Ok(WorkerRequest::Close) => break,
            Ok(request) => request,
            Err(error) => {
                eprintln!("screen host process: {error}");
                return 1;
            }
        };
        // A request that takes pictures or ends their production rests the
        // stages of the picture stream first (their memory is the same).
        let quiet = super::clock::quiet_for(&request).map(|reset| stream.quiesce(reset));
        let reply = {
            let mut worker = worker.lock().unwrap_or_else(PoisonError::into_inner);
            let reply = worker.prepare_reply(request);
            lane.publish(&worker);
            reply
        };
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                eprintln!("screen host process: {error}");
                return 1;
            }
        };
        // The pointer of the overlay goes out ahead of the answer, so the
        // daemon has it when the picture is done.
        let sent = flush_cursor(output).and_then(|()| reply.write(output));
        drop(reply);
        drop(quiet);
        if let Err(error) = sent {
            eprintln!("screen host process: {error}");
            return 1;
        }
    }
    0
}

#[cfg(test)]
#[path = "batches_capture_worker_fix_task_tests.rs"]
mod tests;
