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
    encode_relay_record, read_relay, write_relay, BenchRequest, ProbeReport, RelayEnd, RelayHello,
    ScreenIpcReply, ScreenIpcRequest, StreamRequest, MAX_RELAY_VIEWER_BYTES, RELAY_BROKEN,
    RELAY_CLOCK, RELAY_END, RELAY_HELLO, RELAY_HOST, RELAY_RECORD, RELAY_VIEWER, WATCH_WAIT_SECS,
};
use crate::daemon::ipc_protocol::{write_response, IpcResponse};
use crate::screen::core::bench::{self, BenchOptions};
use crate::screen::core::gop_stream::{now_us, path_of, PictureEvent};
use crate::screen::core::metrics::ClockSync;
use crate::screen::core::negotiate::{CodecOffer, CodecPreference};
use crate::screen::core::registry::{SessionView, StopSelector};
use crate::screen::core::viewer_client::{handshake, ViewerError, ViewerRequest, ViewerSession};
use crate::screen::core::wire;
use crate::screen::core::wire_control::{
    decode_viewer_frame, encode_host_frame, HostFrame, ViewerFrame,
};
use crate::screen::core::wire_hello::{ScreenErrorCode, SessionMode};
use crate::share::access_grant::CapabilityDecision;
use crate::share::{PeerOpenTarget, ShareService};

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
                let request=StreamRequest {mode:SessionMode::View,source:Some(crate::screen::core::window_list::SourceId::Synthetic),
                    decoders:vec![CodecOffer::tiles()],display_size:None,max_fps:1,frame_capabilities:None,expected_node:metadata_pin};
                return self.screen_stream(stream,&target,request,true);
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
                    codecs: hello.encoders.clone(),
                    limits: hello.limits.clone(),
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
        let codec = CodecPreference::parse(&request.codec)
            .ok_or_else(|| format!("unknown codec {:?}; auto, h264 or tiles", request.codec))?;
        let options = BenchOptions {
            codec,
            size: request.size,
            fps: request.fps,
            seconds: request.seconds,
        };
        options.validate()?;
        let codecs = crate::screen::os::codec_inventory::registry();
        let decoders = options.decoders(&codecs)?;
        let (service, link) = self.screen_link(target)?;
        let source = Some(crate::screen::core::window_list::SourceId::Synthetic);
        let mut viewer = viewer_request(SessionMode::View, source, decoders);
        viewer.display_size = Some(options.size);
        viewer.max_fps = options.fps;
        let mut pending = stream.try_clone().map_err(|error| error.to_string())?;
        let connection = link.connection.clone();
        let measured = service.screen_runtime().block_on(async move {
            let build = |server: &_| link.client_hello(server, &viewer);
            let announce = |capability| {
                let reply = IpcResponse::Screen(ScreenIpcReply::Pending { capability });
                let _ = write_response(&mut pending, &reply);
            };
            let session = handshake(connection, build, announce).await?;
            bench::run(session, options, &codecs).await
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
        if request.expected_node.as_deref().is_some_and(|expected| expected.trim().is_empty()) {
            return write_response(&mut stream, &node_pin_failure().response());
        }
        let (service, link) = match self.screen_link(target) {
            Ok(linked) => linked,
            Err(error) => return write_response(&mut stream, &Failure::from(error).response()),
        };
        if request.expected_node.as_deref().is_some_and(|expected| link.connection.remote_id().to_string() != expected) {
            link.connection.close(iroh::endpoint::VarInt::from_u32(0x5345), b"screen node pin changed");
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
        let (in_tx, in_rx) = mpsc::channel::<Option<ViewerFrame>>(64);
        let viewer = ViewerRequest {
            mode: request.mode,
            source: request.source,
            decoders: request.decoders,
            display_size: request.display_size,
            max_fps: request.max_fps,
            frame_capabilities: request.frame_capabilities,
        };
        service
            .screen_runtime()
            .spawn(relay_session(link, viewer, out_tx, in_rx, metadata));
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
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let _ = reader.join();
        Ok(())
    }
}

fn node_pin_failure() -> Failure {
    Failure { code: Some("screen_node_required".into()), msg: "Der bestaetigte Nodepin fehlt".into() }
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
    }
}

/// Reads the client's relay frames: viewer control frames for the session,
/// `None` when the client closes (`RELAY_CLOSE`, end of stream, bad frame).
fn relay_input(input: &mut TcpStream, frames: &mpsc::Sender<Option<ViewerFrame>>) {
    while let Ok((tag, payload)) = read_relay(input, MAX_RELAY_VIEWER_BYTES) {
        if tag != RELAY_VIEWER {
            break;
        }
        let Some((wire_tag, body)) = payload.split_first() else {
            break;
        };
        let Ok(frame) = decode_viewer_frame(*wire_tag, body) else {
            break;
        };
        // Pictures are acknowledged and `PING` is sent by the worker itself.
        let own = matches!(
            frame,
            ViewerFrame::FrameAck(_) | ViewerFrame::PreviewAck(_) | ViewerFrame::Ping(_)
        );
        if !own && frames.blocking_send(Some(frame)).is_err() {
            return;
        }
    }
    let _ = frames.blocking_send(None);
}

/// Runs the relayed session on the share runtime.
async fn relay_session(
    link: crate::share::node_screen::ScreenLink,
    request: ViewerRequest,
    out: mpsc::Sender<(u8, Vec<u8>)>,
    mut input: mpsc::Receiver<Option<ViewerFrame>>,
    metadata: bool,
) {
    let connection = link.connection.clone();
    let pending_out = out.clone();
    let build = |server: &_| link.client_hello(server, &request);
    let announce = |capability| {
        if let Ok(json) = wire::to_json(&RelayHello::Pending { capability }) {
            let _ = pending_out.try_send((RELAY_HELLO, json));
        }
    };
    let opened=if metadata {crate::screen::core::viewer_client::handshake_metadata(connection,build,announce).await}
        else {handshake(connection,build,announce).await};
    let mut session = match opened {
        Ok(session) => session,
        Err(error) => {
            end(&out, error.code(), Some(error.detail())).await;
            return;
        }
    };
    let path = path_of(&session.connection).0.key().to_string();
    let accepted = RelayHello::Accepted {
        hello: session.hello.clone(),
        path,
    };
    if let Ok(json) = wire::to_json(&accepted) {
        if out.send((RELAY_HELLO, json)).await.is_err() {
            session.close();
            return;
        }
    }
    let ended = relay_loop(&mut session, &out, &mut input).await;
    session.close();
    let (code, detail) = ended;
    end(&out, code, detail).await;
}

async fn end(out: &mpsc::Sender<(u8, Vec<u8>)>, code: String, detail: Option<String>) {
    if let Ok(json) = wire::to_json(&RelayEnd { code, detail }) {
        let _ = out.send((RELAY_END, json)).await;
    }
}

/// Forwards host frames and pictures to the client and its control frames
/// to the host; returns the end code and detail.
async fn relay_loop(
    session: &mut ViewerSession,
    out: &mpsc::Sender<(u8, Vec<u8>)>,
    input: &mut mpsc::Receiver<Option<ViewerFrame>>,
) -> (String, Option<String>) {
    let mut clock = ClockSync::default();
    let mut offset = None;
    loop {
        let forwarded = tokio::select! {
            frame = session.host.recv() => match frame {
                Some(Ok(frame)) => {
                    if let HostFrame::Pong(pong) = &frame {
                        clock.on_pong(pong, now_us());
                        if clock.offset_us() != offset {
                            offset = clock.offset_us();
                            let value = offset.unwrap_or(0).to_be_bytes().to_vec();
                            let _ = out.send((RELAY_CLOCK, value)).await;
                        }
                    }
                    match encode_host_frame(&frame) {
                        Ok((tag, payload)) => {
                            let mut body = Vec::with_capacity(payload.len() + 1);
                            body.push(tag);
                            body.extend_from_slice(&payload);
                            out.send((RELAY_HOST, body)).await
                        }
                        Err(_) => Ok(()),
                    }
                }
                Some(Err(error)) => return ("connection_lost".into(), Some(error.to_string())),
                None => return ("ended".into(), None),
            },
            event = session.pictures.recv() => match event {
                Some(PictureEvent::Record { header, record, .. }) => {
                    match encode_relay_record(&header, &record) {
                        Ok(body) => out.send((RELAY_RECORD, body)).await,
                        Err(_) => Ok(()),
                    }
                }
                Some(PictureEvent::Broken { gop_id }) => {
                    out.send((RELAY_BROKEN, gop_id.to_be_bytes().to_vec())).await
                }
                None => return ("ended".into(), None),
            },
            frame = input.recv() => match frame {
                Some(Some(frame)) => {
                    let _ = session.control.send(frame).await;
                    Ok(())
                }
                Some(None) | None => return ("viewer_closed".into(), None),
            },
        };
        if forwarded.is_err() {
            return ("viewer_closed".into(), None);
        }
    }
}
