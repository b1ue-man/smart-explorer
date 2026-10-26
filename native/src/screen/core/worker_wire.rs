//! Frames between the daemon and its host process (SCR-M3, contract
//! „Hostprozess-Rahmen“). They run over the host process's standard input
//! (daemon to process) and standard output (process to daemon): tag `u8`,
//! length `u32` big-endian, payload. Every length is checked against the
//! limit of its tag before memory for it is requested, on both sides. The
//! daemon opens the source (`OPEN` → `OPENED` or `REFUSED`), then asks for
//! one encoded picture per frame (`PRODUCE` → `ENCODED`, `UNCHANGED`,
//! `PAUSED` or `FAILED`) and for raw pictures of still images (`CAPTURE` →
//! `RAW`); `CLOSE` or the end of the input ends the process. With SCR-M6 a
//! source opened for a controlling viewer (`OPEN` with `control`) takes the
//! viewer's input (`INPUT` → `INPUT_DONE` with the reason of a block); the
//! process injects it, because on Wayland only the portal session of the
//! capture carries the input (C5). Input events are never logged.
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use super::host_stage::{StageEncoder, StageFrame, StageRequest};
use super::input_model::InputEvent;
use super::negotiate::{CodecChoice, CodecKind, H264Profile};
use super::pixel_convert::FrameSize;
use super::session_state::SessionState;
use super::window_list::SourceId;
use super::wire::{from_json, invalid, to_json, BeReader, MAX_CONTROL_BYTES};
use super::wire_binary::MAX_RECORD_BYTES;
use super::wire_control::ControlBlock;
use super::wire_hello::ScreenErrorCode;
use super::frame_budget::FrameLimits;

#[path = "worker_frame_wire.rs"]
mod frame_wire;
use frame_wire::{encode_frame,decode_encoded};

// Daemon to host process.
pub(crate) const OPEN: u8 = 1;
pub(crate) const PRODUCE: u8 = 2;
pub(crate) const CAPTURE: u8 = 3;
pub(crate) const CLOSE: u8 = 4;
pub(crate) const INPUT: u8 = 5;
pub(crate) const BOUND_INPUT:u8=10;
pub(crate) const SET_CAPTURE_ACTIVE: u8 = 6;
pub(crate) const OPEN_FAMILY: u8 = 7;
pub(crate) const FAMILY_COMMAND: u8 = 8;
pub(crate) const RELEASE_INPUT: u8 = 9;
pub(crate) const FAMILY_REPLY: u8 = 26;
pub(crate) const CAPTURE_ACTIVE: u8 = 24;
pub(crate) const REFERENCE:u8=25;
// Host process to daemon.
pub(crate) const OPENED: u8 = 16;
pub(crate) const REFUSED: u8 = 17;
pub(crate) const ENCODED: u8 = 18;
pub(crate) const UNCHANGED: u8 = 19;
pub(crate) const PAUSED: u8 = 20;
pub(crate) const FAILED: u8 = 21;
pub(crate) const RAW: u8 = 22;
pub(crate) const INPUT_DONE: u8 = 23;

/// Absolute private-worker allocation ceiling. Each PRODUCE retains its
/// narrower negotiated contract; a packet length never increases that contract.
fn packet_budget() -> io::Result<super::frame_size_caps::PacketBudget> {
    frame_wire::absolute_budget()
}
/// Longest encoder name.
const MAX_NAME_BYTES: usize = 256;

/// The payload limit of `tag`; `None` for a tag this version does not know.
pub(crate) fn tag_limit(tag: u8) -> Option<usize> {
    match tag {
        super::worker_capture_wire::CONTROL | super::worker_capture_wire::CONTROL_REPLY
            | super::worker_capture_wire::GRANT_REQUEST | super::worker_capture_wire::GRANT_REPLY
            | super::worker_capture_wire::RETIRED | super::worker_capture_wire::REDUCED => Some(super::wire::MAX_WINDOWS_BYTES),
        super::worker_capture_wire::OWNED_REFERENCE => packet_budget().ok()?.encoded_bytes.max(packet_budget().ok()?.raw_bytes).checked_add(58),
        OPEN | OPEN_FAMILY | FAMILY_COMMAND | RELEASE_INPUT | PRODUCE | CAPTURE | INPUT | BOUND_INPUT | OPENED | REFUSED | PAUSED | FAILED | INPUT_DONE => {
            Some(MAX_CONTROL_BYTES)
        }
        CLOSE => Some(0),
        SET_CAPTURE_ACTIVE | CAPTURE_ACTIVE => Some(1),
        UNCHANGED => Some(8),
        ENCODED => Some(packet_budget().ok()?.encoded_bytes),
        REFERENCE => packet_budget().ok()?.encoded_bytes.checked_add(42),
        RAW => Some(packet_budget().ok()?.raw_bytes),
        FAMILY_REPLY => Some(super::wire::MAX_WINDOWS_BYTES),
        _ => None,
    }
}

/// Writes one frame; a payload above its tag's limit is refused before
/// anything is written.
pub(crate) fn write_message(writer: &mut impl Write, tag: u8, payload: &[u8]) -> io::Result<()> {
    let limit = tag_limit(tag).ok_or_else(|| invalid("unknown host process frame"))?;
    if payload.len() > limit {
        return Err(invalid("host process frame exceeds the limit of its tag"));
    }
    let len = u32::try_from(payload.len())
        .map_err(|_| invalid("host process frame exceeds the limit of its tag"))?;
    let mut head = [0u8; 5];
    head[0] = tag;
    head[1..].copy_from_slice(&len.to_be_bytes());
    writer.write_all(&head)?;
    writer.write_all(payload)?;
    writer.flush()
}

/// Reads one frame; `None` when the stream ended cleanly before a frame.
pub(crate) fn read_message(reader: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    read_message_with_limits(reader,None)
}

/// PRODUCE supplies its actual v2 limits; None preserves non-PRODUCE/legacy IO.
pub(crate) fn read_message_with_limits(reader:&mut impl Read, limits:Option<FrameLimits>)->io::Result<Option<(u8,Vec<u8>)>> {
    let mut head = [0u8; 5];
    loop {
        match reader.read(&mut head[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    reader.read_exact(&mut head[1..])?;
    let limit = tag_limit(head[0]).ok_or_else(|| invalid("unknown host process frame"))?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > limit {
        return Err(invalid("host process frame exceeds the limit of its tag"));
    }
    if matches!(head[0], RAW | ENCODED | super::worker_capture_wire::OWNED_REFERENCE) {
        return Err(invalid("worker pictures require a prior owned transport ticket"));
    }
    if head[0]==REFERENCE {
        return Ok(Some((REFERENCE,frame_wire::read_reference_payload(reader,len,limits)?)));
    }
    if let Some(limits)=limits {
        if len>frame_wire::negotiated_payload_limit(head[0],limits)? {
            return Err(invalid("worker reply exceeds PRODUCE working memory"));
        }
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok(Some((head[0], payload)))
}

/// `OPEN`: the source and the size a viewer asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OpenRequest {
    pub source: SourceId,
    #[serde(default)]
    pub wanted: Option<FrameSize>,
    /// The session may control (SCR-M6): a Wayland host asks the portal for
    /// `RemoteDesktop` in the same session; older processes ignore it.
    #[serde(default)]
    pub control: bool,
}

#[derive(Serialize, Deserialize)]
struct InputWire {
    events: Vec<InputEvent>,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(crate) struct BoundInput {
    pub events:Vec<InputEvent>,
    pub reference_generation:u64,
    pub presented:bool,
}

#[derive(Serialize, Deserialize)]
struct InputDoneWire {
    #[serde(default)]
    blocked: Option<ControlBlock>,
    #[serde(default)]
    legacy_reconnect:bool,
}

/// `PRODUCE` as JSON (`CodecChoice` has no serde form of its own).
#[derive(Serialize, Deserialize)]
struct ProduceWire {
    frame_id: u64,
    #[serde(default)]
    frame_contract: Option<super::frame_budget::FrameContract>,
    #[serde(default)]
    visible_rects: Option<Vec<super::visibility::VisibleRect>>,
    keyframe: bool,
    target_bps: u64,
    size: FrameSize,
    codec: CodecKind,
    #[serde(default)]
    profile: Option<H264Profile>,
    encoder_hardware: bool,
    decoder_hardware: bool,
    max_size: FrameSize,
    #[serde(default = "legacy_pixels")]
    max_pixels: u64,
    #[serde(default)]
    max_macroblocks_per_second: Option<u64>,
    encoder_epoch: u64,
    fps: u32,
}

#[derive(Serialize, Deserialize)]
struct CaptureWire {
    frame_id: u64,
    #[serde(default,skip_serializing_if="Option::is_none")]
    frame_contract:Option<super::frame_budget::FrameContract>,
}

#[derive(Serialize, Deserialize)]
struct SizeWire {
    size: FrameSize,
}

fn legacy_pixels() -> u64 { super::pixel_convert::MAX_FRAME_PIXELS }

#[derive(Serialize, Deserialize)]
struct RefusedWire {
    reason: String,
}

#[derive(Serialize, Deserialize)]
struct PausedWire {
    state: SessionState,
}

#[derive(Serialize, Deserialize)]
struct FailedWire {
    code: String,
    message: String,
}

fn push_size(out: &mut Vec<u8>, size: FrameSize) {
    out.extend_from_slice(&size.width.to_be_bytes());
    out.extend_from_slice(&size.height.to_be_bytes());
}

fn read_size(reader: &mut BeReader<'_>) -> io::Result<FrameSize> {
    let size = FrameSize::new(reader.u32()?, reader.u32()?);
    if size.width == 0 || size.height == 0 {
        return Err(invalid("host process names an empty source"));
    }
    Ok(size)
}

/// Writes one request (daemon side).
pub(crate) fn send_request(writer: &mut impl Write, request: &WorkerRequest) -> io::Result<()> {
    let (tag, payload) = request.encode()?;
    write_message(writer, tag, &payload)
}

/// Writes one answer (host process side).
pub(crate) fn send_reply(writer: &mut impl Write, reply: &WorkerReply) -> io::Result<()> {
    let (tag, payload) = reply.encode()?;
    write_message(writer, tag, &payload)
}

#[path = "worker_request_wire.rs"]
mod requests;
#[path = "worker_reply_wire.rs"]
mod replies;
pub(crate) use requests::WorkerRequest;
pub(crate) use replies::WorkerReply;
