//! The viewer side of a screen session (SCR-M2, decision C3). On a
//! connection with the screen ALPN the host opens the control stream and
//! writes `SERVER_HELLO` first (as in Exec: QUIC announces a stream only
//! with its first frame); the viewer answers with the client hello that the
//! share side signs (relation proof), allows the host its picture streams
//! (`VIEWER_UNI_STREAMS`), waits through `APPROVAL_PENDING` without a time
//! limit (C5) and then runs four tasks: the writer of the control stream,
//! the reader of the host's frames, the acceptor of the picture streams
//! (which acknowledges every frame) and the `PING` clock once a second. A
//! device without the screen ALPN ends the TLS handshake with alert 120,
//! seen as transport code `0x178`: `screen_unsupported` (NR N2); so does a
//! host of another protocol version.
use std::io;
use std::time::Duration;

use iroh::endpoint::{Connection, VarInt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::gop_stream::{
    accept_picture_streams_with_memory, now_us, picture_channel, PictureEvent, VIEWER_UNI_STREAMS,
};
use super::negotiate::CodecOffer;
use super::pixel_convert::FrameSize;
use super::window_list::SourceId;
use super::wire::supports_version;
use super::wire_control::{recv_host_frame, send_viewer_frame, HostFrame, Ping, ViewerFrame};
use super::wire_hello::{
    recv_hello_reply, recv_server_hello, send_client_hello, HelloReply, ScreenClientHello,
    ScreenErrorCode, ScreenHelloOk, ScreenServerHello, SessionMode,
};
use crate::share::access_grant::Capability;

/// Time for the host's `SERVER_HELLO` (the Exec handshake allows the same).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// Interval of the viewer's `PING` (clock offset, liveness on the host).
const PING_INTERVAL: Duration = Duration::from_secs(1);
/// Control frames queued for the writer, host frames for the reader.
const CONTROL_QUEUE: usize = 256;
/// Close code of a viewer that ends its session.
pub(crate) const VIEWER_CLOSE: VarInt = VarInt::from_u32(0);

/// What the viewer asks the host for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewerRequest {
    pub mode: SessionMode,
    pub source: Option<SourceId>,
    pub decoders: Vec<CodecOffer>,
    pub display_size: Option<FrameSize>,
    pub max_fps: u16,
    pub frame_capabilities: Option<super::frame_budget::FrameCapabilities>,
}

/// Why a session did not start or ended.
#[derive(Debug)]
pub(crate) enum ViewerError {
    /// The host refused with this code key (`HELLO_ERROR`, `ERROR`).
    Refused(String),
    /// The device does not know the screen protocol or this version.
    Unsupported,
    Io(io::Error),
}

impl ViewerError {
    /// The code key as surfaces show it (`permission_denied`, …).
    pub(crate) fn code(&self) -> String {
        match self {
            Self::Refused(code) => code.clone(),
            Self::Unsupported => ScreenErrorCode::ScreenUnsupported.key().into(),
            Self::Io(_) => "connection_failed".into(),
        }
    }

    pub(crate) fn detail(&self) -> String {
        match self {
            Self::Refused(code) => refusal_text(code),
            Self::Unsupported => refusal_text(ScreenErrorCode::ScreenUnsupported.key()),
            Self::Io(error) => error.to_string(),
        }
    }
}

impl From<io::Error> for ViewerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The German text of a refusal code; unknown codes of newer hosts stay as
/// they are.
pub(crate) fn refusal_text(code: &str) -> String {
    let text = match ScreenErrorCode::from_key(code) {
        Some(ScreenErrorCode::PermissionDenied) => {
            "Dieses Gerät darf den Bildschirm nicht ansehen (Recht „Bildschirm ansehen“ ist aus)"
        }
        Some(ScreenErrorCode::RightNotGranted) => {
            "Nur Ansehen ist erlaubt; „Bildschirm steuern“ ist aus"
        }
        Some(ScreenErrorCode::ApprovalDenied) => "Am Gerät abgelehnt",
        Some(ScreenErrorCode::ApprovalCancelled) => "Die Rückfrage wurde zurückgezogen",
        Some(ScreenErrorCode::ApprovalUnavailable) => "Am Gerät kann niemand gefragt werden",
        Some(ScreenErrorCode::NoDisplay) => "Das Gerät hat keine grafische Sitzung zum Zeigen",
        Some(ScreenErrorCode::IndicatorUnavailable) => {
            "Bildschirmfreigabe braucht die Desktop-App auf dem Gerät"
        }
        Some(ScreenErrorCode::ControlBusy) => "Ein anderes Gerät steuert gerade",
        Some(ScreenErrorCode::ControlUnavailable) => "Steuern ist auf diesem Gerät nicht möglich",
        Some(ScreenErrorCode::HostShutdown) => "Smart Explorer auf dem Gerät wurde beendet",
        Some(ScreenErrorCode::ScreenUnsupported) => {
            "Gerät kennt die Bildschirmfreigabe nicht (ältere Version)"
        }
        Some(ScreenErrorCode::CaptureFailed) => "Die Aufnahme auf dem Gerät ist fehlgeschlagen",
        Some(ScreenErrorCode::EncoderFailed) => "Der Kodierer auf dem Gerät ist fehlgeschlagen",
        Some(ScreenErrorCode::SessionLimit) => {
            "Das Gerät zeigt schon die höchste Zahl an Sitzungen"
        }
        None => return code.to_string(),
    };
    text.to_string()
}

/// One running session of this viewer. Dropping it ends its tasks; `close`
/// tells the host.
pub(crate) struct ViewerSession {
    pub connection: Connection,
    pub hello: ScreenHelloOk,
    /// Frames for the host (`SET_MODE`, `INPUT`, …).
    pub control: mpsc::Sender<ViewerFrame>,
    /// The host's control frames; an error ends the stream.
    pub host: mpsc::Receiver<io::Result<HostFrame>>,
    pub pictures: mpsc::Receiver<PictureEvent>,
    tasks: Vec<JoinHandle<()>>,
    pub(crate) memory: super::frame_memory::MemoryHold,
}

impl ViewerSession {
    pub(crate) fn close(&self) {
        self.connection.close(VIEWER_CLOSE, b"viewer closed");
    }
}

impl Drop for ViewerSession {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Runs the handshake on `connection`; `build` signs the client hello for
/// the host's hello, `on_pending` hears every `APPROVAL_PENDING`.
pub(crate) async fn handshake(
    connection: Connection,
    build: impl FnOnce(&ScreenServerHello) -> io::Result<ScreenClientHello>,
    on_pending: impl FnMut(Capability),
) -> Result<ViewerSession, ViewerError> {
    handshake_for(connection,|server|build(server).map(|hello|(hello,None)),on_pending,false).await
}

/// A list/icon channel. Older hosts retain LIST_WINDOWS over the synthetic source only.
pub(crate) async fn handshake_metadata(
    connection: Connection,
    build: impl FnOnce(&ScreenServerHello) -> io::Result<ScreenClientHello>,
    on_pending: impl FnMut(Capability),
) -> Result<ViewerSession, ViewerError> {
    handshake_for(connection,|server|build(server).map(|hello|(hello,None)),on_pending,true).await
}

pub(crate) async fn handshake_admitted(
    connection:Connection,
    build:impl FnOnce(&ScreenServerHello)->io::Result<(ScreenClientHello,super::frame_memory::MemoryHold)>,
    on_pending:impl FnMut(Capability),
)->Result<ViewerSession,ViewerError> {
    handshake_for(connection,build,on_pending,false).await
}

async fn handshake_for(
    connection: Connection,
    build: impl FnOnce(&ScreenServerHello) -> io::Result<(ScreenClientHello,super::frame_memory::MemoryHold)>,
    mut on_pending: impl FnMut(Capability),
    metadata: bool,
) -> Result<ViewerSession, ViewerError> {
    let (mut send, mut recv) = crate::share::route_protocol::run_with_route(
        &connection, HANDSHAKE_TIMEOUT, None,
        async { connection.accept_bi().await.map_err(io::Error::other) }).await?;
    let server = crate::share::route_protocol::run_with_route(
        &connection, HANDSHAKE_TIMEOUT, None, recv_server_hello(&mut recv)).await?;
    if !supports_version(server.protocol_version) {
        connection.close(VIEWER_CLOSE, b"screen protocol version");
        return Err(ViewerError::Unsupported);
    }
    let (mut hello,memory) = build(&server)?;
    hello.frame_capabilities=hello.frame_capabilities.map(|caps|caps.for_server(server.frame_memory_class)).transpose()?;
    hello.family_version = server.family_version.filter(|version|*version==super::family_protocol::FAMILY_PROTOCOL);
    if hello.source.as_ref().is_some_and(|source|super::family_protocol::selector(source).is_some()) && hello.family_version.is_none() {
        return Err(ViewerError::Unsupported);
    }
    hello.icon_version = server.icon_version.filter(|version|*version==super::app_icon::ICON_VERSION);
    hello.metadata_only = metadata && (hello.icon_version.is_some() || hello.family_version.is_some());
    if metadata {
        hello.mode = SessionMode::View;
        hello.source = Some(SourceId::Synthetic);
        hello.frame_capabilities = None;
    }
    connection.set_max_concurrent_uni_streams(VarInt::from_u32(VIEWER_UNI_STREAMS));
    send_client_hello(&mut send, &hello).await?;
    let accepted = loop {
        match recv_hello_reply(&mut recv).await? {
            HelloReply::Accepted(accepted) => break accepted,
            HelloReply::Pending(pending) => on_pending(pending.capability),
            HelloReply::Refused(refused) => {
                connection.close(VIEWER_CLOSE, b"refused");
                return Err(ViewerError::Refused(refused.code));
            }
        }
    };
    if !metadata && accepted.frame_contract.is_none() && memory.as_ref().is_some_and(|lease|lease.bytes()<super::frame_budget::MAX_WORKING_BYTES) {
        return Err(ViewerError::Io(io::Error::other("legacy host requires the complete legacy memory budget")));
    }
    if accepted.icon_version.is_some() && accepted.icon_version!=hello.icon_version {
        return Err(ViewerError::Io(io::Error::other("host enabled an unoffered icon contract")));
    }
    if accepted.family_version.is_some() && accepted.family_version != hello.family_version {
        return Err(ViewerError::Io(io::Error::other("host enabled an unoffered window family protocol")));
    }
    if hello.source.as_ref().is_some_and(|source|super::family_protocol::selector(source).is_some()) && accepted.family_version.is_none() {
        return Err(ViewerError::Unsupported);
    }
    if let Some(contract) = accepted.frame_contract {
        let offered = hello.frame_capabilities.ok_or_else(|| io::Error::other("host enabled an unoffered frame protocol"))?;
        offered.validate()?;
        if contract.version != offered.version || (contract.hybrid_regions && !offered.hybrid_regions)
            || contract.limits.max_size.width > offered.frames.max_size.width.min(offered.texture_size.width)
            || contract.limits.max_size.height > offered.frames.max_size.height.min(offered.texture_size.height)
            || contract.limits.max_pixels > offered.frames.max_pixels || contract.limits.working_bytes > offered.frames.working_bytes {
            return Err(ViewerError::Io(io::Error::other("host frame contract exceeds viewer's capability evidence")));
        }
    }
    let (control, mut outgoing) = mpsc::channel::<ViewerFrame>(CONTROL_QUEUE);
    let (host_tx, host) = mpsc::channel(CONTROL_QUEUE);
    let (pictures_tx, pictures) = picture_channel();
    let icons_supported=accepted.icon_version==Some(super::app_icon::ICON_VERSION);
    let pending_icons=std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<super::app_icon::IconRequest,std::time::Instant>::new()));
    let sent_icons=pending_icons.clone();
    let writer = tokio::spawn(async move {
        while let Some(frame) = outgoing.recv().await {
            if let ViewerFrame::IconRequest(request)=&frame {
                if !icons_supported {continue;}
                let admitted={
                    let mut pending=sent_icons.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    pending.retain(|_,at|at.elapsed()<Duration::from_secs(30));
                    if pending.len()>=8 || pending.contains_key(request) {false}
                    else {pending.insert(request.clone(),std::time::Instant::now());true}
                };
                if !admitted {continue;}
            }
            if send_viewer_frame(&mut send, &frame).await.is_err() {
                return;
            }
        }
        let _ = send.finish();
    });
    let reader = tokio::spawn(async move {
        loop {
            let frame = recv_host_frame(&mut recv).await;
            if let Ok(HostFrame::IconReply(response))=&frame {
                // At most eight correlated icon replies may enter the host-frame queue.
                if pending_icons.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&response.request).is_none() {continue;}
            }
            let failed = frame.is_err();
            if host_tx.send(frame).await.is_err() || failed {
                return;
            }
        }
    });
    let streams = tokio::spawn(accept_picture_streams_with_memory(
        connection.clone(),
        pictures_tx,
        control.clone(),
        accepted.frame_contract, memory.clone(),
    ));
    let pings = control.clone();
    let clock = tokio::spawn(async move {
        let mut interval = tokio::time::interval(PING_INTERVAL);
        let mut sequence = 0u64;
        loop {
            interval.tick().await;
            sequence += 1;
            let ping = Ping {
                sequence,
                sent_us: now_us(),
            };
            if pings.send(ViewerFrame::Ping(ping)).await.is_err() {
                return;
            }
        }
    });
    Ok(ViewerSession {
        connection,
        hello: accepted,
        control,
        host,
        pictures,
        tasks: vec![writer, reader, streams, clock],
        memory,
    })
}

/// Whether a connect error means the device does not know the screen ALPN:
/// TLS alert 120 (`no_application_protocol`) is QUIC transport code `0x178`.
pub(crate) fn is_unsupported_alpn(error: &(dyn std::error::Error + 'static)) -> bool {
    let crypto_120 = iroh::endpoint::TransportErrorCode::crypto(120);
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(iroh::endpoint::ConnectionError::ConnectionClosed(close)) =
            error.downcast_ref::<iroh::endpoint::ConnectionError>()
        {
            if close.error_code == crypto_120 {
                return true;
            }
        }
        if error
            .to_string()
            .contains("cryptographic handshake failed: error 120")
        {
            return true;
        }
        current = error.source();
    }
    false
}
