//! The viewer side of a screen session (SCR-M2, decision C3). On a
//! connection with the screen ALPN the host opens the control stream and
//! writes `SERVER_HELLO` first (as in Exec: QUIC announces a stream only
//! with its first frame); the viewer answers with the client hello that the
//! share side signs (relation proof), allows the host its picture streams
//! (`VIEWER_UNI_STREAMS`), waits through `APPROVAL_PENDING` without a time
//! limit (C5) and then runs four tasks: the writer of the control stream,
//! the reader of the host's frames, the acceptor of the picture streams
//! (which acknowledges every frame) and the `PING` clock once a second
//! (`viewer_client_tasks.rs`). A device without the screen ALPN ends the TLS
//! handshake with alert 120, seen as transport code `0x178`:
//! `screen_unsupported` (NR N2); so does a host of another protocol version.
use std::io;
use std::time::Duration;

use iroh::endpoint::{Connection, VarInt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::gop_stream::PictureEvent;
use super::negotiate::CodecOffer;
use super::pixel_convert::FrameSize;
use super::window_list::SourceId;
use super::wire::supports_version;
use super::wire_control::{HostFrame, ViewerFrame};
use super::wire_hello::{
    recv_hello_reply, recv_server_hello, send_client_hello, HelloReply, ScreenClientHello,
    ScreenErrorCode, ScreenHelloOk, ScreenServerHello, SessionMode,
};
use crate::share::access_grant::Capability;

#[path = "viewer_client_refusal.rs"]
mod refusal;
use refusal::refusal_key;
pub use refusal::refusal_text;
#[path = "viewer_client_tasks.rs"]
mod tasks;

/// Time for the host's `SERVER_HELLO` (the Exec handshake allows the same).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// Interval of the viewer's `PING` (clock offset, liveness on the host).
const PING_INTERVAL: Duration = Duration::from_secs(1);
/// Interval while the viewer reported itself hidden: a third of the host's
/// 15-s silence limit (`host_session.rs` `VIEWER_SILENCE`), so a hidden
/// session stays alive without a wake-up every second on both devices.
const HIDDEN_PING_INTERVAL: Duration = Duration::from_secs(5);
/// Control frames queued for the writer, host frames for the reader.
const CONTROL_QUEUE: usize = 256;
/// Close code of a viewer that ends its session.
pub const VIEWER_CLOSE: VarInt = VarInt::from_u32(0);

/// What the viewer asks the host for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerRequest {
    pub mode: SessionMode,
    pub source: Option<SourceId>,
    pub decoders: Vec<CodecOffer>,
    pub display_size: Option<FrameSize>,
    pub max_fps: u16,
    pub frame_capabilities: Option<super::frame_budget::FrameCapabilities>,
    /// Announce `gop_continue_v1` (B-36): only for a decoding side that
    /// keeps its decoder over a continuation stream (`gop_continue.rs`).
    pub gop_continue: bool,
    /// The decoders that keep their references over a continuation
    /// (`gop_continue_decoders`, B-36 per codec); empty names none.
    pub gop_continue_decoders: Vec<CodecOffer>,
    /// The viewer's choice for the host's display (plan 2026-11-17 P5);
    /// `None` names none, as before.
    pub virtual_display: Option<super::virtual_display::VirtualDisplayChoice>,
    /// The viewer's „Codec“ for the hello (`ScreenClientHello::
    /// codec_choice`, review 2026-11-19 Codex 3/6); `None` names none.
    pub codec_choice: Option<super::negotiate::CodecSetting>,
    /// „Höchste Datenrate“ for the hello (`ScreenClientHello::max_rate_bps`);
    /// `None` is automatic.
    pub max_rate_bps: Option<u64>,
    /// „Übertragung“ for the hello (`ScreenClientHello::delivery`).
    pub delivery: Option<super::delivery::Delivery>,
}

/// Why a session did not start or ended.
#[derive(Debug)]
pub enum ViewerError {
    /// The host refused with this code key (`HELLO_ERROR`, `ERROR`).
    Refused(String),
    /// The device does not know the screen protocol or this version.
    Unsupported,
    Io(io::Error),
}

impl ViewerError {
    /// The code key as surfaces show it (`permission_denied`, …).
    pub fn code(&self) -> String {
        match self {
            // Only the key: a reason in words follows it after „: “.
            Self::Refused(code) => refusal_key(code).into(),
            Self::Unsupported => ScreenErrorCode::ScreenUnsupported.key().into(),
            Self::Io(_) => "connection_failed".into(),
        }
    }

    /// The refusal of an `ERROR` frame with the host's reason (`code:
    /// detail`; measurement of 2026-11-18: `se screen bench` dropped it).
    pub fn refused_by(error: &super::wire_control::ScreenWireError) -> Self {
        match error.detail.as_deref().map(str::trim) {
            Some(detail) if !detail.is_empty() => {
                Self::Refused(format!("{}: {detail}", error.code))
            }
            _ => Self::Refused(error.code.clone()),
        }
    }

    /// The text for a person: a codec reason of the host in German alone
    /// (`codec_reason_text`), another reason after the code's text.
    pub fn detail(&self) -> String {
        match self {
            Self::Refused(code) => match code.split_once(": ") {
                // A full metadata queue of the host is no failed capture
                // (round 10, `host_session_control.rs`): it may be asked
                // again; its key stays for scripts (CLI `--json`, MCP).
                Some((_, reason)) if reason.starts_with("metadata_queue_full") => {
                    "Zu viele Anfragen warten auf das Gerät; bitte erneut versuchen \
                     (metadata_queue_full)"
                        .into()
                }
                Some((key, reason)) => super::codec_reason_text::chosen_codec_text(reason)
                    .unwrap_or_else(|| format!("{}: {reason}", refusal_text(key))),
                None => refusal_text(code),
            },
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

/// One running session of this viewer. Dropping it ends its tasks; `close`
/// tells the host.
pub struct ViewerSession {
    pub connection: Connection,
    pub hello: ScreenHelloOk,
    /// Frames for the host (`SET_MODE`, `INPUT`, …).
    pub control: mpsc::Sender<ViewerFrame>,
    /// The host's control frames; an error ends the stream.
    pub host: mpsc::Receiver<io::Result<HostFrame>>,
    pub pictures: mpsc::Receiver<PictureEvent>,
    tasks: Vec<JoinHandle<()>>,
    pub memory: super::frame_memory::MemoryHold,
    /// Local allocation evidence; never serialized as a remote frame contract.
    pub local_limits: Option<super::frame_budget::FrameLimits>,
    /// The viewer asked for the pointer overlay (P6); a reconnect asks again.
    pub cursor_wish: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The host announced `rate_ceiling_v1` (plan 2026-11-14 „Bildkette“
    /// BK6): only then does `RATE_CEILING` leave this viewer.
    pub rate_ceiling: bool,
    /// The host announced `set_decoders_v1` (P1): only then does
    /// `SET_DECODERS` leave this viewer; without it a new „Codec“ or
    /// „Dekodierer“ opens the session again.
    pub set_decoders: bool,
    /// The host's sound datagrams while the picture task reads the
    /// datagrams (`video_transport.rs`); `take_audio`.
    audio: Option<mpsc::Receiver<bytes::Bytes>>,
}

impl ViewerSession {
    pub fn close(&self) {
        self.connection.close(VIEWER_CLOSE, b"viewer closed");
    }

    /// The sound datagrams, when the session's picture task reads the
    /// connection's datagrams; `None`: read them from the connection.
    pub fn take_audio(&mut self) -> Option<mpsc::Receiver<bytes::Bytes>> {
        self.audio.take()
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
pub async fn handshake(
    connection: Connection,
    build: impl FnOnce(&ScreenServerHello) -> io::Result<ScreenClientHello>,
    on_pending: impl FnMut(Capability),
) -> Result<ViewerSession, ViewerError> {
    handshake_for(
        connection,
        |server| build(server).map(|hello| (hello, None)),
        on_pending,
        false,
    )
    .await
}

/// A list/icon channel. Older hosts retain LIST_WINDOWS over the synthetic source only.
pub async fn handshake_metadata(
    connection: Connection,
    build: impl FnOnce(&ScreenServerHello) -> io::Result<ScreenClientHello>,
    on_pending: impl FnMut(Capability),
) -> Result<ViewerSession, ViewerError> {
    handshake_for(
        connection,
        |server| build(server).map(|hello| (hello, None)),
        on_pending,
        true,
    )
    .await
}

pub async fn handshake_admitted(
    connection: Connection,
    build: impl FnOnce(
        &ScreenServerHello,
    ) -> io::Result<(ScreenClientHello, super::frame_memory::MemoryHold)>,
    on_pending: impl FnMut(Capability),
) -> Result<ViewerSession, ViewerError> {
    handshake_for(connection, build, on_pending, false).await
}

async fn handshake_for(
    connection: Connection,
    build: impl FnOnce(
        &ScreenServerHello,
    ) -> io::Result<(ScreenClientHello, super::frame_memory::MemoryHold)>,
    mut on_pending: impl FnMut(Capability),
    metadata: bool,
) -> Result<ViewerSession, ViewerError> {
    let (mut send, mut recv) =
        crate::share::route_protocol::run_with_route(&connection, HANDSHAKE_TIMEOUT, None, async {
            connection.accept_bi().await.map_err(io::Error::other)
        })
        .await?;
    let server = crate::share::route_protocol::run_with_route(
        &connection,
        HANDSHAKE_TIMEOUT,
        None,
        recv_server_hello(&mut recv),
    )
    .await?;
    if !supports_version(server.protocol_version) {
        connection.close(VIEWER_CLOSE, b"screen protocol version");
        return Err(ViewerError::Unsupported);
    }
    let (mut hello, mut memory) = build(&server)?;
    // B-31: H.264 and the tiles in `decoders`, HEVC and AV1 in
    // `decoders_v2` only for a host that announced `codecs_v2`.
    let decoders = hello.all_decoders();
    hello.offer_decoders(&decoders, &server);
    hello.frame_capabilities = hello
        .frame_capabilities
        .map(|caps| caps.for_server(server.frame_memory_class))
        .transpose()?
        .map(|mut caps| {
            // Hybrid regions (video for changed areas, lossless tiles for
            // stable ones) stay off until they work on every host: on
            // 2026-10-27 they left about 1 picture/s and a stale right third
            // from Android and flickering, mostly black pictures from Linux
            // and Windows hosts. The plain picture stream is the 0.5.184 path.
            caps.hybrid_regions = false;
            caps
        });
    // Large control frames in parts (round 10, `control_parts.rs`).
    hello.control_parts = Some(super::control_parts::CONTROL_PARTS_V1);
    hello.family_version = server
        .family_version
        .filter(|version| *version == super::family_protocol::FAMILY_PROTOCOL);
    if hello
        .source
        .as_ref()
        .is_some_and(|source| super::family_protocol::selector(source).is_some())
        && hello.family_version.is_none()
    {
        return Err(ViewerError::Unsupported);
    }
    hello.icon_version = server
        .icon_version
        .filter(|version| *version == super::app_icon::ICON_VERSION);
    hello.metadata_only =
        metadata && (hello.icon_version.is_some() || hello.family_version.is_some());
    if metadata {
        hello.mode = SessionMode::View;
        hello.source = Some(SourceId::Synthetic);
        hello.frame_capabilities = None;
    }
    if !metadata && server.frame_memory_class.is_none() {
        if let Some(lease) = &memory {
            let limits = super::frame_budget::legacy_limits(
                lease.bytes(),
                hello.frame_capabilities.map(|caps| caps.texture_size),
            )?;
            // Existing v1 hosts honor display_size but cannot interpret v2's
            // memory offer. Ask them for pixels the actual local lease can hold.
            let mut wanted = limits.fit(hello.display_size.unwrap_or(FrameSize::MAX));
            if limits.max_pixels < 256 || limits.max_size.width < 16 || limits.max_size.height < 16
            {
                hello.decoders.retain(|offer| !offer.codec.is_video());
            } else if hello.decoders.iter().any(|offer| offer.codec.is_video()) {
                wanted = super::frame_size_caps::fit_macroblocks(wanted, limits.max_pixels / 256);
            }
            hello.display_size = Some(wanted);
        }
    }
    let probe = hello.mode == SessionMode::Probe;
    // The picture streams and, announced in the hello, the host's probe.
    let streams = super::bandwidth_probe::viewer_streams(&mut hello, metadata);
    connection.set_max_concurrent_uni_streams(VarInt::from_u32(streams));
    send_client_hello(&mut send, &hello).await?;
    let accepted = loop {
        match recv_hello_reply(&mut recv).await? {
            HelloReply::Accepted(accepted) => break accepted,
            HelloReply::Pending(pending) => on_pending(pending.capability),
            HelloReply::Refused(refused) => {
                connection.close(VIEWER_CLOSE, b"refused");
                // The host's reason in words travels with the code
                // (`encoder_failed: chosen_codec_unavailable: av1: …`); a
                // refusal without one keeps the bare code key.
                let reason = match refused.message {
                    Some(message) => format!("{}: {message}", refused.code),
                    None => refused.code,
                };
                return Err(ViewerError::Refused(reason));
            }
        }
    };
    if accepted.icon_version.is_some() && accepted.icon_version != hello.icon_version {
        return Err(ViewerError::Io(io::Error::other(
            "host enabled an unoffered icon contract",
        )));
    }
    if accepted.family_version.is_some() && accepted.family_version != hello.family_version {
        return Err(ViewerError::Io(io::Error::other(
            "host enabled an unoffered window family protocol",
        )));
    }
    if hello
        .source
        .as_ref()
        .is_some_and(|source| super::family_protocol::selector(source).is_some())
        && accepted.family_version.is_none()
    {
        return Err(ViewerError::Unsupported);
    }
    if let Some(contract) = accepted.frame_contract {
        let offered = hello
            .frame_capabilities
            .ok_or_else(|| io::Error::other("host enabled an unoffered frame protocol"))?;
        offered.validate()?;
        if contract.version != offered.version
            || (contract.hybrid_regions && !offered.hybrid_regions)
            || contract.limits.max_size.width
                > offered
                    .frames
                    .max_size
                    .width
                    .min(offered.texture_size.width)
            || contract.limits.max_size.height
                > offered
                    .frames
                    .max_size
                    .height
                    .min(offered.texture_size.height)
            || contract.limits.max_pixels > offered.frames.max_pixels
            || contract.limits.working_bytes > offered.frames.working_bytes
            || memory
                .as_ref()
                .is_some_and(|lease| contract.limits.working_bytes > lease.bytes())
        {
            return Err(ViewerError::Io(io::Error::other(
                "host frame contract exceeds viewer's capability evidence",
            )));
        }
        if let Some(lease) = memory.as_mut().and_then(std::sync::Arc::get_mut) {
            lease.release_to(contract.limits.working_bytes);
        }
    }
    let local_limits = if metadata {
        None
    } else if let Some(contract) = accepted.frame_contract {
        Some(contract.limits)
    } else {
        memory
            .as_ref()
            .map(|lease| {
                super::frame_budget::legacy_limits(
                    lease.bytes(),
                    hello.frame_capabilities.map(|caps| caps.texture_size),
                )
            })
            .transpose()?
    };
    let presence = super::gop_stream::PicturePresence::viewer(hello.mode, metadata);
    let cursor_wish = std::sync::Arc::<std::sync::atomic::AtomicBool>::default();
    let tasks::Running {
        control,
        host,
        pictures,
        audio,
        tasks,
    } = tasks::spawn(
        tasks::Start {
            connection: connection.clone(),
            send,
            recv,
            probe,
            presence,
            memory: memory.clone(),
            local_limits,
            cursor_overlay: super::cursor_overlay::negotiated(&server, hello.mode, metadata),
            cursor_wish: cursor_wish.clone(),
            rate_ceiling: super::wire_hello::rate_ceiling_negotiated(&server) && !metadata,
            set_decoders: super::set_decoders::negotiated(&server) && !metadata,
            delivery: super::bandwidth_probe::delivery_negotiated(&server) && !metadata,
            video: super::video_transport::viewer_carriers(&server, hello.mode, metadata),
            tcp: super::video_transport::viewer_tcp(&server, hello.mode, metadata),
        },
        &accepted,
    );
    let rate_ceiling = super::wire_hello::rate_ceiling_negotiated(&server) && !metadata;
    let set_decoders = super::set_decoders::negotiated(&server) && !metadata;
    Ok(ViewerSession {
        connection,
        hello: accepted,
        control,
        host,
        pictures,
        tasks,
        memory,
        local_limits,
        cursor_wish,
        rate_ceiling,
        set_decoders,
        audio,
    })
}

/// Whether a connect error means the device does not know the screen ALPN:
/// TLS alert 120 (`no_application_protocol`) is QUIC transport code `0x178`.
pub fn is_unsupported_alpn(error: &(dyn std::error::Error + 'static)) -> bool {
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

#[cfg(test)]
#[path = "rollout_error_reason_task_tests.rs"]
mod rollout_error_reason_task_tests;
