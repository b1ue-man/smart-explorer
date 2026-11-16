//! The capture thread of an activated Windows source (plan 2026-11-14
//! bildkette, B-10). The thread owns the grabber's native waits: it asks the
//! compositor for the next picture (`AcquireNextFrame` with a timeout,
//! Graphics Capture's `FrameArrived`) shortly before the pipeline's next
//! request is due (`dxgi_wait.rs`) and keeps the newest picture. A `Produce`
//! request takes that picture, or "unchanged", and never starts a capture
//! itself; the interface of `Grabber` and `WindowsSource` stays as it was
//! (package BK3 turns the chain around).
//!
//! Memory: the capture plan admits two distinct source images (the picture
//! the pipeline still holds and a new one). A picture the pipeline never took
//! is therefore dropped right before the next readback (`release`), and the
//! new picture then carries no regions (they would describe a change against
//! a picture nobody saw).
//!
//! Pauses stay where they were: `WindowsSource` checks the secure desktop,
//! the session and the window before every request, and the thread captures
//! nothing while the same checks pause it. Lock order: grabber, then slot.
//!
//! BK3b: the kept pictures may lie on the GPU (`Picture::Gpu`, while a
//! texture encoder is armed); `capture_picture` hands them out as they are,
//! `capture` reads them back. The source's generation can be read without
//! the grabber (`generation_reader`), for input on its own way.
//!
//! Merge W1: a key frame of a picture in a retired NV12 ring makes a new
//! capture session (`renew`); a still image (`exact_frame`) is the exact BGRA
//! of the next picture (`d3d_input::request_exact`), on a still screen of a
//! new capture session.
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{
    state_before_capture, update_report, CapturePath, GenerationReader, Grabber, StateProbe,
    FIRST_FRAME_WAIT,
};
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::capture_picture::{Picture, SourcePicture};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::os::capture_dxgi::wait::{Cadence, Schedule, Step, MAX_BLOCK};
use crate::screen::os::mf_encoder::d3d_input;

/// What the request side reads without waiting for the native objects.
#[derive(Clone)]
struct Meta {
    size: FrameSize,
    generation: u64,
    limits: FrameLimits,
    grant: Option<CaptureGrant>,
    available: bool,
    path: Option<CapturePath>,
}

impl Meta {
    fn of(grabber: &dyn Grabber) -> Self {
        Self {
            size: grabber.size(),
            generation: grabber.reference_generation(),
            limits: grabber.frame_limits(),
            grant: grabber.capture_grant(),
            available: grabber.source_available(),
            path: grabber.path(),
        }
    }
}

struct Slot {
    /// The newest picture, not handed out yet.
    fresh: Option<Picture>,
    /// The newest picture, for key frames.
    last: Option<Picture>,
    paused: Option<SessionState>,
    error: Option<io::Error>,
    /// A readback runs; its picture follows within milliseconds.
    reading: bool,
    active: bool,
    stop: bool,
    /// Make the capture session anew and take its first picture (merge W1).
    renew: bool,
    /// Pictures published so far (a still image waits for a newer one).
    published: u64,
    cadence: Cadence,
    meta: Meta,
}

struct Shared {
    slot: Mutex<Slot>,
    changed: Condvar,
    grabber: Mutex<Box<dyn Grabber>>,
}

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn wait<'a>(
    changed: &Condvar,
    slot: MutexGuard<'a, Slot>,
    timeout: Duration,
) -> MutexGuard<'a, Slot> {
    changed
        .wait_timeout(slot, timeout)
        .map(|(slot, _)| slot)
        .unwrap_or_else(|error| error.into_inner().0)
}

/// `grabber` capturing on its own thread; if the thread cannot start, the
/// returned grabber captures per request as before.
pub(super) fn start(
    grabber: Box<dyn Grabber>,
    probe: Box<dyn StateProbe + Send>,
    window: bool,
) -> Box<dyn Grabber> {
    let meta = Meta::of(grabber.as_ref());
    update_report(|report| report.path = meta.path);
    let shared = Arc::new(Shared {
        slot: Mutex::new(Slot {
            fresh: None,
            last: None,
            paused: None,
            error: None,
            reading: false,
            active: true,
            stop: false,
            renew: false,
            published: 0,
            cadence: Cadence::default(),
            meta,
        }),
        changed: Condvar::new(),
        grabber: Mutex::new(grabber),
    });
    let worker = shared.clone();
    let thread = std::thread::Builder::new()
        .name("screen-capture".into())
        .spawn(move || run(&worker, probe, window))
        .map_err(|error| eprintln!("screen capture thread did not start: {error}"))
        .ok();
    Box::new(ThreadedGrabber { shared, thread })
}

/// Sleeps up to `pause`; commands and the end wake it.
fn idle(shared: &Shared, pause: Duration) {
    let slot = lock(&shared.slot);
    if !slot.stop {
        drop(wait(&shared.changed, slot, pause));
    }
}

/// Right before a readback: drops a picture nobody took and tells whether
/// the previous picture was handed out.
fn release(shared: &Shared) -> bool {
    let mut slot = lock(&shared.slot);
    slot.reading = true;
    let delivered = slot.fresh.is_none();
    if !delivered {
        slot.fresh = None;
        slot.last = None;
    }
    delivered
}

/// Hands one capture result to the request side; true for a new picture.
fn publish(shared: &Shared, result: io::Result<SourcePicture>, meta: Meta) -> bool {
    let mut slot = lock(&shared.slot);
    slot.reading = false;
    if slot.meta.generation != meta.generation {
        // A new capture session (rebuilt, or Graphics Capture after Desktop
        // Duplication): pictures of the old one are not repeated.
        slot.fresh = None;
        slot.last = None;
    }
    let mut picture = false;
    match result {
        Ok(SourcePicture::Frame(image)) => {
            slot.last = Some(image.clone());
            slot.fresh = Some(image);
            slot.paused = None;
            slot.published = slot.published.wrapping_add(1);
            picture = true;
        }
        Ok(SourcePicture::Unchanged) => {}
        Ok(SourcePicture::Paused(state)) => {
            slot.fresh = None;
            slot.last = None;
            slot.paused = Some(state);
        }
        Err(error) => slot.error = Some(error),
    }
    let path = meta.path;
    let changed_path = slot.meta.path != path;
    slot.meta = meta;
    drop(slot);
    shared.changed.notify_all();
    if changed_path {
        update_report(|report| {
            report.path = path;
            if path != Some(CapturePath::GraphicsCapture) {
                report.rate_capped = false;
            }
        });
    }
    picture
}

fn run(shared: &Shared, mut probe: Box<dyn StateProbe + Send>, window: bool) {
    crate::screen::os::mf_common::ensure_com();
    let mut schedule = Schedule::default();
    let mut interval_set: Option<(Duration, Option<CapturePath>)> = None;
    loop {
        let (step, interval, renew) = {
            let mut slot = lock(&shared.slot);
            loop {
                if slot.stop {
                    return;
                }
                // An error waits until a request took it.
                if slot.active && slot.error.is_none() {
                    break;
                }
                slot = wait(&shared.changed, slot, MAX_BLOCK);
            }
            (
                schedule.step(Instant::now(), &slot.cadence),
                slot.cadence.interval(),
                std::mem::take(&mut slot.renew),
            )
        };
        if !matches!(state_before_capture(window, probe.as_mut()), Ok(None)) {
            lock(&shared.slot).renew |= renew;
            idle(shared, MAX_BLOCK);
            continue;
        }
        let (timeout, due) = match (renew, step) {
            (true, _) => (MAX_BLOCK, None), // a later step takes a late one
            (false, Step::Sleep(pause)) => {
                idle(shared, pause);
                continue;
            }
            (false, Step::Acquire { wait, due }) => (wait, due),
        };
        let mut grabber = lock(&shared.grabber);
        if renew {
            // A new session starts with a whole picture of the screen.
            if let Err(error) = grabber.reopen() {
                let meta = Meta::of(grabber.as_ref());
                drop(grabber);
                publish(shared, Err(error), meta);
                continue;
            }
        }
        let path = grabber.path();
        if let Some(interval) = interval.filter(|i| interval_set != Some((*i, path))) {
            let honored = grabber.set_frame_interval(interval);
            interval_set = Some((interval, path));
            if path == Some(CapturePath::GraphicsCapture) {
                update_report(|report| report.rate_capped = !honored);
            }
        }
        let mut read_started = None;
        let result = grabber.capture_waiting_picture(false, timeout, &mut || {
            read_started = Some(Instant::now());
            release(shared)
        });
        let meta = Meta::of(grabber.as_ref());
        drop(grabber);
        let ready = Instant::now();
        if publish(shared, result, meta) {
            if let Some(started) = read_started {
                schedule.record_cost(ready.saturating_duration_since(started));
            }
            if let Some(due) = due {
                schedule.served(due);
            }
        }
    }
}

struct ThreadedGrabber {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadedGrabber {
    fn meta(&self) -> Meta {
        lock(&self.shared.slot).meta.clone()
    }

    /// Without a running thread (it did not start or ended) the request
    /// captures itself, as before B-10.
    fn threaded(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }

    /// Runs `command` on the grabber; with `reset` (a new capture session or
    /// a stopped one) the pictures and states the thread kept are dropped.
    fn command<T>(
        &mut self,
        active: Option<bool>,
        reset: bool,
        command: impl FnOnce(&mut dyn Grabber) -> T,
    ) -> T {
        let mut grabber = lock(&self.shared.grabber);
        let result = command(grabber.as_mut());
        let meta = Meta::of(grabber.as_ref());
        let mut slot = lock(&self.shared.slot);
        if reset {
            slot.fresh = None;
            slot.last = None;
            slot.paused = None;
            slot.error = None;
        }
        slot.meta = meta;
        if let Some(active) = active {
            slot.active = active;
        }
        drop(slot);
        drop(grabber);
        self.shared.changed.notify_all();
        result
    }
}

impl Grabber for ThreadedGrabber {
    fn size(&self) -> FrameSize {
        self.meta().size
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.meta().grant
    }
    fn frame_limits(&self) -> FrameLimits {
        self.meta().limits
    }
    fn reference_generation(&self) -> u64 {
        self.meta().generation
    }
    fn source_available(&self) -> bool {
        self.meta().available
    }
    fn path(&self) -> Option<CapturePath> {
        self.meta().path
    }

    /// CPU pixels: a picture that lies on the GPU is captured anew exactly.
    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        let picture = self.capture_picture(keyframe)?;
        self.exact_frame(picture)
    }

    /// Merge W1: a GPU picture becomes the exact BGRA of the next picture,
    /// read back from the capture's source texture once; on a still screen
    /// after `MAX_BLOCK` a new capture session delivers it.
    fn exact_frame(&mut self, picture: SourcePicture) -> io::Result<SourceFrame> {
        if !matches!(picture, SourcePicture::Frame(Picture::Gpu(_))) {
            return picture.into_source_frame();
        }
        d3d_input::request_exact();
        if !self.threaded() {
            let mut grabber = lock(&self.shared.grabber);
            let result = grabber
                .reopen()
                .and_then(|()| grabber.capture_picture(true));
            lock(&self.shared.slot).meta = Meta::of(grabber.as_ref());
            return result?.into_source_frame();
        }
        let start = Instant::now();
        let end = start + MAX_BLOCK + FIRST_FRAME_WAIT;
        let mut slot = lock(&self.shared.slot);
        let seen = slot.published;
        let mut renewed = false;
        loop {
            if let Some(error) = slot.error.take() {
                drop(slot);
                self.shared.changed.notify_all();
                return Err(error);
            }
            if slot.published != seen {
                if let Some(Picture::Cpu(image)) = &slot.last {
                    return Ok(SourceFrame::Frame(image.clone()));
                }
            }
            if let Some(state) = slot.paused.filter(|state| *state != SessionState::Starting) {
                return Ok(SourceFrame::Paused(state));
            }
            let now = Instant::now();
            if !renewed && now >= start + MAX_BLOCK {
                renewed = true;
                slot.renew = true;
                d3d_input::request_exact();
                self.shared.changed.notify_all();
            }
            if now >= end {
                return Ok(SourceFrame::Paused(SessionState::Starting));
            }
            let until = if renewed { end } else { start + MAX_BLOCK };
            slot = wait(
                &self.shared.changed,
                slot,
                until.saturating_duration_since(now),
            );
        }
    }

    fn generation_reader(&self) -> Option<GenerationReader> {
        let shared = self.shared.clone();
        Some(Arc::new(move || lock(&shared.slot).meta.generation))
    }

    /// The newest picture the compositor delivered, or "unchanged"; with
    /// `keyframe` the newest picture again. Only before the first picture
    /// (and after a lost one) this waits, as long as a capture did before.
    fn capture_picture(&mut self, keyframe: bool) -> io::Result<SourcePicture> {
        if !self.threaded() {
            let mut grabber = lock(&self.shared.grabber);
            let mut result = grabber.capture_picture(keyframe);
            if keyframe && matches!(&result, Ok(SourcePicture::Frame(p)) if !p.held()) {
                // Merge W1: the key frame of a retired ring is captured anew.
                result = grabber
                    .reopen()
                    .and_then(|()| grabber.capture_picture(true));
            }
            lock(&self.shared.slot).meta = Meta::of(grabber.as_ref());
            return result;
        }
        let mut deadline = Instant::now() + FIRST_FRAME_WAIT;
        let mut slot = lock(&self.shared.slot);
        slot.cadence.record(Instant::now());
        loop {
            if let Some(error) = slot.error.take() {
                drop(slot);
                self.shared.changed.notify_all();
                return Err(error);
            }
            if let Some(image) = slot.fresh.take() {
                return Ok(SourcePicture::Frame(image));
            }
            let now = Instant::now();
            if keyframe && slot.last.as_ref().is_some_and(|last| !last.held()) {
                // Merge W1: the kept picture lay in a retired NV12 ring; a
                // new capture session delivers the key frame's picture.
                slot.last = None;
                slot.renew = true;
                deadline = now + FIRST_FRAME_WAIT;
                self.shared.changed.notify_all();
            }
            if !slot.reading || now >= deadline {
                if let Some(last) = &slot.last {
                    return Ok(if keyframe {
                        SourcePicture::Frame(last.clone())
                    } else {
                        SourcePicture::Unchanged
                    });
                }
                match slot.paused {
                    Some(state) if state != SessionState::Starting => {
                        return Ok(SourcePicture::Paused(state))
                    }
                    _ if now >= deadline => {
                        return Ok(SourcePicture::Paused(SessionState::Starting))
                    }
                    _ => {}
                }
            }
            slot = wait(&self.shared.changed, slot, deadline - now);
        }
    }

    fn reopen(&mut self) -> io::Result<()> {
        self.command(None, true, |grabber| grabber.reopen())
    }

    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.command(Some(active), !active, |grabber| {
            grabber.set_capture_active(active)
        })
    }
}

impl Drop for ThreadedGrabber {
    fn drop(&mut self) {
        lock(&self.shared.slot).stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            // The thread blocks at most `MAX_BLOCK` in one native wait.
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
#[path = "bildkette_w1_thread_task_tests.rs"]
mod bildkette_w1_thread_task_tests;
