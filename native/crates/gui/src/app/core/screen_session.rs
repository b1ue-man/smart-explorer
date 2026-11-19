//! One viewer session of the desktop viewer (SCR-M7, U25-3/4): what the GUI
//! thread sees of it (`SessionInfo`, the newest picture), the handle that
//! sends frames and ends it, and the connection to the daemon that a test
//! replaces (`Backend`). Opening, reading and decoding happen on threads of
//! their own (`screen_decode_thread`); the GUI thread only takes the newest
//! picture out of a slot and reads the state, so decoding and a slow
//! connection never block it. Picture contents are never logged.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError};

use eframe::egui;

pub use super::screen_session_backend::{
    Backend, DecoderOpener, Opener, ProbeInfo, Prober, StreamLink, Wake,
};
use super::screen_target::ScreenTarget;
use super::screen_visibility::ResumeGate;
use crate::daemon::StreamRequest;
use crate::screen::core::host_backend::YuvPicture;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::visibility::ViewerVisibility;
use crate::screen::core::window_list::WindowList;
use crate::screen::core::wire_control::{
    ControlBlock, ControlHolder, HostStats, SetMode, StreamInfo, StreamMode, ViewerFrame,
};
use crate::screen::core::wire_hello::{ScreenErrorCode, ScreenHelloOk, SessionMode};

pub use super::screen_chain_window::LatencyWindow;

#[path = "screen_session_choice.rs"]
mod choice;
pub(crate) use choice::hardware_bars_tiles;

/// The planes or the pixels of a decoded picture.
pub enum PictureData {
    Bgra(BgraImage),
    Yuv(YuvPicture),
}

/// The newest decoded picture of a session.
pub struct Picture {
    /// Counts the pictures of the session; the draw code uploads a picture
    /// to the GPU once.
    pub seq: u64,
    pub presented: super::screen_presented::PresentedFrame,
    pub size: FrameSize,
    pub data: PictureData,
    pub memory: Option<Arc<crate::screen::core::frame_memory::MemoryLease>>,
}

/// Why a session ended, as the status line shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndInfo {
    pub text: String,
    /// The code the host or the relay gave, if any.
    pub code: Option<String>,
}

/// What the viewer knows of a session (U25-3 `ViewerSession`).
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub state: SessionState,
    pub reason: Option<String>,
    pub end: Option<EndInfo>,
    pub hello: Option<ScreenHelloOk>,
    /// `direct` or `relay`, from the handshake.
    pub path: Option<String>,
    pub controller: Option<ControlHolder>,
    pub blocked: Option<ControlBlock>,
    /// The host holds this viewer's input back for now (E2, `CONTROL_STATE`).
    pub held: Option<crate::screen::core::wire_control::InputHeld>,
    pub stream: Option<StreamInfo>,
    pub stats: Option<HostStats>,
    /// Not delivered by the relay yet; shown as „–“.
    pub rtt_ms: Option<u32>,
    pub clock_offset_us: i64,
    pub windows: Option<WindowList>,
    pub family: Option<crate::screen::core::family_viewer::FamilyViewer>,
    pub family_icon: Option<std::sync::Arc<egui::IconData>>,
    pub icons: Arc<Mutex<super::screen_icons::ScreenIcons>>,
    pub icon_peer: String,
    /// The text of the last error frame that did not end the session.
    pub notice: Option<String>,
    /// The host refused the control (`control_busy`): another device has it.
    pub control_denied: bool,
    pub latency: LatencyWindow,
    /// How often the host accepted this session (a connection counts once).
    pub accepted: u32,
    pub pictures: u64,
    /// The host's pointer for the overlay (plan 2026-11-14 P6).
    pub cursor: crate::screen::core::cursor_overlay::CursorView,
    /// „Höchste Datenrate“ of this session (BK6, B-09).
    pub rate_ceiling: crate::daemon::RateCeilingState,
    /// The decoder of the running picture group with its stage (BK6).
    pub decoder: Option<String>,
}

impl SessionInfo {
    fn new() -> Self {
        Self {
            state: SessionState::Connecting,
            reason: None,
            end: None,
            hello: None,
            path: None,
            controller: None,
            blocked: None,
            held: None,
            stream: None,
            stats: None,
            rtt_ms: None,
            clock_offset_us: 0,
            windows: None,
            family: None,
            family_icon: None,
            icons: Arc::new(Mutex::new(super::screen_icons::ScreenIcons::default())),
            icon_peer: String::new(),
            notice: None,
            control_denied: false,
            latency: LatencyWindow::default(),
            accepted: 0,
            pictures: 0,
            cursor: Default::default(),
            rate_ceiling: Default::default(),
            decoder: None,
        }
    }

    pub fn is_ended(&self) -> bool {
        self.end.is_some() || self.state == SessionState::Ended
    }
}

/// What the GUI thread and the session's threads share.
pub struct Shared {
    pub(super) retirement: Arc<super::screen_render_owner::RenderSlot>,
    info: Mutex<SessionInfo>,
    pub(super) picture: Mutex<Option<Picture>>,
    /// The draw code takes planes (GPU path) instead of BGRA.
    prefer_yuv: AtomicBool,
    /// Shortest time between two shown pictures of a preview, in ms; every
    /// picture is still decoded (later tile frames build on earlier ones).
    present_gap_ms: AtomicU64,
    pub(super) resume: Mutex<ResumeGate>,
    pub(super) presentation: Mutex<super::screen_presented::PresentationState>,
    pub(super) visibility: Mutex<Option<(ViewerVisibility, bool)>>,
    decoder_mode: Mutex<SetMode>,
    /// The viewer switched to the tiles itself (`screen_session_choice.rs`).
    pub(super) lossless_fallback: AtomicBool,
    /// Codecs this session gave up (`screen_decode_guard.rs`, fix of
    /// 2026-11-18: for this session only, never for the process).
    given_up: Mutex<crate::screen::core::decoder_health::GivenUp>,
    /// The „Codec“ this session runs for (`screen_session_choice.rs`).
    codec: Mutex<crate::screen::core::negotiate::CodecSetting>,
    /// The codec of the session's first stream (`screen_session_choice.rs`).
    first_codec: Mutex<Option<crate::screen::core::negotiate::CodecKind>>,
    pub(super) memory: Mutex<Option<Arc<crate::screen::core::frame_memory::MemoryLease>>>,
    /// Returned picture storage and local decoder switches (V1).
    pub(super) local: super::screen_decode_local::DecodeLocal,
    /// The window's Direct3D 11 presenter while it takes GPU pictures (P8).
    pub(super) direct: super::screen_direct::DirectSlot,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub fn new(prefer_yuv: bool, present_gap_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            retirement: Arc::default(),
            info: Mutex::new(SessionInfo::new()),
            picture: Mutex::new(None),
            memory: Mutex::new(None),
            prefer_yuv: AtomicBool::new(prefer_yuv),
            present_gap_ms: AtomicU64::new(present_gap_ms),
            resume: Mutex::new(ResumeGate::default()),
            presentation: Mutex::new(super::screen_presented::PresentationState::default()),
            visibility: Mutex::new(None),
            decoder_mode: Mutex::new(SetMode {
                mode: StreamMode::Full,
                source: None,
                size: None,
                fps: None,
                sharpness: false,
                lossless: false,
            }),
            lossless_fallback: AtomicBool::new(false),
            given_up: Mutex::new(Default::default()),
            codec: Mutex::new(Default::default()),
            first_codec: Mutex::new(None),
            local: Default::default(),
            direct: Default::default(),
        })
    }

    pub fn info(&self) -> MutexGuard<'_, SessionInfo> {
        lock(&self.info)
    }

    /// A full view asks the host for the pointer overlay (P6).
    pub(super) fn wants_cursor(&self) -> bool {
        let full = lock(&self.decoder_mode).mode == StreamMode::Full;
        full && self.info().family.is_none()
    }

    /// The newest picture since the last call.
    pub fn take_picture(&self) -> Option<Picture> {
        let picture = lock(&self.picture).take();
        if picture.is_some() {
            // The hand-over to the graphics (BK0, `screen_chain_window`).
            lock(&self.info).latency.on_taken();
        }
        picture
    }

    pub fn update(&self, edit: impl FnOnce(&mut SessionInfo)) {
        edit(&mut lock(&self.info));
    }

    /// Replaces the shown picture; the previous one that nobody took is gone.
    pub fn put_picture(&self, picture: Picture) {
        let presentation = lock(&self.presentation);
        if presentation.matches(&picture.presented) {
            let unshown = lock(&self.picture).is_some();
            self.local.replace(&self.picture, picture);
            drop(presentation);
            lock(&self.info).latency.on_put(unshown);
        }
    }

    pub fn visibility_ack(&self, geometry_generation: u64, min_frame_id: u64) {
        lock(&self.resume).acknowledge(geometry_generation, min_frame_id);
    }

    pub fn prefers_yuv(&self) -> bool {
        self.prefer_yuv.load(Ordering::Relaxed)
    }

    pub fn present_gap_ms(&self) -> u64 {
        self.present_gap_ms.load(Ordering::Relaxed)
    }

    /// The session ended without a relay end (it never opened, or the
    /// connection broke).
    pub fn end(&self, end: EndInfo) {
        self.stop_presentation();
        self.update(|info| {
            if info.end.is_none() {
                info.end = Some(end);
            }
            info.state = SessionState::Ended;
        });
    }
}

/// Frames for the sender thread.
pub enum Outgoing {
    Frame(ViewerFrame),
    Close,
}

/// The refusal text `code: message` of the relay as an end: the code of a
/// known refusal gets its German text.
pub fn refusal_end(message: &str) -> EndInfo {
    if let Some((code, detail)) = message.split_once(": ") {
        if ScreenErrorCode::from_key(code).is_some() {
            return EndInfo {
                text: super::screen_texts::error_text(code, Some(detail)),
                code: Some(code.into()),
            };
        }
    }
    EndInfo {
        text: message.to_string(),
        code: None,
    }
}

/// A viewer session: the GUI's handle.
pub struct ViewerSession {
    pub target: ScreenTarget,
    /// The mode the session runs in (a preview raised to the full view
    /// becomes `View`).
    pub mode: SessionMode,
    pub(super) shared: Arc<Shared>,
    outbox: mpsc::Sender<Outgoing>,
}

impl ViewerSession {
    /// Starts a session; connecting happens on its own thread. With
    /// `take_control` the session asks for the control once accepted.
    pub fn open(
        backend: &Backend,
        target: ScreenTarget,
        mut request: StreamRequest,
        take_control: bool,
        prefer_yuv: bool,
        present_gap_ms: u64,
    ) -> Self {
        let (outbox, inbox) = mpsc::channel();
        // Decoder stages a library gave up for earlier sessions are tried
        // again (fix of 2026-11-18, `decoder_health::begin_session`).
        let epoch = crate::screen::core::decoder_health::begin_session();
        request.expected_node = target.expected_node.clone();
        let shared = Shared::new(prefer_yuv, present_gap_ms);
        shared.local.epoch.store(epoch, Ordering::Release);
        let opening = SetMode {
            mode: if request.mode == SessionMode::Preview {
                StreamMode::Preview
            } else {
                StreamMode::Full
            },
            source: None,
            size: request.display_size,
            fps: (request.max_fps != 0).then_some(request.max_fps),
            sharpness: false,
            lossless: false,
        };
        // The stored „Codec“ goes out once the host accepted (user rule
        // 2026-11-18, `screen_session_choice.rs`).
        shared.set_opening(opening, request.codec.unwrap_or_default());
        shared.update(|info| {
            info.family = request
                .source
                .as_ref()
                .and_then(crate::screen::core::family_viewer::FamilyViewer::for_source)
        });
        let mode = request.mode;
        match target.to_peer() {
            Ok(peer) => {
                let sender = outbox.clone();
                {
                    let mut info = shared.info();
                    info.icon_peer = serde_json::to_string(&peer).unwrap_or_default();
                    lock(&info.icons)
                        .requests
                        .set_sender(Arc::new(move |request| {
                            let _ = sender.send(Outgoing::Frame(ViewerFrame::IconRequest(request)));
                        }));
                }
                super::screen_decode_thread::spawn(
                    backend.clone(),
                    peer,
                    request,
                    shared.clone(),
                    (outbox.clone(), inbox),
                    take_control,
                )
            }
            Err(message) => {
                shared.end(EndInfo {
                    text: message,
                    code: None,
                });
            }
        }
        Self {
            target,
            mode,
            shared,
            outbox,
        }
    }

    #[cfg(test)]
    pub fn fixture(shared: Arc<Shared>) -> Self {
        let (outbox, _inbox) = mpsc::channel();
        Self {
            target: ScreenTarget::direct("renderer-fixture", Some("renderer-fixture")),
            mode: SessionMode::View,
            shared,
            outbox,
        }
    }

    pub fn info(&self) -> MutexGuard<'_, SessionInfo> {
        self.shared.info()
    }

    /// The newest picture since the last call.
    pub fn take_picture(&self) -> Option<Picture> {
        self.shared.take_picture()
    }

    /// A paint drew the shown picture again because no new one came (r1).
    pub fn note_repeated(&self) {
        self.shared.update(|info| info.latency.on_repeated());
    }

    pub fn invalidate_input(&self) {
        self.shared.invalidate_presentation();
    }

    pub fn can_present(&self, frame: super::screen_presented::PresentedFrame) -> bool {
        self.shared.can_present(&frame)
    }

    pub fn presented_callback(
        &self,
        frame: super::screen_presented::PresentedFrame,
    ) -> super::screen_presented::Presented {
        let shared = self.shared.clone();
        let outbox = self.outbox.clone();
        Arc::new(move || {
            let modern = shared.modern();
            shared.presented(frame.clone(), || {
                !modern
                    || outbox
                        .send(Outgoing::Frame(ViewerFrame::Presented {
                            gop_id: frame.gop_id,
                            frame_id: frame.frame_id,
                        }))
                        .is_ok()
            });
        })
    }

    pub fn input_ready(&self) -> bool {
        lock(&self.shared.presentation).validity.active() && lock(&self.shared.resume).ready()
    }

    pub fn set_prefer_yuv(&self, on: bool) {
        self.shared.prefer_yuv.store(on, Ordering::Relaxed);
    }

    pub fn set_present_gap_ms(&self, gap_ms: u64) {
        self.shared.present_gap_ms.store(gap_ms, Ordering::Relaxed);
    }

    /// Queues a frame for the host; the GUI thread never waits for the socket.
    pub fn send(&self, mut frame: ViewerFrame) {
        self.shared.decoder_mode(&mut frame);
        if let ViewerFrame::Input(events) = frame {
            if self.info().family.is_some() {
                super::family_viewer_session::input(self, events);
            } else {
                let _ = self
                    .outbox
                    .send(Outgoing::Frame(ViewerFrame::Input(events)));
            }
            return;
        }
        let _ = self.outbox.send(Outgoing::Frame(frame));
    }

    /// Ends the session (the host sees the viewer leave).
    pub fn close(&self) {
        self.shared.stop_presentation();
        let _ = self.outbox.send(Outgoing::Close);
    }

    pub fn is_ended(&self) -> bool {
        self.info().is_ended()
    }
}

impl Drop for ViewerSession {
    fn drop(&mut self) {
        self.close();
    }
}
