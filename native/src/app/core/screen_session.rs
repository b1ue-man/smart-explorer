//! One viewer session of the desktop viewer (SCR-M7, U25-3/4): what the GUI
//! thread sees of it (`SessionInfo`, the newest picture), the handle that
//! sends frames and ends it, and the connection to the daemon that a test
//! replaces (`Backend`). Opening, reading and decoding happen on threads of
//! their own (`screen_decode_thread`); the GUI thread only takes the newest
//! picture out of a slot and reads the state, so decoding and a slow
//! connection never block it. Picture contents are never logged.
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError};

use eframe::egui;

use super::screen_target::ScreenTarget;
use crate::daemon::{StreamEvent, StreamRequest};
use crate::screen::core::host_backend::{ScreenDecoder, YuvPicture};
use crate::screen::core::negotiate::CodecOffer;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::WindowList;
use crate::screen::core::wire_binary::GopHeader;
use crate::screen::core::wire_control::{
    ControlBlock, ControlHolder, HostStats, StreamInfo, ViewerFrame,
};
use crate::screen::core::wire_hello::{ScreenErrorCode, ScreenHelloOk, SessionMode};
use crate::share::access_grant::CapabilityDecision;
use crate::share::PeerOpenTarget;

/// Samples the latency figures of the statistics are taken from. It only
/// smooths the display: the figures follow the latest frames.
pub(crate) const LATENCY_WINDOW: usize = 200;

/// The planes or the pixels of a decoded picture.
pub(crate) enum PictureData {
    Bgra(BgraImage),
    Yuv(YuvPicture),
}

/// The newest decoded picture of a session.
pub(crate) struct Picture {
    /// Counts the pictures of the session; the draw code uploads a picture
    /// to the GPU once.
    pub seq: u64,
    pub size: FrameSize,
    pub data: PictureData,
}

/// Why a session ended, as the status line shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EndInfo {
    pub text: String,
    /// The code the host or the relay gave, if any.
    pub code: Option<String>,
}

/// The latest capture-to-display times in µs.
#[derive(Clone, Debug, Default)]
pub(crate) struct LatencyWindow {
    samples: VecDeque<u64>,
}

impl LatencyWindow {
    pub(crate) fn record(&mut self, latency_us: u64) {
        if self.samples.len() == LATENCY_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(latency_us);
    }

    /// Nearest-rank p50 and p95 in ms.
    pub(crate) fn percentiles_ms(&self) -> Option<(f64, f64)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let rank = |share: f64| {
            let index = (share * sorted.len() as f64).ceil() as usize;
            sorted[index.clamp(1, sorted.len()) - 1] as f64 / 1000.0
        };
        Some((rank(0.50), rank(0.95)))
    }
}

/// What the viewer knows of a session (U25-3 `ViewerSession`).
#[derive(Clone, Debug)]
pub(crate) struct SessionInfo {
    pub state: SessionState,
    pub reason: Option<String>,
    pub end: Option<EndInfo>,
    pub hello: Option<ScreenHelloOk>,
    /// `direct` or `relay`, from the handshake.
    pub path: Option<String>,
    pub controller: Option<ControlHolder>,
    pub blocked: Option<ControlBlock>,
    pub stream: Option<StreamInfo>,
    pub stats: Option<HostStats>,
    /// Not delivered by the relay yet; shown as „–“.
    pub rtt_ms: Option<u32>,
    pub clock_offset_us: i64,
    pub windows: Option<WindowList>,
    /// The text of the last error frame that did not end the session.
    pub notice: Option<String>,
    /// The host refused the control (`control_busy`): another device has it.
    pub control_denied: bool,
    pub latency: LatencyWindow,
    /// How often the host accepted this session (a connection counts once).
    pub accepted: u32,
    pub pictures: u64,
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
            stream: None,
            stats: None,
            rtt_ms: None,
            clock_offset_us: 0,
            windows: None,
            notice: None,
            control_denied: false,
            latency: LatencyWindow::default(),
            accepted: 0,
            pictures: 0,
        }
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.end.is_some() || self.state == SessionState::Ended
    }
}

/// Wakes the GUI thread (production: `Context::request_repaint`).
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// A relayed session as plain functions, so the daemon's `ScreenStream`
/// need not be named and a test can stand in for the host.
pub(crate) struct StreamLink {
    /// The next event; blocks until one arrives.
    pub next: Box<dyn FnMut() -> io::Result<StreamEvent> + Send>,
    /// Sends a control frame to the host.
    pub send: Arc<dyn Fn(&ViewerFrame) -> io::Result<()> + Send + Sync>,
    /// Ends the session.
    pub close: Arc<dyn Fn() -> io::Result<()> + Send + Sync>,
}

pub(crate) type Opener =
    Arc<dyn Fn(PeerOpenTarget, StreamRequest) -> Result<StreamLink, String> + Send + Sync>;
pub(crate) type DecoderOpener =
    Arc<dyn Fn(&GopHeader) -> io::Result<Box<dyn ScreenDecoder>> + Send + Sync>;

/// What a probe of a device tells the group view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProbeInfo {
    /// The device knows the screen protocol of this version.
    pub supported: bool,
    pub view: CapabilityDecision,
}

pub(crate) type Prober = Arc<dyn Fn(PeerOpenTarget) -> Result<ProbeInfo, String> + Send + Sync>;

/// Everything a session needs from outside.
#[derive(Clone)]
pub(crate) struct Backend {
    pub open: Opener,
    pub decoder: DecoderOpener,
    pub probe: Prober,
    pub wake: Wake,
    /// The decoders this process offers the host.
    pub offers: Vec<CodecOffer>,
}

impl Backend {
    /// The daemon's relay and the codecs of this process.
    pub(crate) fn production(ctx: &egui::Context) -> Self {
        let registry = Arc::new(crate::screen::os::codec_inventory::registry());
        let offers = registry.decoder_offers();
        let open: Opener = Arc::new(
            |target: PeerOpenTarget, request: StreamRequest| -> Result<StreamLink, String> {
                let mut stream = crate::daemon::screen_stream(target, request)?;
                let control = stream.control();
                let (sender, closer) = (control.clone(), control);
                Ok(StreamLink {
                    next: Box::new(move || stream.next_event()),
                    send: Arc::new(move |frame: &ViewerFrame| sender.send(frame)),
                    close: Arc::new(move || closer.close()),
                })
            },
        );
        let decoder: DecoderOpener =
            Arc::new(move |header: &GopHeader| registry.open_decoder(header));
        let probe: Prober = Arc::new(|target| {
            crate::daemon::screen_probe(target).map(|report| ProbeInfo {
                supported: report.supported,
                view: report.view,
            })
        });
        let ctx = ctx.clone();
        Backend {
            open,
            decoder,
            probe,
            wake: Arc::new(move || ctx.request_repaint()),
            offers,
        }
    }
}

/// What the GUI thread and the session's threads share.
pub(crate) struct Shared {
    info: Mutex<SessionInfo>,
    picture: Mutex<Option<Picture>>,
    /// The draw code takes planes (GPU path) instead of BGRA.
    prefer_yuv: AtomicBool,
    /// Shortest time between two shown pictures of a preview, in ms; every
    /// picture is still decoded (later tile frames build on earlier ones).
    present_gap_ms: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub(crate) fn new(prefer_yuv: bool, present_gap_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            info: Mutex::new(SessionInfo::new()),
            picture: Mutex::new(None),
            prefer_yuv: AtomicBool::new(prefer_yuv),
            present_gap_ms: AtomicU64::new(present_gap_ms),
        })
    }

    pub(crate) fn info(&self) -> MutexGuard<'_, SessionInfo> {
        lock(&self.info)
    }

    /// The newest picture since the last call.
    pub(crate) fn take_picture(&self) -> Option<Picture> {
        lock(&self.picture).take()
    }

    pub(crate) fn update(&self, edit: impl FnOnce(&mut SessionInfo)) {
        edit(&mut lock(&self.info));
    }

    /// Replaces the shown picture; the previous one that nobody took is gone.
    pub(crate) fn put_picture(&self, picture: Picture) {
        *lock(&self.picture) = Some(picture);
    }

    pub(crate) fn prefers_yuv(&self) -> bool {
        self.prefer_yuv.load(Ordering::Relaxed)
    }

    pub(crate) fn present_gap_ms(&self) -> u64 {
        self.present_gap_ms.load(Ordering::Relaxed)
    }

    /// The session ended without a relay end (it never opened, or the
    /// connection broke).
    pub(crate) fn end(&self, end: EndInfo) {
        self.update(|info| {
            if info.end.is_none() {
                info.end = Some(end);
            }
            info.state = SessionState::Ended;
        });
    }
}

/// Frames for the sender thread.
pub(crate) enum Outgoing {
    Frame(ViewerFrame),
    Close,
}

/// The refusal text `code: message` of the relay as an end: the code of a
/// known refusal gets its German text.
pub(crate) fn refusal_end(message: &str) -> EndInfo {
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
pub(crate) struct ViewerSession {
    pub target: ScreenTarget,
    /// The mode the session runs in (a preview raised to the full view
    /// becomes `View`).
    pub mode: SessionMode,
    shared: Arc<Shared>,
    outbox: mpsc::Sender<Outgoing>,
}

impl ViewerSession {
    /// Starts a session; connecting happens on its own thread. With
    /// `take_control` the session asks for the control once accepted.
    pub(crate) fn open(
        backend: &Backend,
        target: ScreenTarget,
        request: StreamRequest,
        take_control: bool,
        prefer_yuv: bool,
        present_gap_ms: u64,
    ) -> Self {
        let (outbox, inbox) = mpsc::channel();
        let shared = Shared::new(prefer_yuv, present_gap_ms);
        let mode = request.mode;
        match target.to_peer() {
            Ok(peer) => super::screen_decode_thread::spawn(
                backend.clone(),
                peer,
                request,
                shared.clone(),
                (outbox.clone(), inbox),
                take_control,
            ),
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

    pub(crate) fn info(&self) -> MutexGuard<'_, SessionInfo> {
        self.shared.info()
    }

    /// The newest picture since the last call.
    pub(crate) fn take_picture(&self) -> Option<Picture> {
        self.shared.take_picture()
    }

    pub(crate) fn set_prefer_yuv(&self, on: bool) {
        self.shared.prefer_yuv.store(on, Ordering::Relaxed);
    }

    pub(crate) fn set_present_gap_ms(&self, gap_ms: u64) {
        self.shared.present_gap_ms.store(gap_ms, Ordering::Relaxed);
    }

    /// Queues a frame for the host; the GUI thread never waits for the socket.
    pub(crate) fn send(&self, frame: ViewerFrame) {
        let _ = self.outbox.send(Outgoing::Frame(frame));
    }

    /// Ends the session (the host sees the viewer leave).
    pub(crate) fn close(&self) {
        let _ = self.outbox.send(Outgoing::Close);
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.info().is_ended()
    }
}

impl Drop for ViewerSession {
    fn drop(&mut self) {
        self.close();
    }
}
