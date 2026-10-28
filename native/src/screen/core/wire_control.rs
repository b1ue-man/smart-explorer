//! Control frames after the handshake (decision C3). The viewer sends
//! `SET_MODE`, `INPUT`, `FRAME_ACK`, `KEYFRAME_REQUEST`, `PING`,
//! `LIST_WINDOWS`, `SNAPSHOT`, `CONTROL_REQUEST` and `PREVIEW_ACK`; the host
//! sends `STATE`, `PONG`, `WINDOWS`, `STATS`, `CONTROL_STATE` and `ERROR`.
//! Frames that come with every picture or input event have a fixed
//! big-endian layout (`INPUT`; `FRAME_ACK` and `PREVIEW_ACK` 24 bytes;
//! `PING` 16 bytes; `PONG` 32 bytes); the others are JSON. A tag of the
//! other direction or of the handshake is refused.
use std::io;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use super::app_icon::{IconRequest, IconResponse};
use super::input_model::InputEvent;
use super::negotiate::{CodecKind, H264Profile};
use super::pixel_convert::FrameSize;
use super::rate_control::PathKind;
use super::session_state::SessionState;
use super::visibility::{ViewerVisibility, VisibilityAck};
use super::window_list::{SourceId, WindowList};
use super::wire::{
    invalid, read_frame, write_frame, BeReader, MAX_CONTROL_BYTES, MAX_WINDOWS_BYTES,
};
use super::wire_hello::ScreenErrorCode;

#[path = "wire_control_codec.rs"]
mod codec;
pub(crate) use codec::{
    decode_host_frame, decode_viewer_frame, encode_host_frame, encode_viewer_frame,
};

/// The stream kind `SET_MODE` asks for: a click in the group view raises
/// the same connection from `preview` to `full` (C6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StreamMode {
    Preview,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SetMode {
    pub mode: StreamMode,
    #[serde(default)]
    pub source: Option<SourceId>,
    /// Wanted picture size for this mode. Omission retains its previous choice;
    /// preview and full each retain their own explicit size.
    #[serde(default)]
    pub size: Option<FrameSize>,
    #[serde(default)]
    pub fps: Option<u16>,
    /// „Schärfe vor Flüssigkeit“: at most 15 frames/s at full resolution.
    #[serde(default)]
    pub sharpness: bool,
    /// „Verlustfrei“: the tile codec.
    #[serde(default)]
    pub lossless: bool,
}

impl SetMode {
    fn validate(&self) -> io::Result<()> {
        let empty = self
            .size
            .is_some_and(|size| size.width == 0 || size.height == 0);
        if empty || self.fps == Some(0) {
            return Err(invalid(
                "SET_MODE needs a non-empty size and a frame rate of at least 1",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeyframeReason {
    DecodeError,
    Join,
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeyframeRequest {
    pub reason: KeyframeReason,
    #[serde(default)]
    pub gop_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SnapshotRequest {
    #[serde(default)]
    pub source: Option<SourceId>,
    #[serde(default)]
    pub max_size: Option<FrameSize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ControlAction {
    Take,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ControlRequest {
    pub action: ControlAction,
}

/// `FRAME_ACK` and `PREVIEW_ACK`: the last record of `frame_id` arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameAck {
    pub gop_id: u64,
    pub frame_id: u64,
    /// Viewer clock (µs) when the last record of the frame arrived.
    pub received_us: u64,
}

fn encode_u64s(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
}

fn decode_u64s<const N: usize>(payload: &[u8]) -> io::Result<[u64; N]> {
    let mut reader = BeReader::new(payload);
    let mut values = [0u64; N];
    for value in &mut values {
        *value = reader.u64()?;
    }
    reader.finish()?;
    Ok(values)
}

impl FrameAck {
    fn encode(&self) -> Vec<u8> {
        encode_u64s(&[self.gop_id, self.frame_id, self.received_us])
    }

    fn decode(payload: &[u8]) -> io::Result<Self> {
        let [gop_id, frame_id, received_us] = decode_u64s::<3>(payload)?;
        Ok(Self {
            gop_id,
            frame_id,
            received_us,
        })
    }
}

/// Clock probe of the viewer (C9: clock offset from the smallest round trip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ping {
    pub sequence: u64,
    /// Viewer clock (µs) when the `PING` was sent.
    pub sent_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pong {
    pub sequence: u64,
    /// `Ping::sent_us`, echoed.
    pub ping_sent_us: u64,
    /// Host clock (µs) when the `PING` arrived.
    pub received_us: u64,
    /// Host clock (µs) when the `PONG` was sent.
    pub sent_us: u64,
}

impl Pong {
    pub(crate) fn answer(ping: &Ping, received_us: u64, sent_us: u64) -> Self {
        Self {
            sequence: ping.sequence,
            ping_sent_us: ping.sent_us,
            received_us,
            sent_us,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StreamInfo {
    pub codec: CodecKind,
    #[serde(default)]
    pub profile: Option<H264Profile>,
    pub size: FrameSize,
    /// Encoder name for the statistics, e.g. the MFT or `openh264`.
    pub encoder: String,
    #[serde(default)]
    pub hardware: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StateReport {
    pub state: SessionState,
    /// Reason key: `pipewire_missing` for `no_display`, an `EndReason` key
    /// for `ended`.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub stream: Option<StreamInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HostStats {
    pub target_kbps: u64,
    pub sent_kbps: u64,
    pub fps: u32,
    pub path: PathKind,
    pub keyframes: u64,
    pub dropped_before_encode: u64,
    pub stream_resets: u64,
    pub unacked_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ControlHolder {
    pub device_id: String,
    pub device_name: String,
}

/// Why input does not reach the host (C5, SCR-M6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ControlBlock {
    ElevatedWindow,
    SecureDesktop,
    ControlUnavailable,
    UnsupportedOnHost,
    /// A reason of a newer host.
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ControlState {
    #[serde(default)]
    pub holder: Option<ControlHolder>,
    #[serde(default)]
    pub blocked: Option<ControlBlock>,
}

/// `ERROR` during a session. `code` is a `ScreenErrorCode` key or a code of
/// a newer host; `detail` is diagnostic text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScreenWireError {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ScreenWireError {
    pub(crate) fn new(code: ScreenErrorCode, detail: Option<String>) -> Self {
        Self {
            code: code.key().into(),
            detail,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ViewerFrame {
    ListFamilies,
    FamilySnapshot,
    FamilyOperation(super::family_protocol::FamilyOperation),
    FamilyInput(super::family_protocol::FamilyInput),
    /// Complete frame adopted for presentation; only after a v2 contract.
    Presented {
        gop_id: u64,
        frame_id: u64,
    },
    Visibility(ViewerVisibility),
    SetMode(SetMode),
    Input(Vec<InputEvent>),
    FrameAck(FrameAck),
    KeyframeRequest(KeyframeRequest),
    Ping(Ping),
    ListWindows,
    IconRequest(IconRequest),
    Snapshot(SnapshotRequest),
    ControlRequest(ControlRequest),
    PreviewAck(FrameAck),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HostFrame {
    Families(super::family_protocol::FamilyListing),
    FamilySnapshot(crate::remote_window::FamilySnapshot),
    FamilyOperation(super::family_protocol::FamilyOperationResult),
    FamilyInput(super::family_protocol::FamilyInputResult),
    VisibilityAck(VisibilityAck),
    State(StateReport),
    Pong(Pong),
    Windows(WindowList),
    IconReply(IconResponse),
    Stats(HostStats),
    ControlState(ControlState),
    Error(ScreenWireError),
}

pub(crate) async fn send_viewer_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &ViewerFrame,
) -> io::Result<()> {
    let (tag, payload) = encode_viewer_frame(frame)?;
    write_frame(writer, tag, &payload).await
}

/// Host side: viewer frames are at most 64 KiB.
pub(crate) async fn recv_viewer_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ViewerFrame> {
    let (tag, payload) = read_frame(reader, MAX_CONTROL_BYTES).await?;
    decode_viewer_frame(tag, &payload)
}

/// A frame on a capture-free metadata channel: a regular viewer frame or the
/// device-feed subscription (design 5.2). The subscription is no
/// `ViewerFrame` variant, so every existing frame match stays exhaustive.
pub(crate) enum MetadataFrame {
    Viewer(ViewerFrame),
    SubscribeFeed,
}

/// Host side of a metadata channel; `SUBSCRIBE_FEED` carries `{}`.
pub(crate) async fn recv_metadata_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<MetadataFrame> {
    let (tag, payload) = read_frame(reader, MAX_CONTROL_BYTES).await?;
    if tag == super::wire::SUBSCRIBE_FEED {
        super::wire::from_json::<serde_json::Map<String, serde_json::Value>>(&payload)?;
        return Ok(MetadataFrame::SubscribeFeed);
    }
    decode_viewer_frame(tag, &payload).map(MetadataFrame::Viewer)
}

/// Viewer side: subscribes a metadata channel to the host's device feed.
/// Only after the host announced `family_feed_version` in its hello.
// Consumer: the viewer's device feed (work package WP2, `peer_feed.rs`).
#[allow(dead_code)]
pub(crate) async fn send_subscribe_feed<W: AsyncWrite + Unpin>(writer: &mut W) -> io::Result<()> {
    write_frame(writer, super::wire::SUBSCRIBE_FEED, b"{}").await
}

pub(crate) async fn send_host_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &HostFrame,
) -> io::Result<()> {
    let (tag, payload) = encode_host_frame(frame)?;
    write_frame(writer, tag, &payload).await
}

/// Viewer side: WINDOWS keeps 1 MiB; ICON_REPLY has its smaller dedicated
/// budget; the remaining host frames stay below 64 KiB.
pub(crate) async fn recv_host_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<HostFrame> {
    let (tag, payload) = read_frame(reader, MAX_WINDOWS_BYTES).await?;
    decode_host_frame(tag, &payload)
}
