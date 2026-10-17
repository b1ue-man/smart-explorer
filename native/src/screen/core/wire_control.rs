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

use super::input_model::{decode_batch, encode_batch, InputEvent};
use super::negotiate::{CodecKind, H264Profile};
use super::pixel_convert::FrameSize;
use super::rate_control::PathKind;
use super::session_state::SessionState;
use super::window_list::{SourceId, WindowList};
use super::wire::{
    from_json, invalid, read_frame, to_json, write_frame, BeReader, CONTROL_REQUEST, CONTROL_STATE,
    ERROR, FRAME_ACK, INPUT, KEYFRAME_REQUEST, LIST_WINDOWS, MAX_CONTROL_BYTES, MAX_WINDOWS_BYTES,
    PING, PONG, PREVIEW_ACK, SET_MODE, SNAPSHOT, STATE, STATS, WINDOWS,
};
use super::wire_hello::ScreenErrorCode;

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
    /// Wanted picture size; larger tiles of the group view ask for more.
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
    SetMode(SetMode),
    Input(Vec<InputEvent>),
    FrameAck(FrameAck),
    KeyframeRequest(KeyframeRequest),
    Ping(Ping),
    ListWindows,
    Snapshot(SnapshotRequest),
    ControlRequest(ControlRequest),
    PreviewAck(FrameAck),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HostFrame {
    State(StateReport),
    Pong(Pong),
    Windows(WindowList),
    Stats(HostStats),
    ControlState(ControlState),
    Error(ScreenWireError),
}

pub(crate) fn encode_viewer_frame(frame: &ViewerFrame) -> io::Result<(u8, Vec<u8>)> {
    Ok(match frame {
        ViewerFrame::SetMode(set_mode) => {
            set_mode.validate()?;
            (SET_MODE, to_json(set_mode)?)
        }
        ViewerFrame::Input(events) => (INPUT, encode_batch(events)?),
        ViewerFrame::FrameAck(ack) => (FRAME_ACK, ack.encode()),
        ViewerFrame::KeyframeRequest(request) => (KEYFRAME_REQUEST, to_json(request)?),
        ViewerFrame::Ping(ping) => (PING, encode_u64s(&[ping.sequence, ping.sent_us])),
        ViewerFrame::ListWindows => (LIST_WINDOWS, b"{}".to_vec()),
        ViewerFrame::Snapshot(request) => (SNAPSHOT, to_json(request)?),
        ViewerFrame::ControlRequest(request) => (CONTROL_REQUEST, to_json(request)?),
        ViewerFrame::PreviewAck(ack) => (PREVIEW_ACK, ack.encode()),
    })
}

pub(crate) fn decode_viewer_frame(tag: u8, payload: &[u8]) -> io::Result<ViewerFrame> {
    Ok(match tag {
        SET_MODE => {
            let set_mode: SetMode = from_json(payload)?;
            set_mode.validate()?;
            ViewerFrame::SetMode(set_mode)
        }
        INPUT => ViewerFrame::Input(decode_batch(payload)?),
        FRAME_ACK => ViewerFrame::FrameAck(FrameAck::decode(payload)?),
        KEYFRAME_REQUEST => ViewerFrame::KeyframeRequest(from_json(payload)?),
        PING => {
            let [sequence, sent_us] = decode_u64s::<2>(payload)?;
            ViewerFrame::Ping(Ping { sequence, sent_us })
        }
        LIST_WINDOWS => {
            from_json::<serde_json::Map<String, serde_json::Value>>(payload)?;
            ViewerFrame::ListWindows
        }
        SNAPSHOT => ViewerFrame::Snapshot(from_json(payload)?),
        CONTROL_REQUEST => ViewerFrame::ControlRequest(from_json(payload)?),
        PREVIEW_ACK => ViewerFrame::PreviewAck(FrameAck::decode(payload)?),
        _ => return Err(invalid("unknown screen frame from the viewer")),
    })
}

pub(crate) fn encode_host_frame(frame: &HostFrame) -> io::Result<(u8, Vec<u8>)> {
    Ok(match frame {
        HostFrame::State(report) => (STATE, to_json(report)?),
        HostFrame::Pong(pong) => (
            PONG,
            encode_u64s(&[
                pong.sequence,
                pong.ping_sent_us,
                pong.received_us,
                pong.sent_us,
            ]),
        ),
        HostFrame::Windows(list) => {
            list.validate()?;
            (WINDOWS, to_json(list)?)
        }
        HostFrame::Stats(stats) => (STATS, to_json(stats)?),
        HostFrame::ControlState(state) => (CONTROL_STATE, to_json(state)?),
        HostFrame::Error(error) => (ERROR, to_json(error)?),
    })
}

pub(crate) fn decode_host_frame(tag: u8, payload: &[u8]) -> io::Result<HostFrame> {
    Ok(match tag {
        STATE => HostFrame::State(from_json(payload)?),
        PONG => {
            let [sequence, ping_sent_us, received_us, sent_us] = decode_u64s::<4>(payload)?;
            HostFrame::Pong(Pong {
                sequence,
                ping_sent_us,
                received_us,
                sent_us,
            })
        }
        WINDOWS => {
            let list: WindowList = from_json(payload)?;
            list.validate()?;
            HostFrame::Windows(list)
        }
        STATS => HostFrame::Stats(from_json(payload)?),
        CONTROL_STATE => HostFrame::ControlState(from_json(payload)?),
        ERROR => HostFrame::Error(from_json(payload)?),
        _ => return Err(invalid("unknown screen frame from the host")),
    })
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

pub(crate) async fn send_host_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &HostFrame,
) -> io::Result<()> {
    let (tag, payload) = encode_host_frame(frame)?;
    write_frame(writer, tag, &payload).await
}

/// Viewer side: `WINDOWS` may take 1 MiB, every other host frame 64 KiB.
pub(crate) async fn recv_host_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<HostFrame> {
    let (tag, payload) = read_frame(reader, MAX_WINDOWS_BYTES).await?;
    decode_host_frame(tag, &payload)
}
