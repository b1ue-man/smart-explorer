//! Daemon side of `IpcRequest::Screen` (SCR-M2, strand S4). The variant
//! reaches `ShareHost::serve_screen` through `ipc_host_ext.rs` after its
//! daemon token was checked. Probe, measurement and the relayed viewer
//! session run on the share node of this worker: it connects to the other
//! device with the screen ALPN and signs the client hello. The sessions of
//! this host come from the node's screen registry; without a running share
//! service there are none. A confirmation the other device's user answers
//! is announced as `Pending` before the final reply.
use std::io;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::daemon::ipc_host::ShareHost;
use crate::daemon::ipc_protocol::screen::{
    self, read_relay, write_relay, BenchRequest, ProbeReport, ScreenIpcReply, ScreenIpcRequest,
    StreamRequest, MAX_RELAY_VIEWER_BYTES, RELAY_VIEWER, WATCH_WAIT_SECS,
};
use crate::daemon::ipc_protocol::{write_response, IpcResponse};
use crate::screen::core::bench::{self, BenchOptions};
use crate::screen::core::negotiate::{CodecOffer, CodecPreference};
use crate::screen::core::registry::{SessionView, StopSelector};
use crate::screen::core::viewer_client::{ViewerError, ViewerRequest};
use crate::screen::core::wire_control::{decode_viewer_frame, ViewerFrame};
use crate::screen::core::wire_hello::{ScreenErrorCode, SessionMode};
use crate::screen::os::session_memory::handshake;
use crate::share::access_grant::CapabilityDecision;
use crate::share::{PeerOpenTarget, ShareService};

#[path = "screen_ipc_reconnect.rs"]
mod reconnect;
#[path = "screen_ipc_relay_loop.rs"]
mod relay;
use relay::relay_loop;

/// How long the worker tries to reach a device that is not online yet.
const CONNECT_WAIT: Duration = Duration::from_secs(20);
/// Relay frames queued towards the client.
const RELAY_QUEUE: usize = 8;

/// A refusal for the client: the text and, for a screen refusal (C7), its
/// code (`IpcResponse::Err.code`).
struct Failure {
    code: Option<String>,
    msg: String,
}

impl From<String> for Failure {
    fn from(msg: String) -> Self {
        Self { code: None, msg }
    }
}

impl From<ViewerError> for Failure {
    fn from(error: ViewerError) -> Self {
        let code = match &error {
            ViewerError::Io(_) => None,
            ViewerError::Refused(_) | ViewerError::Unsupported => Some(error.code()),
        };
        Self {
            code,
            msg: error.detail(),
        }
    }
}

impl Failure {
    fn response(self) -> IpcResponse {
        IpcResponse::Err {
            msg: self.msg,
            code: self.code,
        }
    }
}

impl ShareHost {
    /// Serves one `IpcRequest::Screen` whose daemon token was checked.
    pub(in crate::daemon) fn serve_screen(
        &self,
        mut stream: TcpStream,
        request: ScreenIpcRequest,
    ) -> io::Result<()> {
        let metadata_pin = match &request {
            ScreenIpcRequest::PinnedMetadata { expected_node, .. } => Some(expected_node.clone()),
            _ => None,
        };
        let reply = match request {
            ScreenIpcRequest::HostSessions => {
                let (revision, sessions) = self.screen_sessions(None);
                Ok(ScreenIpcReply::Sessions { revision, sessions })
            }
            ScreenIpcRequest::HostWatch { since } => {
                let (revision, sessions) = self.screen_sessions(Some(since));
                Ok(ScreenIpcReply::Sessions { revision, sessions })
            }
            ScreenIpcRequest::HostStop { session } => {
                let selector = session.map_or(StopSelector::All, StopSelector::One);
                let count = self
                    .screen_service()
                    .map_or(0, |service| service.stop_screen_sessions(&selector));
                Ok(ScreenIpcReply::Stopped { count })
            }
            ScreenIpcRequest::Probe { target } => {
                self.screen_probe(&target).map(ScreenIpcReply::Probe)
            }
            ScreenIpcRequest::Bench { target, options } => self
                .screen_bench(&mut stream, &target, &options)
                .map(ScreenIpcReply::Bench),
            ScreenIpcRequest::Stream { target, request } => {
                return self.screen_stream(stream, &target, request, false);
            }
            ScreenIpcRequest::PinnedStream { target, request } => {
                if request.expected_node.is_none() {
                    return write_response(&mut stream, &node_pin_failure().response());
                }
                return self.screen_stream(stream, &target, request, false);
            }
            ScreenIpcRequest::Metadata { target }
            | ScreenIpcRequest::PinnedMetadata { target, .. } => {
                let request = StreamRequest {
                    mode: SessionMode::View,
                    source: Some(crate::screen::core::window_list::SourceId::Synthetic),
                    decoders: vec![CodecOffer::tiles()],
                    display_size: None,
                    max_fps: 1,
                    frame_capabilities: None,
                    max_rate_bps: None,
                    codec: None,
                    decoder: None,
                    gop_continue: false,
                    gop_continue_decoders: Vec::new(),
                    expected_node: metadata_pin,
                    virtual_display: None,
                    delivery: None,
                };
                return self.screen_stream(stream, &target, request, true);
            }
        };
        let response = match reply {
            Ok(reply) => IpcResponse::Screen(reply),
            Err(failure) => failure.response(),
        };
        write_response(&mut stream, &response)
    }

    fn screen_service(&self) -> Option<ShareService> {
        self.state.lock().ok()?.service.clone()
    }

    /// The sessions of this host; with `since`, once they changed after it.
    fn screen_sessions(&self, since: Option<u64>) -> (u64, Vec<SessionView>) {
        let Some(service) = self.screen_service() else {
            return (0, Vec::new());
        };
        let registry = service.screen_registry();
        match since {
            None => registry.sessions(),
            Some(since) => registry.wait_change(since, Duration::from_secs(WATCH_WAIT_SECS)),
        }
    }

    /// The share service connected to `target` with the screen ALPN, tried
    /// until `CONNECT_WAIT` while the device is not online yet.
    fn screen_link(
        &self,
        target: &PeerOpenTarget,
    ) -> Result<(ShareService, crate::share::node_screen::ScreenLink), ViewerError> {
        let deadline = Instant::now() + CONNECT_WAIT;
        loop {
            self.reload_now().map_err(io::Error::other)?;
            self.drain_events();
            let service = self.screen_service().ok_or_else(|| {
                io::Error::other("Share-Server ist nicht konfiguriert oder Auto-Connect ist aus")
            })?;
            let _ = service.cmd(crate::share::ShareCmd::Refresh);
            match service.screen_link(target) {
                Ok(link) => return Ok((service, link)),
                Err(ViewerError::Io(_)) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn screen_probe(&self, target: &PeerOpenTarget) -> Result<ProbeReport, Failure> {
        let unsupported = |device: String, supported: bool| ProbeReport {
            device,
            supported,
            platform: None,
            view: CapabilityDecision::Off,
            control: CapabilityDecision::Off,
            monitors: Vec::new(),
            input: Default::default(),
            codecs: Vec::new(),
            limits: Vec::new(),
            virtual_display: None,
        };
        let device = target.endpoint_prefix();
        let (service, link) = match self.screen_link(target) {
            Ok(linked) => linked,
            Err(ViewerError::Unsupported) => return Ok(unsupported(device, false)),
            Err(error) => return Err(error.into()),
        };
        let device = link.label().to_string();
        let request = viewer_request(SessionMode::Probe, None, vec![CodecOffer::tiles()]);
        let connection = link.connection.clone();
        let opened = service.screen_runtime().block_on(async move {
            let build = |server: &_| link.client_hello(server, &request);
            handshake(connection, build, |_| {}).await
        });
        match opened {
            Ok(session) => {
                session.close();
                let hello = &session.hello;
                Ok(ProbeReport {
                    device,
                    supported: true,
                    platform: Some(hello.platform),
                    view: hello.authorization.view,
                    control: hello.authorization.control,
                    monitors: hello.monitors.clone(),
                    input: hello.input.clone(),
                    codecs: hello.all_encoders(),
                    limits: hello.limits.clone(),
                    virtual_display: hello.virtual_display.clone(),
                })
            }
            Err(ViewerError::Unsupported) => Ok(unsupported(device, false)),
            Err(ViewerError::Refused(code)) if code == ScreenErrorCode::PermissionDenied.key() => {
                Ok(unsupported(device, true))
            }
            Err(error) => Err(error.into()),
        }
    }

    fn screen_bench(
        &self,
        stream: &mut TcpStream,
        target: &PeerOpenTarget,
        request: &BenchRequest,
    ) -> Result<bench::BenchReport, Failure> {
        let codec = CodecPreference::parse(&request.codec).ok_or_else(|| {
            format!(
                "unknown codec {:?}; auto, h264, hevc, av1 or tiles",
                request.codec
            )
        })?;
        let options = BenchOptions {
            codec,
            size: request.size,
            fps: request.fps,
            seconds: request.seconds,
        };
        options.validate()?;
        let codecs = crate::screen::os::codec_inventory::registry();
        let decoders = options.decoders(&codecs)?;
        // `--source` (BK0): the host's real capture instead of the test picture.
        let source = screen::bench_source(request.source.as_deref())?;
        let named = request.source.clone();
        let (service, link) = self.screen_link(target)?;
        let mut viewer = viewer_request(SessionMode::View, Some(source.clone()), decoders);
        viewer.display_size = Some(options.size);
        viewer.max_fps = options.fps;
        viewer.delivery = request.delivery;
        let mut pending = stream.try_clone().map_err(|error| error.to_string())?;
        let connection = link.connection.clone();
        let measured = service.screen_runtime().block_on(async move {
            let build = |server: &_| link.client_hello(server, &viewer);
            let announce = |capability| {
                let reply = IpcResponse::Screen(ScreenIpcReply::Pending { capability });
                let _ = write_response(&mut pending, &reply);
            };
            let session = handshake(connection, build, announce).await?;
            if screen::monitor_missing(&source, &session.hello.monitors) {
                session.close();
                return Err(ViewerError::Refused(screen::BENCH_SOURCE_NOT_FOUND.into()));
            }
            bench::run_source(session, options, &codecs, named).await
        });
        measured.map_err(Failure::from)
    }

    /// Relays one viewer session over `stream` (the desktop viewer).
    fn screen_stream(
        &self,
        mut stream: TcpStream,
        target: &PeerOpenTarget,
        request: StreamRequest,
        metadata: bool,
    ) -> io::Result<()> {
        if request
            .expected_node
            .as_deref()
            .is_some_and(|expected| expected.trim().is_empty())
        {
            return write_response(&mut stream, &node_pin_failure().response());
        }
        let (service, link) = match self.screen_link(target) {
            Ok(linked) => linked,
            Err(error) => return write_response(&mut stream, &Failure::from(error).response()),
        };
        if request
            .expected_node
            .as_deref()
            .is_some_and(|expected| link.connection.remote_id().to_string() != expected)
        {
            link.connection.close(
                iroh::endpoint::VarInt::from_u32(0x5345),
                b"screen node pin changed",
            );
            return write_response(&mut stream, &Failure {
                code: Some("screen_node_changed".into()),
                msg: "Die authentifizierte Bildschirmgegenstelle stimmt nicht mit dem bestaetigten Nodepin ueberein".into(),
            }.response());
        }
        write_response(
            &mut stream,
            &IpcResponse::Screen(ScreenIpcReply::StreamOpened),
        )?;
        let (out_tx, mut out_rx) = mpsc::channel::<(u8, Vec<u8>)>(RELAY_QUEUE);
        let (in_tx, in_rx) = mpsc::channel::<Option<reconnect::RelayedFrame>>(64);
        let memory = std::sync::Arc::new(std::sync::Mutex::new(None));
        let expected_node = request.expected_node.clone();
        // „Codec“ and „Dekodierer“ of the viewer (BK6) narrow the offers;
        // „Höchste Datenrate“ goes out as `RATE_CEILING` once open.
        let decoders = request.chosen_decoders();
        let max_rate_bps = request.max_rate_bps;
        let viewer = ViewerRequest {
            mode: request.mode,
            source: request.source,
            decoders,
            display_size: request.display_size,
            max_fps: request.max_fps,
            frame_capabilities: request.frame_capabilities,
            // B-36: only a client that keeps its decoder over a
            // continuation asks for it: the desktop viewer when its decoders
            // can (`gui/.../screen_decode_local.rs` `announces`); the Android
            // sink refuses a group without a key frame
            // (`os/android/viewer_sink.rs` `record`) and never asks.
            gop_continue: request.gop_continue,
            // B-36 per codec: the decoders of the client that continue.
            gop_continue_decoders: request.gop_continue_decoders,
            virtual_display: request.virtual_display,
            // Codex 3/6: „Scharf (GPU)“ with one video decoder reads as such.
            codec_choice: request.codec,
            // Codex 4/6: the ceiling holds from the first picture on.
            max_rate_bps: request.max_rate_bps,
            // „Übertragung“ from the first picture on.
            delivery: request.delivery,
        };
        let redial = reconnect::Redial {
            service: service.clone(),
            target: target.clone(),
            expected_node,
        };
        service.screen_runtime().spawn(reconnect::relay_session(
            link,
            redial,
            viewer,
            out_tx,
            in_rx,
            metadata,
            memory.clone(),
            max_rate_bps,
        ));
        let mut input = stream.try_clone()?;
        let reader = std::thread::Builder::new()
            .name("screen-relay-input".into())
            .spawn(move || relay_input(&mut input, &in_tx))?;
        while let Some((tag, payload)) = out_rx.blocking_recv() {
            if write_relay(&mut stream, tag, &payload).is_err() {
                break;
            }
        }
        drop(out_rx);
        drop(memory);
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let _ = reader.join();
        Ok(())
    }
}

fn node_pin_failure() -> Failure {
    Failure {
        code: Some("screen_node_required".into()),
        msg: "Der bestaetigte Nodepin fehlt".into(),
    }
}

fn viewer_request(
    mode: SessionMode,
    source: Option<crate::screen::core::window_list::SourceId>,
    decoders: Vec<CodecOffer>,
) -> ViewerRequest {
    ViewerRequest {
        mode,
        source,
        decoders,
        display_size: None,
        max_fps: 30,
        frame_capabilities: None,
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        virtual_display: None,
        codec_choice: None,
        max_rate_bps: None,
        delivery: None,
    }
}

/// Reads the client's relay frames: viewer control frames for the session,
/// `None` when the client closes (`RELAY_CLOSE`, end of stream, bad frame).
fn relay_input(input: &mut TcpStream, frames: &mpsc::Sender<Option<reconnect::RelayedFrame>>) {
    while let Ok((tag, payload)) = read_relay(input, MAX_RELAY_VIEWER_BYTES) {
        // `RELAY_VIEWER_FALLBACK`: a frame of the viewer's automatic fallback
        // (round 11, N2), sent to the host like every frame.
        let fallback = tag == crate::daemon::ipc_protocol::screen::switch::RELAY_VIEWER_FALLBACK;
        if tag != RELAY_VIEWER && !fallback {
            break;
        }
        let Some((wire_tag, body)) = payload.split_first() else {
            break;
        };
        let Ok(frame) = decode_viewer_frame(*wire_tag, body) else {
            break;
        };
        // Pictures are acknowledged, `PING` is sent, a bandwidth probe is
        // answered and the picture datagrams are negotiated and reported by
        // the worker itself (`video_transport.rs`).
        let own = matches!(
            frame,
            ViewerFrame::FrameAck(_)
                | ViewerFrame::PreviewAck(_)
                | ViewerFrame::Ping(_)
                | ViewerFrame::ProbeResult(_)
                | ViewerFrame::FrameLoss(_)
                | ViewerFrame::VideoOffer(_)
                | ViewerFrame::VideoReport(_)
                | ViewerFrame::VideoTcpOffer(_)
                | ViewerFrame::VideoTcpResult(_)
        );
        if !own
            && frames
                .blocking_send(Some(reconnect::RelayedFrame { frame, fallback }))
                .is_err()
        {
            return;
        }
    }
    let _ = frames.blocking_send(None);
}
