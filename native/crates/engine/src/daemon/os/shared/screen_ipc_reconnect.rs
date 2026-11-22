//! The relayed viewer session and its reconnection after a session switch
//! on the host (E48). A host that follows the active session of its device
//! (E23) ends a session whose session is no longer the active one with the
//! reason `session_changed` (`EndReason::SessionChanged`). The relay does
//! not pass that end to its client: it builds the same session again, with
//! a new connection to the same device (`ShareService::screen_link`), the
//! same node pin, the same request and the same signed handshake
//! (`session_memory::handshake`), so the host checks the right exactly as
//! for a new session. It tries at most once per second and for as long as
//! the attempts make progress (plan 2026-11-17 P9: no total limit): 30 s
//! after the attempt that got further than every attempt before it (the
//! device reached, then its host answered), also when a new session ends at
//! once with `session_changed` again (one plan, `SETTLED_AFTER`). While the
//! right holds it tries on: a host that is not reached or whose new session
//! is not ready yet (`NOT_READY`) is tried again; any other refusal ends the
//! session with the refusal's code, and without progress for 30 s the
//! session ends visibly with `session_changed`.
//!
//! P9 („das Verbinden zum Remote-Bildschirm schlägt regelmäßig fehl“): the
//! first opening of a session follows the same plan. A first handshake that
//! fails in a way that may pass (the device not reached, the host's session
//! or capture not ready yet) is tried again with a new connection instead of
//! ending the viewer's attempt at once; when the attempts stop making
//! progress the session ends with the last failure's code, its step
//! (connection, handshake, host) and the number of attempts.
//! The first accepted hello stays the client's (it takes no second one):
//! the new session's frames and pictures follow on the same relay when its
//! frame contract is the one the client accepted (`reads`), else the session
//! ends with `session_changed`. Control frames of the client that arrive
//! while the session is built again are dropped; the new session is put
//! where the viewer left the old one by replaying what its `SET_MODE` frames
//! chose (`ModeMemory`: source, raise from the group view, picture size) and
//! its pointer wish. Any other end is passed on as before.
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::daemon::ipc_protocol::screen::{
    RateCeilingState, RelayEnd, RelayHello, RELAY_AUDIO, RELAY_END, RELAY_HELLO,
};
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
/// Longest time the relay tries without progress (P9: counted from the
/// last attempt that got further than all before it).
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
pub(super) use mode_memory::{ModeMemory, RelayedFrame};
#[path = "screen_ipc_retry_plan.rs"]
mod retry_plan;
use retry_plan::retryable;
#[cfg_attr(not(test), allow(unused_imports))]
pub(super) use retry_plan::{again, Clock, Dial, Outcome, Pending, Schedule, Step, SystemClock};

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

/// The relay event of a `RATE_CEILING` (`RELAY_RATE_CEILING`); `None` for
/// every other frame.
pub(super) fn rate_state(frame: &ViewerFrame, host_takes_it: bool) -> Option<Vec<u8>> {
    let ViewerFrame::RateCeiling(ceiling) = frame else {
        return None;
    };
    wire::to_json(&RateCeilingState::of(*ceiling, host_takes_it)).ok()
}

/// The dialer of the relay: a new link to the same device, the node pin,
/// the handshake.
struct LinkDialer {
    service: ShareService,
    target: PeerOpenTarget,
    expected_node: Option<String>,
    request: ViewerRequest,
    metadata: bool,
    /// The viewer hears of a confirmation (first opening only).
    pending: Option<mpsc::Sender<(u8, Vec<u8>)>>,
}

/// `error` of the step `step` (P9: the viewer reads where it failed).
fn at_step(step: &str, error: ViewerError) -> ViewerError {
    match error {
        ViewerError::Io(error) => {
            ViewerError::Io(io::Error::new(error.kind(), format!("{step}: {error}")))
        }
        other => other,
    }
}

impl Dial for LinkDialer {
    type Session = ViewerSession;
    fn dial(&self, remaining: Duration) -> Pending<Result<ViewerSession, ViewerError>> {
        let service = self.service.clone();
        let target = self.target.clone();
        let expected = self.expected_node.clone();
        let request = self.request.clone();
        let metadata = self.metadata;
        let pending = self.pending.clone();
        Box::pin(async move {
            let opened = async move {
                let link = tokio::task::spawn_blocking(move || service.screen_link(&target))
                    .await
                    .map_err(|error| ViewerError::Io(io::Error::other(error)))?
                    .map_err(|error| at_step("Verbindung zum Gerät", error))?;
                if let Some(expected) = expected {
                    if link.connection.remote_id().to_string() != expected {
                        link.connection.close(
                            iroh::endpoint::VarInt::from_u32(PIN_CLOSE),
                            b"screen node pin changed",
                        );
                        return Err(ViewerError::Refused("screen_node_changed".into()));
                    }
                }
                open(link, &request, metadata, pending).await
            };
            tokio::time::timeout(remaining, opened)
                .await
                .unwrap_or_else(|_| {
                    Err(ViewerError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Handshake: the screen host did not answer in time",
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
    let step = |error| at_step("Handshake", error);
    let announce = |capability| {
        if let (Some(out), Ok(json)) =
            (&pending, wire::to_json(&RelayHello::Pending { capability }))
        {
            let _ = out.try_send((RELAY_HELLO, json));
        }
    };
    if metadata {
        crate::screen::core::viewer_client::handshake_metadata(connection, build, announce)
            .await
            .map_err(step)
    } else {
        handshake(connection, build, announce).await.map_err(step)
    }
}

async fn end(out: &mpsc::Sender<(u8, Vec<u8>)>, code: String, detail: Option<String>) {
    if let Ok(json) = wire::to_json(&RelayEnd { code, detail }) {
        let _ = out.send((RELAY_END, json)).await;
    }
}

/// What the viewer chose since the first opening, sent to a session built
/// again (the new one starts from the first request): its modes, ceiling,
/// decoders, display, sound, and (Gemini 3/9) its control and visibility.
async fn replay(modes: &ModeMemory, next: &ViewerSession) {
    // Codex 11/4: a display choice older than the last source goes first,
    // so it never undoes a later window choice.
    let display = modes.display();
    if let Some(choice) = display.filter(|_| !modes.display_last()) {
        let _ = next.control.send(ViewerFrame::SetDisplay(choice)).await;
    }
    for mode in modes.replay() {
        let _ = next.control.send(ViewerFrame::SetMode(mode)).await;
    }
    if let Some(ceiling) = modes.rate_ceiling() {
        let _ = next.control.send(ViewerFrame::RateCeiling(ceiling)).await;
    }
    // The decoders of the viewer's last „Codec“/„Dekodierer“ (P1): the new
    // session opened with the first request's.
    if let Some(set) = modes.decoders() {
        let _ = next.control.send(ViewerFrame::SetDecoders(set)).await;
    }
    // „Monitor“/„Auflösung“ chosen in the session (P9); the viewer task drops
    // it for a host without `live`.
    if let Some(choice) = display.filter(|_| modes.display_last()) {
        let _ = next.control.send(ViewerFrame::SetDisplay(choice)).await;
    }
    // The host's sound (P9), only to a host that answered it.
    if let Some(on) = modes.audio().filter(|_| next.hello.audio.is_some()) {
        let _ = next.control.send(ViewerFrame::Audio(on)).await;
    }
    // A control the viewer released stays released, a taken one is taken.
    if let Some(action) = modes.control() {
        let request = crate::screen::core::wire_control::ControlRequest { action };
        let _ = next
            .control
            .send(ViewerFrame::ControlRequest(request))
            .await;
    }
    // The visible area, to a host that reads it (with a frame contract,
    // `host_server.rs`, as the first session's did).
    if let Some(report) = modes.visibility() {
        if next.hello.frame_contract.is_some() {
            let _ = next.control.send(ViewerFrame::Visibility(report)).await;
        }
    }
}

/// The request of a session built again: the ceiling (Gemini 5/10) and the
/// decoders with their „Codec“ (round 8 B4) as the viewer last chose them go
/// in the hello itself, so the new session never starts above the ceiling
/// nor with an encoder of the first request's codec that `SET_DECODERS`
/// (`replay`, unchanged for the host then) would replace at once.
fn current(modes: &ModeMemory, first: &ViewerRequest, opening: Option<u64>) -> ViewerRequest {
    let mut request = first.clone();
    request.max_rate_bps = ceiling(modes, opening);
    // Codex 11/4: the source and display in force, not the first ones (the
    // first source may be gone; `replay` follows with the same choices).
    if let Some(source) = modes.source() {
        request.source = Some(source);
    }
    if let Some(choice) = modes.display().filter(|_| modes.display_last()) {
        request.virtual_display = Some(choice);
    }
    if let Some(set) = modes.decoders() {
        request.decoders = set.decoders;
        request.gop_continue = set.gop_continue;
        request.gop_continue_decoders = set.gop_continue_decoders;
        request.codec_choice = set.codec_choice.or(first.codec_choice);
    }
    request
}

/// „Höchste Datenrate“ as the viewer last chose it (`opening`: the one of
/// the first request).
fn ceiling(modes: &ModeMemory, opening: Option<u64>) -> Option<u64> {
    modes.rate_ceiling().unwrap_or(opening)
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
    mut input: mpsc::Receiver<Option<RelayedFrame>>,
    metadata: bool,
    memory: Arc<Mutex<MemoryHold>>,
    max_rate_bps: Option<u64>,
) {
    // The viewer's choices for a session built again (`ModeMemory`), also
    // those made while the first opening is tried again (Gemini 3/9).
    let mut modes = ModeMemory::default();
    let mut session = match open(link, &request, metadata, Some(out.clone())).await {
        Ok(session) => session,
        // P9: a first opening that may pass is tried again while it makes
        // progress, with a new connection each time.
        Err(error) if retryable(&error) => {
            let dialer = LinkDialer {
                service: redial.service.clone(),
                target: redial.target.clone(),
                expected_node: redial.expected_node.clone(),
                request: request.clone(),
                metadata,
                pending: Some(out.clone()),
            };
            let now = Instant::now();
            let mut schedule = Schedule::new(now);
            schedule.failed(now, &error);
            match again(dialer, SystemClock, &mut schedule, &mut input, &mut modes).await {
                Outcome::Opened(session) => {
                    replay(&modes, &session).await;
                    session
                }
                Outcome::Refused(error) => {
                    end(&out, error.code(), Some(error.detail())).await;
                    return;
                }
                Outcome::GaveUp => {
                    let (code, detail) = schedule
                        .gave_up()
                        .unwrap_or_else(|| (error.code(), error.detail()));
                    end(&out, code, Some(detail)).await;
                    return;
                }
                Outcome::ViewerClosed => {
                    end(&out, EndReason::ViewerClosed.key().into(), None).await;
                    return;
                }
            }
        }
        Err(error) => {
            end(&out, error.code(), Some(error.detail())).await;
            return;
        }
    };
    hold(&memory, &session);
    // „Höchste Datenrate“ (B-09): `RATE_CEILING` only to a host that
    // announced `rate_ceiling_v1`; the viewer hears what became of it.
    // Round 11: the ceiling as last chosen, also while the first opening
    // was tried again (`again` notes the viewer's frames in `modes`).
    let chosen = ceiling(&modes, max_rate_bps);
    if chosen.is_some() && !metadata {
        // Codex 10/7: a host that cannot keep the chosen ceiling shows no
        // picture without it; the viewer reads why.
        if !session.rate_ceiling {
            session.close();
            let detail = crate::daemon::ipc_protocol::screen::RATE_CEILING_UNSUPPORTED;
            end(&out, EndReason::Failed.key().into(), Some(detail.into())).await;
            return;
        }
        let frame = ViewerFrame::RateCeiling(chosen);
        modes.note(&frame);
        if session.rate_ceiling {
            let _ = session.control.send(frame).await;
        }
    }
    let contract = session.hello.frame_contract;
    let path = path_of(&session.connection).0.key().to_string();
    let accepted = RelayHello::Accepted {
        hello: session.hello.clone(),
        path,
        local_limits: session.local_limits,
        rate_ceiling: RateCeilingState::of(chosen, session.rate_ceiling),
        set_decoders: session.set_decoders,
    };
    if let Ok(json) = wire::to_json(&accepted) {
        if out.send((RELAY_HELLO, json)).await.is_err() {
            session.close();
            return;
        }
    }
    let mut plan: Option<Schedule> = None;
    let mut opened = Instant::now();
    loop {
        // The host's sound (P9) reaches the viewer beside the relay loop.
        let audio = crate::screen::os::audio::relay(&session.connection, &out, RELAY_AUDIO);
        let (code, detail) = super::relay_loop(&mut session, &out, &mut input, &mut modes).await;
        drop(audio);
        session.close();
        if code != EndReason::SessionChanged.key() {
            end(&out, code, detail).await;
            return;
        }
        let dialer = LinkDialer {
            service: redial.service.clone(),
            target: redial.target.clone(),
            expected_node: redial.expected_node.clone(),
            request: current(&modes, &request, max_rate_bps),
            metadata,
            pending: None,
        };
        // A session that changed again at once keeps the plan it was built
        // by: one attempt per second and 30 s in all, whatever the host does.
        let now = Instant::now();
        let settled = now.saturating_duration_since(opened) >= SETTLED_AFTER;
        let mut schedule = plan
            .take()
            .filter(|_| !settled)
            .unwrap_or_else(|| Schedule::new(now));
        let outcome = again(dialer, SystemClock, &mut schedule, &mut input, &mut modes).await;
        plan = Some(schedule);
        match outcome {
            Outcome::Opened(next) if !reads(contract, next.hello.frame_contract) => {
                next.close();
                end(&out, code, None).await;
                return;
            }
            // Round 11: a session built again on a host that cannot keep the
            // chosen ceiling ends as at the opening (Codex 10/7).
            Outcome::Opened(next)
                if ceiling(&modes, max_rate_bps).is_some() && !metadata && !next.rate_ceiling =>
            {
                next.close();
                let detail = crate::daemon::ipc_protocol::screen::RATE_CEILING_UNSUPPORTED;
                end(&out, EndReason::Failed.key().into(), Some(detail.into())).await;
                return;
            }
            Outcome::Opened(next) => {
                replay(&modes, &next).await;
                if session
                    .cursor_wish
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    let _ = next.control.send(ViewerFrame::CursorOverlay(true)).await;
                }
                session = next;
                opened = Instant::now();
                hold(&memory, &session);
                // The new host session counts its picture groups and frames
                // from one again (Gemini 3/9.4): the viewer hears the hello
                // once more and starts a new picture epoch with it.
                let path = path_of(&session.connection).0.key().to_string();
                let again = RelayHello::Accepted {
                    hello: session.hello.clone(),
                    path,
                    local_limits: session.local_limits,
                    // The ceiling now, not the opening's (Gemini 5/10).
                    rate_ceiling: RateCeilingState::of(
                        ceiling(&modes, max_rate_bps),
                        session.rate_ceiling,
                    ),
                    set_decoders: session.set_decoders,
                };
                if let Ok(json) = wire::to_json(&again) {
                    if out.send((RELAY_HELLO, json)).await.is_err() {
                        session.close();
                        return;
                    }
                }
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
