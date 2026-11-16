//! The host process's own clock (plan 2026-11-14 „Bildkette“ B-27, B-29).
//! Once the daemon sends a `STREAM` order, two stages of their own produce
//! the pictures: the capture stage captures at the session's cadence (the
//! order's `cadence_fps`, followed when it changes) and keeps only the
//! newest picture: a waiting one is dropped before the next capture and
//! counted (at most one picture lies between the stages, and with the one
//! the encoder holds at most two exist, the count every capture plan
//! admits). The encoding stage (`host_worker_encode`) scales and encodes a
//! picture only for credit: one answer per credit the daemon granted, and
//! with `ahead` one picture more, encoded while the daemon sends the
//! previous one and handed over with the next credit. Without credit
//! nothing is encoded. The capture stage holds the worker lock only for the
//! capture call itself, so input (`host_worker_input`) never waits for
//! scaling, conversion or encoding. Both stages run with raised thread
//! priority on Windows (`os::worker_thread`, B-29). A source that waits for
//! the compositor inside its capture call (planned by BK2b) paces the stage
//! by itself: the stage then waits only for what is left of the interval.
use super::*;
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::capture_picture::Picture;
use crate::screen::core::frame_budget::FrameContract;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::worker_wire::StreamOrder;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

#[path = "host_worker_capture_stage.rs"]
mod capture;
#[path = "host_worker_encode.rs"]
mod encode;
#[cfg(all(test, windows))]
pub(super) use encode::Encoding;
pub(super) use encode::{EncoderOpener, Registry};

/// Without an order and without open credit the capture stage rests
/// (hidden viewer, long back pressure); the next order wakes it.
const ORDER_IDLE: Duration = Duration::from_secs(2);
/// Longest wait of a stage before it looks at its state again.
const RECHECK: Duration = Duration::from_millis(100);

pub(super) fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// Whether `request` needs the stages at rest (it takes pictures, or ends
/// or restarts their production), and whether the stream's encoder goes.
pub(super) fn quiet_for(request: &WorkerRequest) -> Option<bool> {
    use crate::screen::core::worker_capture_wire::CaptureRequest;
    match request {
        WorkerRequest::Capture { .. } | WorkerRequest::Produce(_) => Some(false),
        WorkerRequest::SetCaptureActive(_)
        | WorkerRequest::CaptureControl(CaptureRequest::Activity(_)) => Some(true),
        _ => None,
    }
}

/// What makes an encoder: a change starts a new one with a key frame.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    epoch: u64,
    choice: CodecChoice,
    contract: Option<FrameContract>,
    size: FrameSize,
}

impl Identity {
    fn of(request: &StageRequest) -> Self {
        Self {
            epoch: request.encoder_epoch,
            choice: request.choice,
            contract: request.frame_contract,
            size: request.size,
        }
    }
}

/// One captured picture waiting for the encoding stage.
pub(super) struct Captured {
    /// CPU pixels, or a picture that stays on the GPU for a GPU encoder.
    image: Picture,
    /// When the capture call ended: an older picture is captured anew.
    at: Instant,
    frame_id: u64,
    capture_ts_us: u64,
    capture_us: u64,
    serial: u64,
    grant: Option<CaptureGrant>,
    limits: FrameLimits,
    /// The source's reference before the capture: the picture's.
    generation: u64,
    /// The source's facts after the capture (layout, limits).
    reference: super::run::Reference,
}

/// What one capture call gave.
enum Tick {
    Picture(Captured),
    /// Nothing new: the answer for open credit after this tick.
    Unchanged(WorkerReply),
    /// Stands for the next credit; `true` ends the stream (a failure).
    Event(WorkerReply, bool),
    /// The source changed under the stream: encoder and pictures go.
    Turnover(WorkerReply),
}

/// What the capture stage needs for one tick.
struct CaptureJob {
    request: StageRequest,
    serial: u64,
    interval: Duration,
}

/// The state both stages and the order share.
#[derive(Default)]
pub(super) struct State {
    started: bool,
    stopping: bool,
    /// Control requests that need the stages at rest.
    quiet: u32,
    capturing: bool,
    /// The encoding stage encodes or delivers outside the lock.
    encoding: bool,
    order: Option<StreamOrder>,
    identity: Option<Identity>,
    /// Changes with the identity and at rest: older work belongs to no one.
    serial: u64,
    granted: u64,
    answered: u64,
    key_seen: u64,
    key_wanted: bool,
    /// B-36: the newest recovery wish seen, and the open one (its serial
    /// and the confirmed frame) until a picture answers it.
    recover_seen: u64,
    recover: Option<(u64, u64)>,
    /// Frame id of the next capture (the test picture draws it).
    next_id: u64,
    last_order: Option<Instant>,
    slot: Option<Captured>,
    held: Option<encode::Held>,
    event: Option<(WorkerReply, bool)>,
    unchanged: Option<WorkerReply>,
    reset_encoder: bool,
    /// The encoder needs one repeated capture after motion stops.
    needs_refresh: bool,
    /// The stream ended with a failure until the next identity.
    failed: bool,
    /// Pictures captured and dropped before encoding so far.
    dropped: u64,
}

/// Pictures that leave the state; dropped outside its lock, because their
/// release can write to the daemon.
type Gone = (Option<Captured>, Option<encode::Held>);

impl State {
    /// One interval of the order's cadence.
    fn interval(&self) -> Duration {
        let fps = self
            .order
            .as_ref()
            .map_or(1, |order| order.cadence_fps.max(1));
        Duration::from_micros(1_000_000 / u64::from(fps))
    }

    /// Whether the waiting picture is recent enough to encode: at most two
    /// intervals old, else the next capture replaces it first.
    fn slot_fresh(&self) -> bool {
        let limit = self.interval() * 2;
        self.slot
            .as_ref()
            .is_some_and(|captured| captured.at.elapsed() <= limit)
    }

    /// A capture tick when the stream wants one: while credit is open, while
    /// a picture may be encoded ahead, or (shortly after an order) until one
    /// picture waits. Without any of them the capture stage rests and the
    /// waiting picture stays the newest one captured.
    fn capture_job(&self) -> Option<CaptureJob> {
        let order = self.order.as_ref()?;
        if self.quiet > 0 || self.failed {
            return None;
        }
        let credit = self.answered < self.granted;
        let recent = self.last_order.is_some_and(|at| at.elapsed() < ORDER_IDLE);
        let ahead = order.ahead > 0 && self.held.is_none();
        // A fresh picture the idle encoding stage is about to take stays.
        if (credit || ahead) && !self.encoding && self.slot_fresh() {
            return None;
        }
        let wanted = credit || (recent && (ahead || self.slot.is_none()));
        wanted.then(|| CaptureJob {
            request: order.request.clone(),
            serial: self.serial,
            interval: self.interval(),
        })
    }

    /// Takes the result of a capture tick begun under `serial`.
    fn store(&mut self, tick: Tick, serial: u64) -> Gone {
        if self.serial != serial || self.quiet > 0 || self.stopping {
            return match tick {
                Tick::Picture(captured) => (Some(captured), None),
                _ => (None, None),
            };
        }
        // The newest tick decides what a pending pause says; a failure stays.
        if self.event.as_ref().is_some_and(|(_, fatal)| !fatal) {
            self.event = None;
        }
        self.unchanged = None;
        match tick {
            Tick::Picture(captured) => {
                let gone = self.slot.replace(captured);
                if gone.is_some() {
                    self.dropped += 1;
                }
                (gone, None)
            }
            Tick::Unchanged(reply) => {
                self.unchanged = Some(reply);
                (None, None)
            }
            Tick::Event(reply, fatal) => {
                self.failed |= fatal;
                self.event = Some((reply, fatal));
                (None, None)
            }
            Tick::Turnover(reply) => {
                self.reset_encoder = true;
                self.key_wanted = true;
                self.event = Some((reply, false));
                (self.slot.take(), self.held.take())
            }
        }
    }
}

pub(super) struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    worker: Arc<Mutex<Worker>>,
    output: dispatch::Output,
    remote: Arc<dispatch::RemoteAuthority>,
    opener: Arc<dyn EncoderOpener>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>, limit: Duration) -> MutexGuard<'a, State> {
        match self.changed.wait_timeout(state, limit) {
            Ok((state, _)) => state,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }
}

/// The picture stream of one host process.
pub(super) struct Stream {
    shared: Arc<Shared>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

/// The stages rest while this lives (`Stream::quiesce`).
pub(super) struct Quiet<'a> {
    shared: &'a Shared,
}

impl Drop for Quiet<'_> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.quiet = state.quiet.saturating_sub(1);
        self.shared.changed.notify_all();
    }
}

impl Stream {
    pub(super) fn new(
        worker: Arc<Mutex<Worker>>,
        output: dispatch::Output,
        remote: Arc<dispatch::RemoteAuthority>,
        opener: Arc<dyn EncoderOpener>,
    ) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
                worker,
                output,
                remote,
                opener,
            }),
            threads: Mutex::new(Vec::new()),
        })
    }

    /// Takes an order: new settings, credit, a key frame wish. The first
    /// order starts both stages.
    pub(super) fn order(&self, order: StreamOrder) {
        let (start, gone) = {
            let mut state = self.shared.lock();
            if state.stopping {
                return;
            }
            let mut gone: Gone = (None, None);
            let identity = Identity::of(&order.request);
            if state.identity != Some(identity) {
                state.identity = Some(identity);
                state.serial += 1;
                state.reset_encoder = true;
                state.failed = false;
                state.event = None;
                state.unchanged = None;
                gone = (state.slot.take(), state.held.take());
                if gone.0.is_some() {
                    state.dropped += 1;
                }
            }
            if order.key_serial > state.key_seen {
                state.key_seen = order.key_serial;
                state.key_wanted = true;
                if state.held.as_ref().is_some_and(|held| !held.keyframe) {
                    gone.1 = state.held.take();
                }
            }
            // B-36: a picture encoded before the wish references lost frames.
            if order.recover_serial > state.recover_seen {
                state.recover_seen = order.recover_serial;
                state.recover = Some((order.recover_serial, order.last_good));
                if state.held.as_ref().is_some_and(|held| !held.keyframe) {
                    gone.1 = state.held.take();
                }
            }
            if order.granted > state.granted {
                state.granted = order.granted;
                // The new credit gets the result of a tick after it.
                state.unchanged = None;
            }
            // A held picture answers this credit; captures after it follow.
            let next = order
                .request
                .frame_id
                .saturating_add(u64::from(state.held.is_some()));
            state.next_id = state.next_id.max(next);
            state.order = Some(order);
            state.last_order = Some(Instant::now());
            let start = !state.started;
            state.started = true;
            self.shared.changed.notify_all();
            (start, gone)
        };
        drop(gone);
        if start {
            self.spawn();
        }
    }

    fn spawn(&self) {
        let stages: [(&str, fn(Arc<Shared>)); 2] = [
            ("screen-worker-capture", capture::capture_loop),
            ("screen-worker-encode", encode::encode_loop),
        ];
        let mut threads = self.threads.lock().unwrap_or_else(PoisonError::into_inner);
        for (name, stage) in stages {
            let shared = self.shared.clone();
            match std::thread::Builder::new()
                .name(name.into())
                .spawn(move || stage(shared))
            {
                Ok(thread) => threads.push(thread),
                Err(error) => {
                    // Without its stages the stream answers its credit with
                    // the failure and ends; the daemon ends the session.
                    eprintln!("screen host process: {error}");
                    encode::fail(&self.shared, failed(ScreenErrorCode::CaptureFailed, error));
                    return;
                }
            }
        }
    }

    /// Rests both stages until the guard goes: a running capture or
    /// encoding ends first, a waiting picture and one encoded ahead go (the
    /// next picture is then a key frame), with `reset_encoder` the encoder
    /// as well.
    pub(super) fn quiesce(&self, reset_encoder: bool) -> Quiet<'_> {
        let gone: Gone = {
            let mut state = self.shared.lock();
            state.quiet += 1;
            self.shared.changed.notify_all();
            while state.capturing || state.encoding {
                state = self.shared.wait(state, RECHECK);
            }
            state.serial += 1;
            state.reset_encoder |= reset_encoder;
            let held = state.held.take();
            state.key_wanted |= held.is_some();
            (state.slot.take(), held)
        };
        drop(gone);
        Quiet {
            shared: &self.shared,
        }
    }

    /// Ends both stages (`CLOSE`); their pictures go before the worker.
    pub(super) fn stop(&self) {
        {
            let mut state = self.shared.lock();
            state.stopping = true;
            self.shared.changed.notify_all();
        }
        let threads =
            std::mem::take(&mut *self.threads.lock().unwrap_or_else(PoisonError::into_inner));
        for thread in threads {
            let _ = thread.join();
        }
        let gone: Gone = {
            let mut state = self.shared.lock();
            (state.slot.take(), state.held.take())
        };
        drop(gone);
    }

    #[cfg(test)]
    fn shared(&self) -> &Shared {
        &self.shared
    }
}

#[cfg(test)]
#[path = "bildkette_w4_fallback_task_tests.rs"]
mod bildkette_w4_fallback_task_tests;
#[cfg(test)]
#[path = "bildkette_chain_task_tests.rs"]
mod chain_tests;
