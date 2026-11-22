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
use super::negotiate::{CodecOffer, CodecSetting, MAX_CODEC_OFFERS};
use super::pixel_convert::FrameSize;
use super::virtual_display::{VirtualDisplayReport, VirtualDisplayWish};
use super::window_list::{MonitorInfo, SourceId};
use super::wire::{
    from_json, invalid, read_frame, recv_json, send_json, supports_version, APPROVAL_PENDING,
    CLIENT_HELLO, HELLO_ERROR, HELLO_OK, MAX_CONTROL_BYTES, SCREEN_PROTOCOL_VERSION, SERVER_HELLO,
};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::relation_proof::{relation_transcript, RelationClientHello, RelationServerHello};

/// Transcript prefix of the screen handshake (V7).
pub const SCREEN_TRANSCRIPT_PREFIX: &str = "smart-explorer/share-screen/v1|";
/// Protocol capability both relation hellos name; the screen hellos do not
/// carry it, both sides insert it, and it is part of the proven transcript.
pub const SCREEN_CAPABILITY: &str = "screen";
/// Largest display side a viewer may announce: the largest texture side of
/// current GPUs.
pub const MAX_DISPLAY_SIDE: u32 = 16_384;
fn is_false(value: &bool) -> bool {
    !*value
}

/// Limits a host names in `HELLO_OK` (SCR-M8).
#[allow(dead_code)] // Staged: the Android host of SCR-M8 names them.
pub const LIMIT_CONSENT_EACH_SESSION: &str = "consent_each_session";
#[allow(dead_code)] // Staged: the Android host of SCR-M8 names them.
pub const LIMIT_NO_KEY_EVENTS: &str = "no_key_events";
#[allow(dead_code)] // Staged: the Android host of SCR-M8 names them.
pub const LIMIT_STILL_MAX_3_PER_SECOND: &str = "still_max_3_per_second";

/// What the viewer asks for; decides the capabilities the host checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
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
    pub fn capabilities(self) -> &'static [Capability] {
        match self {
            Self::Probe | Self::Preview | Self::View => &[Capability::ScreenView],
            Self::Control => &[Capability::ScreenView, Capability::ScreenControl],
        }
    }

    /// Whether the host may ask its user for this mode. A probe and a tile
    /// preview never ask; under `Ask` the tile offers „Ansehen anfragen“ (C6).
    pub fn may_ask(self) -> bool {
        matches!(self, Self::View | Self::Control)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Preview => "preview",
            Self::View => "view",
            Self::Control => "control",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenServerHello {
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
    /// Pointer overlay (plan 2026-11-14 P6, `cursor_overlay.rs`): `Some(1)`
    /// when this host takes `CURSOR_REQUEST` after the hello.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_overlay: Option<u16>,
    /// Codec negotiation v2 (plan 2026-11-14 „Bildkette“ B-31): `Some(1)`
    /// when this host reads `decoders_v2` and answers with `encoders_v2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs_v2: Option<u16>,
    /// `rate_ceiling_v1`: `Some(1)` when this host takes `RATE_CEILING`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_ceiling: Option<u16>,
    /// `set_decoders_v1` (`set_decoders.rs`): `Some(1)` when this host takes
    /// `SET_DECODERS` and negotiates the pairing again in the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_decoders: Option<u16>,
    /// Virtual display (plan 2026-11-17 P5, `virtual_display.rs`): `Some(1)`
    /// when this host reads the viewer's `virtual_display` and answers it in
    /// `HELLO_OK`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<u16>,
}

#[path = "wire_hello_codecs.rs"]
mod codecs;
pub use codecs::{rate_ceiling_negotiated, CODECS_V2, RATE_CEILING_V1};

impl ScreenServerHello {
    pub fn new(
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
            cursor_overlay: Some(super::cursor_overlay::CURSOR_OVERLAY_V1),
            codecs_v2: Some(CODECS_V2),
            rate_ceiling: Some(RATE_CEILING_V1),
            set_decoders: Some(super::set_decoders::SET_DECODERS_V1),
            virtual_display: Some(super::virtual_display::VIRTUAL_DISPLAY_V1),
        }
    }

    /// The relation hello `prove_relation` checks.
    pub fn relation(&self) -> RelationServerHello {
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
pub struct ScreenClientHello {
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
    /// HEVC and AV1 decoders, only after the host's `codecs_v2` (B-31);
    /// `decoders` keeps H.264 and the tiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoders_v2: Option<Vec<CodecOffer>>,
    /// `gop_continue_v1` (B-36, `gop_continue.rs`): `Some(1)` when the viewer
    /// keeps its decoder over a continuation stream; an old host ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gop_continue: Option<u16>,
    /// The decoders of `decoders`/`decoders_v2` that keep their references
    /// over a continuation (`gop_continue.rs`, `ContinueOffers`): the host
    /// continues when the negotiated decoder is one of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gop_continue_decoders: Option<Vec<CodecOffer>>,
    /// The viewer's wish for the host's display (P5), only to a host that
    /// announced `virtual_display`; an older host ignores it, a viewer
    /// without it keeps the old behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<VirtualDisplayWish>,
    /// The viewer's fixed display size („Auflösung“); absent: the display
    /// follows `display_size` and every `SET_MODE` size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display_size: Option<FrameSize>,
    /// The viewer's „Codec“ (review 2026-11-19, Codex 3/6): its decoders
    /// name a chosen codec or „Scharf (GPU)“ only as long as they include
    /// both of H.264 and HEVC; this names it (`negotiate::preference_of`).
    /// Absent (an older viewer): the host reads the decoders as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec_choice: Option<CodecSetting>,
    /// „Höchste Datenrate“ in bit/s at the opening (review 2026-11-19,
    /// Codex 4/6): the host holds it from the first picture on, before the
    /// `RATE_CEILING` that follows the opening for older hosts arrives.
    /// Absent: automatic, or a viewer before the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rate_bps: Option<u64>,
    /// The host's sound (plan 2026-11-17 P9, `audio_wire.rs`): `Some(1)`
    /// when this viewer plays it; an older host ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<u16>,
    /// `Some(1)` (`control_parts::CONTROL_PARTS_V1`): this viewer takes a
    /// large control frame as `CONTROL_PART`s, between which small frames
    /// go (review round 10, Codex 10/10); an older host ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_parts: Option<u16>,
}

impl ScreenClientHello {
    /// The relation hello `prove_relation` checks.
    pub fn relation(&self) -> RelationClientHello {
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
    pub fn validate(&self) -> io::Result<()> {
        if !supports_version(self.protocol_version) {
            return Err(invalid("unsupported screen protocol version"));
        }
        if self.decoders.len() > MAX_CODEC_OFFERS {
            return Err(invalid("screen hello lists more than 32 decoders"));
        }
        codecs::validate_v2(self.decoders_v2.as_deref())?;
        codecs::validate_continuing(self.gop_continue_decoders.as_deref())?;
        if let Some(size) = self.display_size {
            let side = 1..=MAX_DISPLAY_SIDE;
            if !side.contains(&size.width) || !side.contains(&size.height) {
                return Err(invalid("screen display size is outside 1..=16384"));
            }
        }
        if self
            .virtual_display_size
            .is_some_and(|size| !super::virtual_display::side_valid(size))
        {
            return Err(invalid("virtual display size is outside 1..=16384"));
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
pub fn screen_transcript(server: &ScreenServerHello, client: &ScreenClientHello) -> String {
    relation_transcript(&server.relation(), &client.relation())
}

/// The decisions of the host for the proven viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenAuthorization {
    pub view: CapabilityDecision,
    pub control: CapabilityDecision,
}

/// How the session was approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    NotRequired,
    Once,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostPlatform {
    Windows,
    Linux,
    Android,
    /// A platform of a newer host.
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenHelloOk {
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
    /// HEVC and AV1 encoders, only for a viewer that sent `decoders_v2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoders_v2: Option<Vec<CodecOffer>>,
    /// What the host did with the viewer's `virtual_display` (P5); a probe
    /// names what a viewer that asks would get.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<VirtualDisplayReport>,
    /// The host sends its sound in this format after the viewer's `AUDIO`
    /// (P9); absent from older hosts and from hosts that cannot capture it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<super::audio_wire::AudioAccept>,
}

impl ScreenHelloOk {
    pub fn validate(&self) -> io::Result<()> {
        if let Some(contract) = self.frame_contract {
            contract.validate()?;
        }
        if self.encoders.len() > MAX_CODEC_OFFERS {
            return Err(invalid("screen hello lists more than 32 encoders"));
        }
        codecs::validate_v2(self.encoders_v2.as_deref())
    }

    /// The answer to the viewer's `virtual_display` (P5); an unsupported one
    /// is also a limit (`virtual_display_unsupported:<reason>`).
    pub fn set_virtual_display(&mut self, report: Option<VirtualDisplayReport>) {
        if let Some(limit) = report.as_ref().and_then(VirtualDisplayReport::limit) {
            if !self.limits.contains(&limit) {
                self.limits.push(limit);
            }
        }
        self.virtual_display = report;
    }
}

#[path = "wire_hello_errors.rs"]
mod errors;
pub use errors::{ScreenErrorCode, ScreenHelloError};

/// The host asks its user; the viewer waits or withdraws by closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenApprovalPending {
    pub capability: Capability,
}

/// The host's answer to `CLIENT_HELLO`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelloReply {
    Accepted(ScreenHelloOk),
    Refused(ScreenHelloError),
    Pending(ScreenApprovalPending),
}

pub async fn send_server_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &ScreenServerHello,
) -> io::Result<()> {
    send_json(writer, SERVER_HELLO, hello).await
}

pub async fn recv_server_hello<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ScreenServerHello> {
    recv_json(reader, SERVER_HELLO).await
}

pub async fn send_client_hello<W: AsyncWrite + Unpin>(
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
pub async fn recv_client_hello<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ScreenClientHello> {
    let hello: ScreenClientHello = recv_json(reader, CLIENT_HELLO).await?;
    hello.validate()?;
    Ok(hello)
}

pub async fn send_hello_reply<W: AsyncWrite + Unpin>(
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

pub async fn recv_hello_reply<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<HelloReply> {
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
pub(super) mod tests;
