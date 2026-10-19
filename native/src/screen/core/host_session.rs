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
use crate::screen::core::host_pipeline::{Pipeline, Produced};
use crate::screen::core::input_model::{InputEvent, PressedInputs};
use crate::screen::core::registry::SessionHandle;
use crate::screen::core::session_state::{EndReason, SessionState};
use crate::screen::core::wire_binary::StreamKind;
use crate::screen::core::wire_control::{
    recv_viewer_frame, send_host_frame, ControlAction, ControlState, HostFrame, Pong,
    ScreenWireError, SetMode, StateReport, StreamMode, ViewerFrame,
};
use crate::screen::core::wire_hello::{ScreenErrorCode, SessionMode};
use crate::share::relation_proof::ProvenPrincipal;

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
    pipeline: Pipeline,
    pressed: PressedInputs,
    state: SessionState,
    started: Instant,
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
    ) -> Self {
        Self {
            context,
            connection,
            send,
            handle,
            principal,
            mode,
            pipeline,
            pressed: PressedInputs::default(),
            state: SessionState::Starting,
            started: Instant::now(),
        }
    }

    /// Runs the session until it ends; `max_fps` paces the capture ticks.
    pub(super) async fn run(mut self, recv: RecvStream, max_fps: u32) {
        let (frames_tx, mut frames) = mpsc::channel(FRAME_QUEUE);
        let reader = tokio::spawn(read_viewer_frames(recv, frames_tx));
        let mut stop = self.handle.stop_signal();
        let tick = Duration::from_micros(1_000_000 / u64::from(max_fps.max(1)));
        let mut capture = tokio::time::interval(tick);
        capture.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut second = tokio::time::interval(Duration::from_secs(1));
        let mut heard = Instant::now();
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
                        heard = Instant::now();
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
                    if heard.elapsed() > VIEWER_SILENCE {
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
        send_host_frame(&mut self.send, &frame).await
    }

    async fn error(&mut self, code: ScreenErrorCode, detail: String) -> io::Result<()> {
        let error = ScreenWireError::new(code, Some(detail));
        self.frame(HostFrame::Error(error)).await
    }

    async fn report(&mut self, state: SessionState, reason: Option<&str>) -> io::Result<()> {
        if self.state == state && reason.is_none() {
            return Ok(());
        }
        self.state = state;
        let stream = self.pipeline.stream_info();
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
        let stats = self.pipeline.second(self.connection);
        self.handle.update(|view| {
            view.bitrate_kbps = stats.sent_kbps;
            view.fps = stats.fps;
            view.path = stats.path.key().into();
        });
        self.frame(HostFrame::Stats(stats)).await
    }

    async fn produce(&mut self) -> Option<Ending> {
        let next = match self.pipeline.tick(self.connection).await {
            Ok(Produced::Sent) => SessionState::Streaming,
            Ok(Produced::Paused(state)) => state,
            Ok(Produced::Idle | Produced::Dropped | Produced::Reset) => return None,
            Ok(Produced::Closed) => return Some((closed_reason(self.connection), None)),
            Err(failure) => {
                let error = ScreenWireError::new(failure.code, Some(failure.error.to_string()));
                return Some((EndReason::Failed, Some(error)));
            }
        };
        self.report(next, None).await.err().map(|_| LOST)
    }

    async fn on_frame(&mut self, frame: ViewerFrame) -> io::Result<()> {
        match frame {
            ViewerFrame::FrameAck(ack) | ViewerFrame::PreviewAck(ack) => {
                self.pipeline.on_ack(ack);
                Ok(())
            }
            ViewerFrame::KeyframeRequest(_) => {
                self.pipeline.request_keyframe();
                Ok(())
            }
            ViewerFrame::Ping(ping) => {
                let received = now_us();
                let pong = Pong::answer(&ping, received, now_us());
                self.frame(HostFrame::Pong(pong)).await
            }
            ViewerFrame::SetMode(set) => self.set_mode(set).await,
            ViewerFrame::ListWindows => match self.context.backend.windows() {
                Ok(list) => self.frame(HostFrame::Windows(list.bounded())).await,
                Err(error) => {
                    self.error(ScreenErrorCode::CaptureFailed, error.to_string())
                        .await
                }
            },
            ViewerFrame::Snapshot(request) => {
                // The source is the session's own (hello or `SET_MODE`); the
                // frame's `source` stays on the wire for later viewers and is
                // not used here (SCR-M4, `se screen snapshot` names its source
                // in the session request).
                let shot = self.pipeline.snapshot(self.connection, request.max_size);
                match shot.await {
                    Ok(()) => Ok(()),
                    Err(failure) => self.error(failure.code, failure.error.to_string()).await,
                }
            }
            ViewerFrame::Input(events) => self.input(&events).await,
            ViewerFrame::ControlRequest(request) => self.control(request.action).await,
        }
    }

    async fn set_mode(&mut self, set: SetMode) -> io::Result<()> {
        if let Some(source) = set.source.as_ref() {
            let content = source.is_screen_content();
            let backend = self.context.backend.as_ref();
            if content && !(backend.graphical_session() && self.context.indicator.show().is_ok()) {
                let detail = format!("source {source}");
                return self
                    .error(ScreenErrorCode::IndicatorUnavailable, detail)
                    .await;
            }
            match backend.open_source(source, None) {
                Ok(opened) => {
                    self.pipeline.set_source(opened);
                    self.handle.set_screen_content(content);
                    let name = source.to_string();
                    self.handle.update(|view| view.source = name);
                }
                Err(refusal) => {
                    return self.error(ScreenErrorCode::NoDisplay, refusal.reason).await;
                }
            }
        }
        let kind = match set.mode {
            StreamMode::Preview => StreamKind::Preview,
            StreamMode::Full => StreamKind::Video,
        };
        self.pipeline.set_mode(kind, set.size, set.fps);
        let shown = match (kind, self.mode) {
            (StreamKind::Preview, _) => SessionMode::Preview,
            (_, SessionMode::Control) => SessionMode::Control,
            _ => SessionMode::View,
        };
        self.handle.update(|view| view.mode = shown);
        Ok(())
    }

    async fn input(&mut self, events: &[InputEvent]) -> io::Result<()> {
        if !self.handle.holds_control() {
            let detail = "input without control".to_string();
            return self.error(ScreenErrorCode::RightNotGranted, detail).await;
        }
        match self.context.backend.inject(events) {
            Ok(()) => {
                for event in events {
                    self.pressed.observe(event);
                }
                Ok(())
            }
            Err(blocked) => {
                let state = ControlState {
                    holder: self.handle.controller(),
                    blocked: Some(blocked),
                };
                self.frame(HostFrame::ControlState(state)).await
            }
        }
    }

    async fn control(&mut self, action: ControlAction) -> io::Result<()> {
        if self.mode != SessionMode::Control {
            let detail = "this session may only view".to_string();
            return self.error(ScreenErrorCode::RightNotGranted, detail).await;
        }
        match action {
            ControlAction::Take => match self.handle.take_control() {
                Ok(()) => {
                    let audit = self.context.audit;
                    log_event(
                        audit,
                        self.principal,
                        self.mode,
                        "control_taken",
                        None,
                        None,
                    );
                }
                Err(holder) => {
                    let state = ControlState {
                        holder: Some(holder),
                        blocked: None,
                    };
                    self.frame(HostFrame::ControlState(state)).await?;
                    return self
                        .error(ScreenErrorCode::ControlBusy, String::new())
                        .await;
                }
            },
            ControlAction::Release => self.release_control(),
        }
        let state = ControlState {
            holder: self.handle.controller(),
            blocked: None,
        };
        self.frame(HostFrame::ControlState(state)).await
    }

    /// Lets go of every pressed input and of the control.
    fn release_control(&mut self) {
        let released = self.pressed.release_all();
        if !released.is_empty() {
            let _ = self.context.backend.inject(&released);
        }
        if self.handle.release_control() {
            let audit = self.context.audit;
            log_event(
                audit,
                self.principal,
                self.mode,
                "control_released",
                None,
                None,
            );
        }
    }

    async fn finish(mut self, (reason, error): Ending) {
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
