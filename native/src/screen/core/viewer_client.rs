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
    accept_picture_streams, now_us, picture_channel, PictureEvent, VIEWER_UNI_STREAMS,
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
    mut on_pending: impl FnMut(Capability),
) -> Result<ViewerSession, ViewerError> {
    let accepted = tokio::time::timeout(HANDSHAKE_TIMEOUT, connection.accept_bi())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "screen host sent no hello"))?;
    let (mut send, mut recv) = accepted.map_err(io::Error::other)?;
    let server = tokio::time::timeout(HANDSHAKE_TIMEOUT, recv_server_hello(&mut recv))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "screen host sent no hello"))??;
    if !supports_version(server.protocol_version) {
        connection.close(VIEWER_CLOSE, b"screen protocol version");
        return Err(ViewerError::Unsupported);
    }
    let hello = build(&server)?;
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
    let (control, mut outgoing) = mpsc::channel::<ViewerFrame>(CONTROL_QUEUE);
    let (host_tx, host) = mpsc::channel(CONTROL_QUEUE);
    let (pictures_tx, pictures) = picture_channel();
    let writer = tokio::spawn(async move {
        while let Some(frame) = outgoing.recv().await {
            if send_viewer_frame(&mut send, &frame).await.is_err() {
                return;
            }
        }
        let _ = send.finish();
    });
    let reader = tokio::spawn(async move {
        loop {
            let frame = recv_host_frame(&mut recv).await;
            let failed = frame.is_err();
            if host_tx.send(frame).await.is_err() || failed {
                return;
            }
        }
    });
    let streams = tokio::spawn(accept_picture_streams(
        connection.clone(),
        pictures_tx,
        control.clone(),
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
