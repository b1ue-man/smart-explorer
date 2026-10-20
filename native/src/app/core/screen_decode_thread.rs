//! The threads of one viewer session (SCR-M7, U25-4). The decode thread
//! opens the relayed stream, reads its events in a loop and decodes the
//! pictures; the GUI thread only ever takes the newest picture out of a slot
//! (`Shared`) and wakes up for it. A sender thread writes the frames the GUI
//! queues (`SET_MODE`, `INPUT`, …), so a stalled socket never blocks the GUI
//! either. The relay acknowledges pictures and answers `PING` itself (SCR-M2);
//! the viewer sends neither `FRAME_ACK` nor `PREVIEW_ACK`.
//!
//! Picture groups: a record of a newer group drops the previous group and
//! opens a decoder for the new one; a record of an older group is stale and
//! ignored. A decode error or a broken group is answered with one key frame
//! request (`KeyframeRequester`: at most one open request); until a key frame
//! of the group arrives its records are skipped.
use std::io;
use std::sync::{mpsc, Arc};
use std::time::Instant;

use super::screen_session::{
    refusal_end, Backend, DecoderOpener, EndInfo, Outgoing, Picture, PictureData, Shared,
    StreamLink,
};
use super::screen_texts::{end_text, error_text};
use crate::daemon::{StreamEvent, StreamRequest};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_backend::ScreenDecoder;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::{KeyframeRequester, SessionState};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, FLAG_KEYFRAME};
use crate::screen::core::wire_control::{
    ControlAction, ControlRequest, HostFrame, KeyframeReason, KeyframeRequest, ViewerFrame,
};
use crate::share::PeerOpenTarget;

type SendFrame = Arc<dyn Fn(&ViewerFrame) -> io::Result<()> + Send + Sync>;

/// Starts the session's threads; the stream opens on the first of them.
pub(crate) fn spawn(
    backend: Backend,
    peer: PeerOpenTarget,
    request: StreamRequest,
    shared: Arc<Shared>,
    (outbox, inbox): (mpsc::Sender<Outgoing>, mpsc::Receiver<Outgoing>),
    take_control: bool,
) {
    let (failed, wake) = (shared.clone(), backend.wake.clone());
    let started = std::thread::Builder::new()
        .name("screen-viewer-decode".into())
        .spawn(move || {
            run(
                backend,
                peer,
                request,
                shared,
                (outbox, inbox),
                take_control,
            )
        });
    if let Err(error) = started {
        failed.end(EndInfo {
            text: format!("Die Sitzung konnte nicht starten: {error}"),
            code: None,
        });
        wake();
    }
}

fn run(
    backend: Backend,
    peer: PeerOpenTarget,
    request: StreamRequest,
    shared: Arc<Shared>,
    (outbox, inbox): (mpsc::Sender<Outgoing>, mpsc::Receiver<Outgoing>),
    take_control: bool,
) {
    let wake = backend.wake.clone();
    let link = match (backend.open)(peer, request) {
        Ok(link) => link,
        Err(message) => {
            shared.end(refusal_end(&message));
            wake();
            return;
        }
    };
    let StreamLink {
        mut next,
        send,
        close,
    } = link;
    spawn_sender(inbox, send.clone(), close);
    let mut state = DecodeState::new(shared, backend.decoder.clone(), send, take_control);
    loop {
        match next() {
            Ok(event) => {
                let last = state.on_event(event);
                wake();
                if last {
                    break;
                }
            }
            Err(_) => {
                state.connection_lost();
                wake();
                break;
            }
        }
    }
    // Lets the sender thread go.
    let _ = outbox.send(Outgoing::Close);
}

/// Writes the queued frames to the host; ends with the session.
fn spawn_sender(
    inbox: mpsc::Receiver<Outgoing>,
    send: SendFrame,
    close: Arc<dyn Fn() -> io::Result<()> + Send + Sync>,
) {
    let _ = std::thread::Builder::new()
        .name("screen-viewer-send".into())
        .spawn(move || {
            while let Ok(item) = inbox.recv() {
                match item {
                    Outgoing::Frame(frame) => {
                        if send(&frame).is_err() {
                            break;
                        }
                    }
                    Outgoing::Close => break,
                }
            }
            let _ = close();
        });
}

/// One picture group with its decoder.
struct Gop {
    id: u64,
    decoder: Box<dyn ScreenDecoder>,
    /// The decoder keeps planes instead of converting.
    yuv: bool,
}

/// What the decode thread keeps between two events.
pub(crate) struct DecodeState {
    shared: Arc<Shared>,
    open_decoder: DecoderOpener,
    send: SendFrame,
    take_control: bool,
    gop: Option<Gop>,
    /// The newest group seen; records of older ones are stale.
    latest_gop: u64,
    /// A group whose records are skipped until its next key frame.
    broken: Option<u64>,
    keyframes: KeyframeRequester,
    seq: u64,
    last_shown: Option<Instant>,
    started: Instant,
}

impl DecodeState {
    pub(crate) fn new(
        shared: Arc<Shared>,
        open_decoder: DecoderOpener,
        send: SendFrame,
        take_control: bool,
    ) -> Self {
        Self {
            shared,
            open_decoder,
            send,
            take_control,
            gop: None,
            latest_gop: 0,
            broken: None,
            keyframes: KeyframeRequester::default(),
            seq: 0,
            last_shown: None,
            started: Instant::now(),
        }
    }

    /// Applies one event; `true` when it was the session's last.
    pub(crate) fn on_event(&mut self, event: StreamEvent) -> bool {
        match event {
            StreamEvent::Pending(_) => self
                .shared
                .update(|info| info.state = SessionState::WaitingForApproval),
            StreamEvent::Accepted { hello, path } => {
                self.shared.update(|info| {
                    info.hello = Some(hello);
                    info.path = Some(path);
                    info.accepted += 1;
                    if matches!(
                        info.state,
                        SessionState::Connecting | SessionState::WaitingForApproval
                    ) {
                        info.state = SessionState::Starting;
                    }
                });
                if self.take_control {
                    let take = ControlRequest {
                        action: ControlAction::Take,
                    };
                    let _ = (self.send)(&ViewerFrame::ControlRequest(take));
                }
            }
            StreamEvent::Host(frame) => self.host(frame),
            StreamEvent::Record { header, record } => self.record(header, record),
            StreamEvent::Broken { gop_id } => {
                if self.gop.as_ref().is_some_and(|gop| gop.id == gop_id) {
                    self.gop = None;
                }
                self.broken = Some(gop_id);
                self.request_keyframe(KeyframeReason::Reset, gop_id);
            }
            StreamEvent::Clock(offset) => self.shared.update(|info| info.clock_offset_us = offset),
            StreamEvent::End(end) => {
                let text = end_text(&end.code, end.detail.as_deref());
                self.shared.update(|info| {
                    info.end = Some(EndInfo {
                        text,
                        code: Some(end.code),
                    });
                    info.state = SessionState::Ended;
                });
                return true;
            }
        }
        false
    }

    /// The reading side broke without an end from the relay.
    pub(crate) fn connection_lost(&mut self) {
        self.shared.end(EndInfo {
            text: end_text("connection_lost", None),
            code: Some("connection_lost".into()),
        });
    }

    fn host(&mut self, frame: HostFrame) {
        match frame {
            HostFrame::State(report) => self.shared.update(|info| {
                info.state = report.state;
                info.reason = report.reason;
                if report.stream.is_some() {
                    info.stream = report.stream;
                }
            }),
            HostFrame::Error(error) => {
                let text = error_text(&error.code, error.detail.as_deref());
                let busy = error.code == "control_busy";
                self.shared.update(|info| {
                    info.notice = Some(text);
                    info.control_denied |= busy;
                });
            }
            HostFrame::Windows(list) => self.shared.update(|info| info.windows = Some(list)),
            HostFrame::ControlState(state) => self.shared.update(|info| {
                info.controller = state.holder;
                info.blocked = state.blocked;
            }),
            HostFrame::Stats(stats) => self.shared.update(|info| info.stats = Some(stats)),
            // The relay answers `PING` and reports the clock itself.
            HostFrame::Pong(_) => {}
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// At most one key frame request is open (`KEYFRAME_RETRY_US`).
    fn request_keyframe(&mut self, reason: KeyframeReason, gop_id: u64) {
        if self.keyframes.should_request(self.now()) {
            let request = KeyframeRequest {
                reason,
                gop_id: Some(gop_id),
            };
            let _ = (self.send)(&ViewerFrame::KeyframeRequest(request));
        }
    }

    fn open_gop(&mut self, header: &GopHeader) -> Option<Gop> {
        match (self.open_decoder)(header) {
            Ok(mut decoder) => {
                let yuv = self.shared.prefers_yuv();
                if yuv {
                    decoder.prefer_yuv(true);
                }
                Some(Gop {
                    id: header.gop_id,
                    decoder,
                    yuv,
                })
            }
            Err(error) => {
                let text = format!("Das Bild kann nicht dekodiert werden: {error}");
                self.shared.update(|info| info.notice = Some(text));
                self.broken = Some(header.gop_id);
                self.request_keyframe(KeyframeReason::DecodeError, header.gop_id);
                None
            }
        }
    }

    fn record(&mut self, header: GopHeader, record: FrameRecord) {
        if header.gop_id < self.latest_gop {
            return;
        }
        let keyframe = record.flags & FLAG_KEYFRAME != 0;
        if self.broken == Some(header.gop_id) {
            if !keyframe {
                return;
            }
            self.broken = None;
            self.gop = None;
        }
        self.latest_gop = header.gop_id;
        let mut gop = match self.gop.take() {
            Some(gop) if gop.id == header.gop_id => gop,
            // The previous group is dropped with its decoder.
            _ => match self.open_gop(&header) {
                Some(gop) => gop,
                None => return,
            },
        };
        let want_yuv = self.shared.prefers_yuv();
        if want_yuv != gop.yuv {
            gop.decoder.prefer_yuv(want_yuv);
            gop.yuv = want_yuv;
        }
        match gop.decoder.decode(&record) {
            Ok(complete) => {
                if keyframe {
                    self.keyframes.on_keyframe();
                }
                if complete {
                    self.show(&gop, header.size, record.capture_ts_us);
                }
                self.gop = Some(gop);
            }
            Err(_) => {
                self.broken = Some(header.gop_id);
                self.request_keyframe(KeyframeReason::DecodeError, header.gop_id);
            }
        }
    }

    /// Hands the finished picture to the GUI thread.
    fn show(&mut self, gop: &Gop, size: FrameSize, capture_ts_us: u64) {
        let shown_us = now_us();
        let gap = std::time::Duration::from_millis(self.shared.present_gap_ms());
        let due = self
            .last_shown
            .is_none_or(|shown| gap.is_zero() || shown.elapsed() >= gap);
        self.shared.update(|info| {
            let latency =
                i128::from(shown_us) + i128::from(info.clock_offset_us) - i128::from(capture_ts_us);
            if let Ok(latency) = u64::try_from(latency) {
                info.latency.record(latency);
            }
            info.pictures += 1;
        });
        if !due {
            return;
        }
        let planes = gop
            .yuv
            .then(|| gop.decoder.yuv_picture())
            .flatten()
            .map(PictureData::Yuv);
        let data = planes.unwrap_or_else(|| PictureData::Bgra(gop.decoder.picture().clone()));
        self.seq += 1;
        self.last_shown = Some(Instant::now());
        self.shared.put_picture(Picture {
            seq: self.seq,
            size,
            data,
        });
    }
}
