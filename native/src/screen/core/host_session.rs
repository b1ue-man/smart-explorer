//! The running part of one host session (SCR-M2, decisions C3, C5): after
//! `HELLO_OK` the session loop takes the viewer's control frames from a
//! reader task, lets the pipeline produce pictures at the agreed rate,
//! reports `STATE` and `STATS` once a second and watches the stop signal of
//! the registry (host user, rights change, end of the grant). Input passes
//! only while this session holds the control; a released control lets go of
//! every pressed key and button. At the end the viewer gets the `ERROR` (if
//! any) and `STATE` `ended` with the reason, the access log gets
//! `session_ended` with reason, duration and bytes, and the connection
//! closes.
use std::io;
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream, VarInt};
use tokio::sync::mpsc;

use super::{HostContext, CLOSE_GRACE};
use crate::screen::core::authorize::log_event;
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_pipeline::{Pipeline, PipelineActor, Produced, WorkCompletion};
use crate::screen::core::input_model::{InputEvent, PressedInputs};
use crate::screen::core::registry::SessionHandle;
use crate::screen::core::session_state::{EndReason, SessionState};
use crate::screen::core::wire_binary::StreamKind;
use crate::screen::core::wire_control::{
    recv_viewer_frame, ControlAction, ControlState, HostFrame, Pong, ScreenWireError, SetMode,
    StateReport, StreamInfo, StreamMode, ViewerFrame,
};
use crate::screen::core::wire_hello::{ScreenErrorCode, SessionMode};
use crate::share::relation_proof::ProvenPrincipal;

#[path = "host_session_control.rs"]
mod control;

#[path = "host_family.rs"]
mod family;
#[path = "host_input_release.rs"]
mod input_release;
#[path = "host_visibility.rs"]
mod visibility;

/// A viewer that sends nothing (not even `PING`) this long has gone.
const VIEWER_SILENCE: Duration = Duration::from_secs(15);
/// Code of the connection close after a session.
const SESSION_CLOSE: VarInt = VarInt::from_u32(0);
/// Viewer frames queued ahead of the session loop.
const FRAME_QUEUE: usize = 64;

/// Reads the viewer's control frames into the session's queue.
async fn read_viewer_frames(mut recv: RecvStream, frames: mpsc::Sender<io::Result<ViewerFrame>>) {
    loop {
        let frame = recv_viewer_frame(&mut recv).await;
        let failed = frame.is_err();
        if frames.send(frame).await.is_err() || failed {
            return;
        }
    }
}

/// One session after `HELLO_OK`.
pub(super) struct Session<'a> {
    context: &'a HostContext,
    connection: &'a Connection,
    send: SendStream,
    handle: SessionHandle,
    principal: &'a ProvenPrincipal,
    mode: SessionMode,
    pipeline: PipelineActor,
    pressed: PressedInputs,
    input_release: Option<
        tokio::task::JoinHandle<Result<(), crate::screen::core::wire_control::ControlBlock>>,
    >,
    release_blocked: Option<crate::screen::core::wire_control::ControlBlock>,
    state: SessionState,
    reported_stream: Option<StreamInfo>,
    started: Instant,
    visibility_supported: bool,
    visibility_generation: u64,
    visibility_ack_pending: bool,
    visible: bool,
    input_floor: Option<u64>,
    observed_reference_floor: u64,
    icons_supported: bool,
    family: family::FamilyState,
}

/// How a session ended: the reason and the `ERROR` the viewer gets first.
type Ending = (EndReason, Option<ScreenWireError>);

/// The ending of a session whose viewer can no longer be written to.
const LOST: Ending = (EndReason::ConnectionLost, None);

impl<'a> Session<'a> {
    pub(super) fn new(
        context: &'a HostContext,
        connection: &'a Connection,
        send: SendStream,
        handle: SessionHandle,
        principal: &'a ProvenPrincipal,
        mode: SessionMode,
        pipeline: Pipeline,
        visibility_supported: bool,
        icons_supported: bool,
        families_supported: bool,
        family_source: Option<String>,
    ) -> Self {
        let pipeline = PipelineActor::new(pipeline, connection.clone());
        let observed_reference_floor = pipeline.reference_floor();
        Self {
            context,
            connection,
            send,
            handle,
            principal,
            mode,
            pipeline,
            pressed: PressedInputs::default(),
            input_release: None,
            release_blocked: None,
            state: SessionState::Starting,
            reported_stream: None,
            started: Instant::now(),
            visibility_supported,
            visibility_generation: 0,
            visibility_ack_pending: false,
            visible: true,
            input_floor: None,
            observed_reference_floor,
            icons_supported,
            family: family::FamilyState::new(families_supported, family_source),
        }
    }

    /// Runs the session until it ends; `max_fps` paces the capture ticks.
    pub(super) async fn run(mut self, recv: RecvStream, max_fps: u32) {
        let (frames_tx, mut frames) = mpsc::channel(FRAME_QUEUE);
        let reader = tokio::spawn(read_viewer_frames(recv, frames_tx));
        if self.family.selected.is_some() && self.family_snapshot(true).await.is_err() {
            reader.abort();
            self.finish((EndReason::Failed, None)).await;
            return;
        }
        let mut stop = self.handle.stop_signal();
        let tick = Duration::from_micros(1_000_000 / u64::from(max_fps.max(1)));
        let mut capture = tokio::time::interval(tick);
        capture.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut second = tokio::time::interval(Duration::from_secs(1));
        let mut heard = crate::share::route_protocol::active_now();
        let ending: Ending = loop {
            tokio::select! {
                changed = stop.changed() => {
                    let signalled = *stop.borrow();
                    if changed.is_err() || signalled.is_some() {
                        break (signalled.unwrap_or(EndReason::HostShutdown), None);
                    }
                }
                frame = frames.recv() => match frame {
                    None => break (EndReason::ViewerClosed, None),
                    Some(Err(_)) => break (closed_reason(self.connection), None),
                    Some(Ok(frame)) => {
                        heard = crate::share::route_protocol::active_now();
                        if self.on_frame(frame).await.is_err() {
                            break LOST;
                        }
                    }
                },
                _ = capture.tick() => {
                    if let Some(ending) = self.produce().await {
                        break ending;
                    }
                }
                _ = second.tick() => {
                    if crate::share::route_protocol::active_now().saturating_duration_since(heard) > VIEWER_SILENCE {
                        break LOST;
                    }
                    if self.second().await.is_err() {
                        break LOST;
                    }
                }
            }
        };
        reader.abort();
        self.finish(ending).await;
    }

    async fn frame(&mut self, frame: HostFrame) -> io::Result<()> {
        crate::screen::core::control_sender::send(
            self.connection.remote_id().to_string(),
            &mut self.send,
            &frame,
        )
        .await
    }

    async fn error(&mut self, code: ScreenErrorCode, detail: String) -> io::Result<()> {
        let error = ScreenWireError::new(code, Some(detail));
        self.frame(HostFrame::Error(error)).await
    }

    async fn report(&mut self, state: SessionState, reason: Option<&str>) -> io::Result<()> {
        let stream = self.pipeline.stream_info();
        if self.state == state && reason.is_none() && self.reported_stream == stream {
            return Ok(());
        }
        self.state = state;
        self.reported_stream = stream.clone();
        self.handle.update(|view| {
            view.state = state;
            if let Some(stream) = &stream {
                view.codec = Some(stream.codec.key().into());
                view.encoder = Some(stream.encoder.clone());
            }
        });
        let report = StateReport {
            state,
            reason: reason.map(str::to_string),
            stream,
        };
        self.frame(HostFrame::State(report)).await
    }

    async fn second(&mut self) -> io::Result<()> {
        if self.family.selected.is_some() {
            self.family_snapshot(false).await?;
        }
        let stats = self.pipeline.second(self.connection);
        self.handle.update(|view| {
            view.bitrate_kbps = stats.sent_kbps;
            view.fps = stats.fps;
            view.path = stats.path.key().into();
        });
        self.frame(HostFrame::Stats(stats)).await
    }

    async fn produce(&mut self) -> Option<Ending> {
        self.poll_input_release();
        if let Some(applied) = self.pipeline.poll_capture_active() {
            if let Err(error) = applied {
                return Some((
                    EndReason::Failed,
                    Some(ScreenWireError::new(
                        ScreenErrorCode::CaptureFailed,
                        Some(error.to_string()),
                    )),
                ));
            }
            if self.visibility_ack_pending {
                self.visibility_ack_pending = false;
                let floor = self.pipeline.next_frame_id();
                self.input_floor = Some(floor);
                let ack = crate::screen::core::visibility::VisibilityAck {
                    geometry_generation: self.visibility_generation,
                    min_frame_id: floor,
                };
                if self.frame(HostFrame::VisibilityAck(ack)).await.is_err() {
                    return Some(LOST);
                }
                if !self.visible
                    && self
                        .report(SessionState::ViewerHidden, Some("viewer_hidden"))
                        .await
                        .is_err()
                {
                    return Some(LOST);
                }
            }
        }
        if self.visibility_ack_pending {
            return None;
        }
        if !self.visible {
            return None;
        }
        let completion = self.pipeline.take_completion();
        let completed = completion.is_some();
        self.refresh_input_reference();
        let next = match completion {
            None => None,
            Some(WorkCompletion::Snapshot(result)) => {
                if let Err(failure) = result {
                    if self
                        .error(failure.code, failure.error.to_string())
                        .await
                        .is_err()
                    {
                        return Some(LOST);
                    }
                }
                None
            }
            Some(WorkCompletion::Tick(produced)) => match produced {
                Ok(Produced::Sent) => Some(SessionState::Streaming),
                Ok(Produced::Paused(state)) => Some(state),
                Ok(Produced::Idle | Produced::Dropped | Produced::Reset) => None,
                Ok(Produced::Closed) => return Some((closed_reason(self.connection), None)),
                Err(failure) => {
                    return Some((
                        EndReason::Failed,
                        Some(ScreenWireError::new(
                            failure.code,
                            Some(failure.error.to_string()),
                        )),
                    ))
                }
            },
        };
        if completed && self.family.selected.is_some() {
            if let Err(error) = self.family_snapshot(false).await {
                return Some((
                    EndReason::Failed,
                    Some(ScreenWireError::new(
                        ScreenErrorCode::CaptureFailed,
                        Some(error.to_string()),
                    )),
                ));
            }
        }
        if let Err(failure) = self.pipeline.begin_tick() {
            return Some((
                EndReason::Failed,
                Some(ScreenWireError::new(
                    failure.code,
                    Some(failure.error.to_string()),
                )),
            ));
        }
        let notice = self.pipeline.take_fallback_notice();
        if next.is_some() || notice.is_some() {
            self.report(next.unwrap_or(self.state), notice.as_deref())
                .await
                .err()
                .map(|_| LOST)
        } else {
            None
        }
    }

    async fn finish(mut self, (reason, error): Ending) {
        self.pipeline.cancel();
        self.release_control();
        let last = async {
            if let Some(error) = error {
                self.frame(HostFrame::Error(error)).await?;
            }
            self.report(SessionState::Ended, Some(reason.key())).await
        };
        let _ = tokio::time::timeout(CLOSE_GRACE, last).await;
        let _ = self.send.finish();
        let bytes = self.pipeline.totals().bytes;
        let seconds = self.started.elapsed().as_secs();
        let text = format!("{}; {seconds} s; {bytes} bytes", reason.key());
        let audit = self.context.audit;
        log_event(
            audit,
            self.principal,
            self.mode,
            "session_ended",
            None,
            Some(&text),
        );
        if reason != EndReason::ViewerClosed {
            let _ = tokio::time::timeout(CLOSE_GRACE, self.connection.closed()).await;
        }
        self.connection
            .close(SESSION_CLOSE, b"screen session ended");
    }
}

/// Why the viewer's stream failed: it closed the connection, or it was lost.
fn closed_reason(connection: &Connection) -> EndReason {
    match connection.close_reason() {
        Some(ConnectionError::ApplicationClosed(_)) | None => EndReason::ViewerClosed,
        Some(_) => EndReason::ConnectionLost,
    }
}
