//! The relayed viewer session and its reconnection after a session switch
//! on the host (E48). A host that follows the active session of its device
//! (E23) ends a session whose session is no longer the active one with the
//! reason `session_changed` (`EndReason::SessionChanged`). The relay does
//! not pass that end to its client: it builds the same session again, with
//! a new connection to the same device (`ShareService::screen_link`), the
//! same node pin, the same request and the same signed handshake
//! (`session_memory::handshake`), so the host checks the right exactly as
//! for a new session. It tries at most once per second and for at most
//! 30 s, also when a new session ends at once with `session_changed` again
//! (one plan, `SETTLED_AFTER`). While the right holds it tries on: a host
//! that is not reached or whose new session is not ready yet (`NOT_READY`)
//! is tried again; any other refusal ends the session with the refusal's
//! code, and after 30 s the session ends visibly with `session_changed`.
//! The first accepted hello stays the client's (it takes no second one):
//! the new session's frames and pictures follow on the same relay when its
//! frame contract is the one the client accepted (`reads`), else the session
//! ends with `session_changed`. Control frames of the client that arrive
//! while the session is built again are dropped; the new session is put
//! where the viewer left the old one by replaying what its `SET_MODE` frames
//! chose (`ModeMemory`: source, raise from the group view, picture size) and
//! its pointer wish. Any other end is passed on as before.
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::daemon::ipc_protocol::screen::{RelayEnd, RelayHello, RELAY_END, RELAY_HELLO};
use crate::screen::core::frame_budget::FrameContract;
use crate::screen::core::frame_memory::MemoryHold;
use crate::screen::core::gop_stream::path_of;
use crate::screen::core::session_state::{EndReason, SessionState};
use crate::screen::core::viewer_client::{ViewerError, ViewerRequest, ViewerSession};
use crate::screen::core::wire;
use crate::screen::core::wire_control::{HostFrame, ViewerFrame};
use crate::screen::core::wire_hello::ScreenErrorCode;
use crate::screen::os::session_memory::handshake;
use crate::share::node_screen::ScreenLink;
use crate::share::{PeerOpenTarget, ShareService};

/// Least time between two attempts.
pub(super) const RECONNECT_EVERY: Duration = Duration::from_secs(1);
/// Longest time the relay tries after a session switch.
pub(super) const RECONNECT_FOR: Duration = Duration::from_secs(30);
/// A session built again that ends with `session_changed` sooner than this
/// continues the plan of its reconnection instead of starting a new one.
const SETTLED_AFTER: Duration = Duration::from_secs(5);
/// Refusals of a host whose new session is not ready yet (no session
/// resolved, no indicator, capture not up, host process of the old session
/// ended): no right is refused, so the attempt is repeated.
const NOT_READY: [ScreenErrorCode; 4] = [
    ScreenErrorCode::NoDisplay,
    ScreenErrorCode::IndicatorUnavailable,
    ScreenErrorCode::CaptureFailed,
    ScreenErrorCode::HostShutdown,
];
/// Close code of a connection whose node is not the pinned one.
const PIN_CLOSE: u32 = 0x5345;

#[path = "screen_ipc_mode_memory.rs"]
mod mode_memory;
pub(super) use mode_memory::ModeMemory;

#[cfg(test)]
#[path = "bildschirm_wechsel_task_tests.rs"]
mod wechsel_tests;

/// What a new connection to the same device needs.
pub(super) struct Redial {
    pub service: ShareService,
    pub target: PeerOpenTarget,
    /// The node pin of the first connection, checked again.
    pub expected_node: Option<String>,
}

/// Whether `frame` is the host's end of a session because its session is no
/// longer the active one.
pub(super) fn changed_session(frame: &HostFrame) -> bool {
    matches!(
        frame,
        HostFrame::State(report)
            if report.state == SessionState::Ended
                && report.reason.as_deref() == Some(EndReason::SessionChanged.key())
    )
}

/// What the reconnection does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// One attempt now, with at most `remaining` for it.
    Attempt { remaining: Duration },
    /// The next attempt is due after this time.
    Wait(Duration),
    /// The 30 s are over.
    GiveUp,
}

/// The time plan of one reconnection: at most one attempt per
/// `RECONNECT_EVERY`, none after `RECONNECT_FOR`.
pub(super) struct Schedule {
    start: Instant,
    next: Instant,
}

impl Schedule {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            start: now,
            next: now,
        }
    }

    pub(super) fn step(&mut self, now: Instant) -> Step {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= RECONNECT_FOR {
            return Step::GiveUp;
        }
        if now < self.next {
            return Step::Wait(self.next - now);
        }
        self.next = now + RECONNECT_EVERY;
        Step::Attempt {
            remaining: RECONNECT_FOR - elapsed,
        }
    }
}

/// A future the reconnection waits for.
pub(super) type Pending<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The clock of the reconnection (a simulated one in the task tests).
pub(super) trait Clock: Send {
    fn now(&self) -> Instant;
    fn sleep(&self, time: Duration) -> Pending<()>;
}

/// The clock of the system.
pub(super) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, time: Duration) -> Pending<()> {
        Box::pin(tokio::time::sleep(time))
    }
}

/// One new connection and handshake to the same device.
pub(super) trait Dial: Send {
    type Session: Send + 'static;
    fn dial(&self, remaining: Duration) -> Pending<Result<Self::Session, ViewerError>>;
}

/// How a reconnection ended.
pub(super) enum Outcome<S> {
    Opened(S),
    Refused(ViewerError),
    GaveUp,
    ViewerClosed,
}

/// Whether a failed attempt may be repeated: the device was not reached, or
/// its new session is not ready yet (`NOT_READY`). Any other refusal of the
/// host is final.
fn retryable(error: &ViewerError) -> bool {
    match error {
        ViewerError::Io(_) => true,
        ViewerError::Refused(code) => NOT_READY.iter().any(|ready| ready.key() == code.as_str()),
        ViewerError::Unsupported => false,
    }
}

/// Waits for `wait`; `None` when the client closed meanwhile. Control frames
/// of the client arriving now belong to the ended session and are dropped.
async fn unless_closed<T>(
    mut wait: Pending<T>,
    input: &mut mpsc::Receiver<Option<ViewerFrame>>,
) -> Option<T> {
    loop {
        tokio::select! {
            biased;
            frame = input.recv() => {
                if !matches!(frame, Some(Some(_))) {
                    return None;
                }
            }
            value = &mut wait => return Some(value),
        }
    }
}

/// Builds the session again after `session_changed` by the plan `schedule`:
/// one dial per attempt.
pub(super) async fn again<D: Dial, C: Clock>(
    dialer: D,
    clock: C,
    schedule: &mut Schedule,
    input: &mut mpsc::Receiver<Option<ViewerFrame>>,
) -> Outcome<D::Session> {
    loop {
        match schedule.step(clock.now()) {
            Step::GiveUp => return Outcome::GaveUp,
            Step::Wait(time) => {
                if unless_closed(clock.sleep(time), input).await.is_none() {
                    return Outcome::ViewerClosed;
                }
            }
            Step::Attempt { remaining } => {
                match unless_closed(dialer.dial(remaining), input).await {
                    None => return Outcome::ViewerClosed,
                    Some(Ok(session)) => return Outcome::Opened(session),
                    Some(Err(error)) if retryable(&error) => {}
                    Some(Err(error)) => return Outcome::Refused(error),
                }
            }
        }
    }
}

/// The dialer of the relay: a new link to the same device, the node pin,
/// the handshake.
struct LinkDialer {
    service: ShareService,
    target: PeerOpenTarget,
    expected_node: Option<String>,
    request: ViewerRequest,
    metadata: bool,
}

impl Dial for LinkDialer {
    type Session = ViewerSession;
    fn dial(&self, remaining: Duration) -> Pending<Result<ViewerSession, ViewerError>> {
        let service = self.service.clone();
        let target = self.target.clone();
        let expected = self.expected_node.clone();
        let request = self.request.clone();
        let metadata = self.metadata;
        Box::pin(async move {
            let opened = async move {
                let link = tokio::task::spawn_blocking(move || service.screen_link(&target))
                    .await
                    .map_err(|error| ViewerError::Io(io::Error::other(error)))??;
                if let Some(expected) = expected {
                    if link.connection.remote_id().to_string() != expected {
                        link.connection.close(
                            iroh::endpoint::VarInt::from_u32(PIN_CLOSE),
                            b"screen node pin changed",
                        );
                        return Err(ViewerError::Refused("screen_node_changed".into()));
                    }
                }
                open(link, &request, metadata, None).await
            };
            tokio::time::timeout(remaining, opened)
                .await
                .unwrap_or_else(|_| {
                    Err(ViewerError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the screen host did not answer after the session switch",
                    )))
                })
        })
    }
}

/// The signed handshake over `link`; with `pending` the client learns of a
/// confirmation the device's user answers (first opening only).
async fn open(
    link: ScreenLink,
    request: &ViewerRequest,
    metadata: bool,
    pending: Option<mpsc::Sender<(u8, Vec<u8>)>>,
) -> Result<ViewerSession, ViewerError> {
    let connection = link.connection.clone();
    let build = |server: &_| link.client_hello(server, request);
    let announce = |capability| {
        if let (Some(out), Ok(json)) =
            (&pending, wire::to_json(&RelayHello::Pending { capability }))
        {
            let _ = out.try_send((RELAY_HELLO, json));
        }
    };
    if metadata {
        crate::screen::core::viewer_client::handshake_metadata(connection, build, announce).await
    } else {
        handshake(connection, build, announce).await
    }
}

async fn end(out: &mpsc::Sender<(u8, Vec<u8>)>, code: String, detail: Option<String>) {
    if let Ok(json) = wire::to_json(&RelayEnd { code, detail }) {
        let _ = out.send((RELAY_END, json)).await;
    }
}

fn hold(memory: &Mutex<MemoryHold>, session: &ViewerSession) {
    *memory.lock().unwrap_or_else(PoisonError::into_inner) = session.memory.clone();
}

/// Whether the client, which keeps the frame contract `first` of the hello
/// it accepted (`ScreenStream::next_event`), reads pictures under `next`.
fn reads(first: Option<FrameContract>, next: Option<FrameContract>) -> bool {
    match (next, first) {
        (None, None) => true,
        (Some(next), Some(first)) => {
            next.version == first.version
                && next.limits == first.limits
                && (!next.hybrid_regions || first.hybrid_regions)
        }
        _ => false,
    }
}

/// Runs the relayed session on the share runtime.
pub(super) async fn relay_session(
    link: ScreenLink,
    redial: Redial,
    request: ViewerRequest,
    out: mpsc::Sender<(u8, Vec<u8>)>,
    mut input: mpsc::Receiver<Option<ViewerFrame>>,
    metadata: bool,
    memory: Arc<Mutex<MemoryHold>>,
) {
    let mut session = match open(link, &request, metadata, Some(out.clone())).await {
        Ok(session) => session,
        Err(error) => {
            end(&out, error.code(), Some(error.detail())).await;
            return;
        }
    };
    hold(&memory, &session);
    let contract = session.hello.frame_contract;
    let path = path_of(&session.connection).0.key().to_string();
    let accepted = RelayHello::Accepted {
        hello: session.hello.clone(),
        path,
        local_limits: session.local_limits,
    };
    if let Ok(json) = wire::to_json(&accepted) {
        if out.send((RELAY_HELLO, json)).await.is_err() {
            session.close();
            return;
        }
    }
    let mut plan: Option<Schedule> = None;
    let mut opened = Instant::now();
    let mut modes = ModeMemory::default();
    loop {
        let (code, detail) = super::relay_loop(&mut session, &out, &mut input, &mut modes).await;
        session.close();
        if code != EndReason::SessionChanged.key() {
            end(&out, code, detail).await;
            return;
        }
        let dialer = LinkDialer {
            service: redial.service.clone(),
            target: redial.target.clone(),
            expected_node: redial.expected_node.clone(),
            request: request.clone(),
            metadata,
        };
        // A session that changed again at once keeps the plan it was built
        // by: one attempt per second and 30 s in all, whatever the host does.
        let now = Instant::now();
        let settled = now.saturating_duration_since(opened) >= SETTLED_AFTER;
        let mut schedule = plan
            .take()
            .filter(|_| !settled)
            .unwrap_or_else(|| Schedule::new(now));
        let outcome = again(dialer, SystemClock, &mut schedule, &mut input).await;
        plan = Some(schedule);
        match outcome {
            Outcome::Opened(next) if !reads(contract, next.hello.frame_contract) => {
                next.close();
                end(&out, code, None).await;
                return;
            }
            Outcome::Opened(next) => {
                // What the viewer chose after the first opening (the new
                // session starts from the first request), then the pointer
                // overlay (P6), a wish of the viewer, not of the host.
                for mode in modes.replay() {
                    let _ = next.control.send(ViewerFrame::SetMode(mode)).await;
                }
                if session
                    .cursor_wish
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    let _ = next.control.send(ViewerFrame::CursorOverlay(true)).await;
                }
                session = next;
                opened = Instant::now();
                hold(&memory, &session);
            }
            Outcome::Refused(error) => {
                end(&out, error.code(), Some(error.detail())).await;
                return;
            }
            Outcome::GaveUp => {
                end(&out, code, None).await;
                return;
            }
            Outcome::ViewerClosed => {
                end(&out, EndReason::ViewerClosed.key().into(), None).await;
                return;
            }
        }
    }
}
