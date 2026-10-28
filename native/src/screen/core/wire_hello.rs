//! Hellos of the screen protocol (decisions C3 and C7, contract V7). The host
//! announces itself (`SERVER_HELLO`), the viewer proves its relation and
//! names what it wants (`CLIENT_HELLO`), the host answers `HELLO_OK` or
//! `HELLO_ERROR`, optionally after `APPROVAL_PENDING`. Both hellos map onto
//! the protocol-neutral relation hellos of S1 with the transcript prefix
//! `smart-explorer/share-screen/v1|`, so `prove_relation` checks the screen
//! handshake exactly like the Exec handshake.
use std::io;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use super::input_model::InputCapabilities;
use super::negotiate::{CodecOffer, MAX_CODEC_OFFERS};
use super::pixel_convert::FrameSize;
use super::window_list::{MonitorInfo, SourceId};
use super::wire::{
    from_json, invalid, read_frame, recv_json, send_json, supports_version, APPROVAL_PENDING,
    CLIENT_HELLO, HELLO_ERROR, HELLO_OK, MAX_CONTROL_BYTES, SCREEN_PROTOCOL_VERSION, SERVER_HELLO,
};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::relation_proof::{relation_transcript, RelationClientHello, RelationServerHello};

/// Transcript prefix of the screen handshake (V7).
pub(crate) const SCREEN_TRANSCRIPT_PREFIX: &str = "smart-explorer/share-screen/v1|";
/// Protocol capability both relation hellos name; the screen hellos do not
/// carry it, both sides insert it, and it is part of the proven transcript.
pub(crate) const SCREEN_CAPABILITY: &str = "screen";
/// Largest display side a viewer may announce: the largest texture side of
/// current GPUs.
pub(crate) const MAX_DISPLAY_SIDE: u32 = 16_384;
fn is_false(value: &bool) -> bool {
    !*value
}

/// Limits a host names in `HELLO_OK` (SCR-M8).
#[allow(dead_code)] // Staged: the Android host of SCR-M8 names them.
pub(crate) const LIMIT_CONSENT_EACH_SESSION: &str = "consent_each_session";
#[allow(dead_code)] // Staged: the Android host of SCR-M8 names them.
pub(crate) const LIMIT_NO_KEY_EVENTS: &str = "no_key_events";
#[allow(dead_code)] // Staged: the Android host of SCR-M8 names them.
pub(crate) const LIMIT_STILL_MAX_3_PER_SECOND: &str = "still_max_3_per_second";

/// What the viewer asks for; decides the capabilities the host checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionMode {
    /// Rights, platform, codecs, monitors and limits, then the end; no
    /// stream (`se screen probe`, `se screen devices --probe`).
    Probe,
    /// Tile previews of the group view (C6).
    Preview,
    /// The full stream.
    View,
    /// The full stream and the right to take control.
    Control,
}

impl SessionMode {
    /// Capabilities the host requires; control requires a view (C7).
    pub(crate) fn capabilities(self) -> &'static [Capability] {
        match self {
            Self::Probe | Self::Preview | Self::View => &[Capability::ScreenView],
            Self::Control => &[Capability::ScreenView, Capability::ScreenControl],
        }
    }

    /// Whether the host may ask its user for this mode. A probe and a tile
    /// preview never ask; under `Ask` the tile offers „Ansehen anfragen“ (C6).
    pub(crate) fn may_ask(self) -> bool {
        matches!(self, Self::View | Self::Control)
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Preview => "preview",
            Self::View => "view",
            Self::Control => "control",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScreenServerHello {
    /// Protocol support only; the subsequent offers carry actual resource evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_memory_class: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_version: Option<u16>,
    pub protocol_version: u16,
    pub challenge: String,
    pub server_device_id: String,
    pub server_public_key: String,
    pub server_fingerprint: String,
    pub server_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_version: Option<u8>,
    /// Device feed (design 5.2): `Some(1)` when this host answers
    /// `SUBSCRIBE_FEED` on a metadata channel. The viewer's subscription
    /// frame is its opt-in, so the client hello needs no field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_feed_version: Option<u16>,
}

impl ScreenServerHello {
    pub(crate) fn new(
        challenge: String,
        server_device_id: String,
        server_public_key: String,
        server_fingerprint: String,
        server_node_id: String,
    ) -> Self {
        Self {
            frame_memory_class: Some(super::frame_budget::EXTENDED_MEMORY_CLASS),
            protocol_version: SCREEN_PROTOCOL_VERSION,
            challenge,
            server_device_id,
            server_public_key,
            server_fingerprint,
            server_node_id,
            icon_version: Some(super::app_icon::ICON_VERSION),
            family_version: Some(super::family_protocol::FAMILY_PROTOCOL),
            family_feed_version: Some(super::wire::FAMILY_FEED_VERSION),
        }
    }

    /// The relation hello `prove_relation` checks.
    pub(crate) fn relation(&self) -> RelationServerHello {
        RelationServerHello {
            transcript_prefix: SCREEN_TRANSCRIPT_PREFIX.into(),
            protocol_version: self.protocol_version,
            capability: SCREEN_CAPABILITY.into(),
            challenge: self.challenge.clone(),
            server_device_id: self.server_device_id.clone(),
            server_public_key: self.server_public_key.clone(),
            server_fingerprint: self.server_fingerprint.clone(),
            server_node_id: self.server_node_id.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScreenClientHello {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_version: Option<u16>,
    pub protocol_version: u16,
    /// `direct` or `room`.
    pub relation_kind: String,
    pub relation_id: String,
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
    pub client_nonce: String,
    /// HMAC over `screen_transcript`.
    pub proof: String,
    pub mode: SessionMode,
    /// `None` = the primary monitor.
    #[serde(default)]
    pub source: Option<SourceId>,
    #[serde(default)]
    pub decoders: Vec<CodecOffer>,
    #[serde(default)]
    pub display_size: Option<FrameSize>,
    pub max_fps: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_capabilities: Option<super::frame_budget::FrameCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_version: Option<u8>,
    /// List/icon operations only: no source, encoder, capture or indicator is opened.
    #[serde(default, skip_serializing_if = "is_false")]
    pub metadata_only: bool,
}

impl ScreenClientHello {
    /// The relation hello `prove_relation` checks.
    pub(crate) fn relation(&self) -> RelationClientHello {
        RelationClientHello {
            protocol_version: self.protocol_version,
            capability: SCREEN_CAPABILITY.into(),
            relation_kind: self.relation_kind.clone(),
            relation_id: self.relation_id.clone(),
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            public_key: self.public_key.clone(),
            fingerprint: self.fingerprint.clone(),
            node_id: self.node_id.clone(),
            client_nonce: self.client_nonce.clone(),
            proof: self.proof.clone(),
        }
    }

    /// Checks the fields `prove_relation` does not check.
    pub(crate) fn validate(&self) -> io::Result<()> {
        if !supports_version(self.protocol_version) {
            return Err(invalid("unsupported screen protocol version"));
        }
        if self.decoders.len() > MAX_CODEC_OFFERS {
            return Err(invalid("screen hello lists more than 32 decoders"));
        }
        if let Some(size) = self.display_size {
            let side = 1..=MAX_DISPLAY_SIDE;
            if !side.contains(&size.width) || !side.contains(&size.height) {
                return Err(invalid("screen display size is outside 1..=16384"));
            }
        }
        if let Some(caps) = self.frame_capabilities {
            caps.validate()?;
        }
        if self.metadata_only
            && (self.mode != SessionMode::View
                || (self.icon_version != Some(super::app_icon::ICON_VERSION)
                    && self.family_version != Some(super::family_protocol::FAMILY_PROTOCOL)))
        {
            return Err(invalid(
                "metadata requires the negotiated icon contract and view permission",
            ));
        }
        if self.max_fps == 0 {
            return Err(invalid("screen frame rate must be at least 1"));
        }
        Ok(())
    }
}

/// The text the viewer's proof is an HMAC over (`relation_transcript` with
/// the screen prefix and capability).
pub(crate) fn screen_transcript(server: &ScreenServerHello, client: &ScreenClientHello) -> String {
    relation_transcript(&server.relation(), &client.relation())
}

/// The decisions of the host for the proven viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScreenAuthorization {
    pub view: CapabilityDecision,
    pub control: CapabilityDecision,
}

/// How the session was approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalKind {
    NotRequired,
    Once,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostPlatform {
    Windows,
    Linux,
    Android,
    /// A platform of a newer host.
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScreenHelloOk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_version: Option<u16>,
    pub authorization: ScreenAuthorization,
    pub platform: HostPlatform,
    #[serde(default)]
    pub encoders: Vec<CodecOffer>,
    #[serde(default)]
    pub input: InputCapabilities,
    #[serde(default)]
    pub monitors: Vec<MonitorInfo>,
    #[serde(default)]
    pub limits: Vec<String>,
    pub approval: ApprovalKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_contract: Option<super::frame_budget::FrameContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_version: Option<u8>,
}

impl ScreenHelloOk {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if let Some(contract) = self.frame_contract {
            contract.validate()?;
        }
        if self.encoders.len() > MAX_CODEC_OFFERS {
            return Err(invalid("screen hello lists more than 32 encoders"));
        }
        Ok(())
    }
}

/// Error codes of `HELLO_ERROR` and `ERROR` (C7). The wire carries the key,
/// so a code of a newer host survives as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ScreenErrorCode {
    /// Before the proof or without the right; strangers learn nothing.
    PermissionDenied,
    /// Control asked for, only viewing allowed.
    RightNotGranted,
    ApprovalDenied,
    ApprovalCancelled,
    NoDisplay,
    /// The host has no desktop app to show the indicator (C7).
    IndicatorUnavailable,
    ControlBusy,
    ControlUnavailable,
    HostShutdown,
    /// The device does not know the screen ALPN (viewer side, `0x178`), or a
    /// hello names a protocol version the other side does not speak.
    ScreenUnsupported,
    /// Nobody on the host can be asked (S1 C5).
    ApprovalUnavailable,
    /// The host's capture failed during the session (`ERROR`).
    CaptureFailed,
    /// The host's encoder failed or none fits (`ERROR`, `HELLO_ERROR`).
    EncoderFailed,
    /// Sent only by hosts before R15c, which refused past 8 sessions (4 of
    /// one device); current hosts have no session count. Kept so viewers
    /// still read that refusal.
    SessionLimit,
}

impl ScreenErrorCode {
    pub(crate) const ALL: [Self; 14] = [
        Self::PermissionDenied,
        Self::RightNotGranted,
        Self::ApprovalDenied,
        Self::ApprovalCancelled,
        Self::NoDisplay,
        Self::IndicatorUnavailable,
        Self::ControlBusy,
        Self::ControlUnavailable,
        Self::HostShutdown,
        Self::ScreenUnsupported,
        Self::ApprovalUnavailable,
        Self::CaptureFailed,
        Self::EncoderFailed,
        Self::SessionLimit,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::RightNotGranted => "right_not_granted",
            Self::ApprovalDenied => "approval_denied",
            Self::ApprovalCancelled => "approval_cancelled",
            Self::NoDisplay => "no_display",
            Self::IndicatorUnavailable => "indicator_unavailable",
            Self::ControlBusy => "control_busy",
            Self::ControlUnavailable => "control_unavailable",
            Self::HostShutdown => "host_shutdown",
            Self::ScreenUnsupported => "screen_unsupported",
            Self::ApprovalUnavailable => "approval_unavailable",
            Self::CaptureFailed => "capture_failed",
            Self::EncoderFailed => "encoder_failed",
            Self::SessionLimit => "session_limit",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScreenHelloError {
    pub code: String,
}

impl ScreenHelloError {
    pub(crate) fn new(code: ScreenErrorCode) -> Self {
        Self {
            code: code.key().into(),
        }
    }

    #[allow(dead_code)] // Staged: the desktop viewer of SCR-M7 shows known codes.
    pub(crate) fn known_code(&self) -> Option<ScreenErrorCode> {
        ScreenErrorCode::from_key(&self.code)
    }
}

/// The host asks its user; the viewer waits or withdraws by closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScreenApprovalPending {
    pub capability: Capability,
}

/// The host's answer to `CLIENT_HELLO`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HelloReply {
    Accepted(ScreenHelloOk),
    Refused(ScreenHelloError),
    Pending(ScreenApprovalPending),
}

pub(crate) async fn send_server_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &ScreenServerHello,
) -> io::Result<()> {
    send_json(writer, SERVER_HELLO, hello).await
}

pub(crate) async fn recv_server_hello<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ScreenServerHello> {
    recv_json(reader, SERVER_HELLO).await
}

pub(crate) async fn send_client_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &ScreenClientHello,
) -> io::Result<()> {
    hello.validate()?;
    send_json(writer, CLIENT_HELLO, hello).await
}

// The host reads the hello with `recv_json` (`share/core/node_screen.rs`), so
// that an unknown version gets `screen_unsupported` before the field checks;
// the wire tests use this one.
#[allow(dead_code)]
pub(crate) async fn recv_client_hello<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ScreenClientHello> {
    let hello: ScreenClientHello = recv_json(reader, CLIENT_HELLO).await?;
    hello.validate()?;
    Ok(hello)
}

pub(crate) async fn send_hello_reply<W: AsyncWrite + Unpin>(
    writer: &mut W,
    reply: &HelloReply,
) -> io::Result<()> {
    match reply {
        HelloReply::Accepted(accepted) => {
            accepted.validate()?;
            send_json(writer, HELLO_OK, accepted).await
        }
        HelloReply::Refused(refused) => send_json(writer, HELLO_ERROR, refused).await,
        HelloReply::Pending(pending) => send_json(writer, APPROVAL_PENDING, pending).await,
    }
}

pub(crate) async fn recv_hello_reply<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<HelloReply> {
    let (tag, payload) = read_frame(reader, MAX_CONTROL_BYTES).await?;
    match tag {
        HELLO_OK => {
            let accepted: ScreenHelloOk = from_json(&payload)?;
            accepted.validate()?;
            Ok(HelloReply::Accepted(accepted))
        }
        HELLO_ERROR => Ok(HelloReply::Refused(from_json(&payload)?)),
        APPROVAL_PENDING => Ok(HelloReply::Pending(from_json(&payload)?)),
        _ => Err(invalid("unexpected screen handshake frame")),
    }
}

#[cfg(test)]
#[path = "wire_hello_tests.rs"]
mod tests;
