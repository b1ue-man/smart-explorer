//! The capture stage of the picture stream (plan 2026-11-14 „Bildkette“
//! B-27): one capture call per interval of the session's cadence while the
//! stream wants pictures, with the checks `PRODUCE` makes around it
//! (`host_worker_produce`). It holds the worker lock for the capture call
//! only; the newest picture replaces a waiting one before the next capture.
use super::*;
use crate::screen::core::capture_picture::SourcePicture;

/// The capture stage: one capture per interval of the cadence while the
/// stream wants pictures.
pub(super) fn capture_loop(shared: Arc<Shared>) {
    crate::screen::os::worker_thread::raise_priority();
    let waiter = crate::screen::os::worker_thread::PreciseWait::default();
    let mut due: Option<Instant> = None;
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if state.stopping {
                    return;
                }
                if let Some(job) = state.capture_job() {
                    break job;
                }
                due = None;
                state = shared.wait(state, RECHECK);
            }
        };
        let now = Instant::now();
        // Late by more than one interval: the cadence starts over.
        let at = due.filter(|at| *at + job.interval >= now).unwrap_or(now);
        waiter.until(at);
        due = Some(at + job.interval);
        let (frame_id, refresh, waiting) = {
            let mut state = shared.lock();
            if state.stopping {
                return;
            }
            if state.quiet > 0 || state.serial != job.serial {
                continue;
            }
            // The newest picture replaces a waiting one; the waiting one
            // goes first, so at most two pictures exist at a time.
            let waiting = state.slot.take();
            if waiting.is_some() {
                state.dropped += 1;
                // Its dirty rectangles go with it (P9, lossless tiles).
                crate::screen::core::host_codec_adapters::note_dropped_picture();
            }
            state.capturing = true;
            // A key frame, a continuation (B-36), a lossless correction or a
            // dropped waiting picture needs a picture even when nothing changed.
            let refresh = waiting.is_some()
                || state.key_wanted
                || state.recover.is_some()
                || state.needs_refresh;
            (state.next_id, refresh, waiting)
        };
        drop(waiting);
        let tick = {
            let mut worker = shared.worker.lock().unwrap_or_else(PoisonError::into_inner);
            capture_once(&mut worker, &job, frame_id, refresh)
        };
        let gone = {
            let mut state = shared.lock();
            state.capturing = false;
            let gone = state.store(tick, job.serial);
            shared.changed.notify_all();
            gone
        };
        drop(gone);
        // The pointer of the overlay follows the capture, not the encoding.
        if let Err(error) = super::super::run::flush_cursor(&shared.output) {
            eprintln!("screen host process: {error}");
        }
    }
}

fn wrap(worker: &Worker, reply: WorkerReply) -> WorkerReply {
    super::super::run::Reference::of(worker).wrap(reply)
}

fn unchanged(source_size: FrameSize) -> WorkerReply {
    WorkerReply::Frame(StageFrame::Unchanged { source_size })
}

/// One capture call with the checks of `PRODUCE` (`host_worker_produce`)
/// around it.
fn capture_once(worker: &mut Worker, job: &CaptureJob, frame_id: u64, refresh: bool) -> Tick {
    let request = &job.request;
    if let Err(error) = worker.admit(request.frame_contract) {
        return Tick::Event(
            wrap(worker, failed(ScreenErrorCode::CaptureFailed, error)),
            true,
        );
    }
    let Some((source_size, source_limits, before)) = worker.source.as_ref().map(|source| {
        (
            source.size(),
            source.frame_limits(),
            source.reference_generation(),
        )
    }) else {
        let reply = failed(ScreenErrorCode::CaptureFailed, "no source is open");
        return Tick::Event(wrap(worker, reply), true);
    };
    if !worker.capture_active {
        return Tick::Unchanged(wrap(worker, unchanged(source_size)));
    }
    let limits = request
        .frame_contract
        .map_or(FrameLimits::LEGACY, |contract| contract.limits)
        .for_capture(source_limits);
    if let Err(error) = limits.check(request.size) {
        return Tick::Event(
            wrap(worker, failed(ScreenErrorCode::CaptureFailed, error)),
            true,
        );
    }
    if before != worker.reference_generation {
        worker.reference_generation = before;
        worker.encoder = None;
        return Tick::Turnover(wrap(worker, unchanged(source_size)));
    }
    // A stream set up for an older, larger source waits for the next order.
    if request.size.width > source_size.width || request.size.height > source_size.height {
        return Tick::Unchanged(wrap(worker, unchanged(source_size)));
    }
    let capture_ts_us = now_us();
    let started = Instant::now();
    let Some((captured, after, source_size, grant)) = worker.source.as_mut().map(|source| {
        // A picture may stay on the GPU (BK3b): the encoding stage hands it
        // to a GPU encoder, every other encoder gets it read back.
        let captured = source.capture_picture(frame_id, refresh);
        (
            captured,
            source.reference_generation(),
            source.size(),
            source.capture_grant(),
        )
    }) else {
        let reply = failed(ScreenErrorCode::CaptureFailed, "no source is open");
        return Tick::Event(wrap(worker, reply), true);
    };
    let capture_us = micros(started.elapsed());
    if after != before {
        worker.reference_generation = after;
        worker.encoder = None;
        let reply = match captured {
            Ok(SourcePicture::Paused(state)) => WorkerReply::Frame(StageFrame::Paused(state)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => unchanged(source_size),
            Err(error) => return Tick::Event(wrap(worker, capture_failed(&error)), true),
            _ => unchanged(source_size),
        };
        return Tick::Turnover(wrap(worker, reply));
    }
    let reference = super::super::run::Reference::of(worker);
    match captured {
        Ok(SourcePicture::Frame(image)) => Tick::Picture(Captured {
            image,
            at: Instant::now(),
            frame_id,
            capture_ts_us,
            capture_us,
            serial: job.serial,
            grant,
            limits,
            generation: before,
            reference,
        }),
        Ok(SourcePicture::Unchanged) => Tick::Unchanged(reference.wrap(unchanged(source_size))),
        Ok(SourcePicture::Paused(state)) => Tick::Event(
            reference.wrap(WorkerReply::Frame(StageFrame::Paused(state))),
            false,
        ),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            Tick::Unchanged(reference.wrap(unchanged(source_size)))
        }
        Err(error) => Tick::Event(reference.wrap(capture_failed(&error)), true),
    }
}
