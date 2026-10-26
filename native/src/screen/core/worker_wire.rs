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
use super::pixel_convert::{BgraImage, FrameSize};
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

/// A request of the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkerRequest {
    OpenFamily(super::worker_family::OpenFamily),
    Family(super::worker_family::FamilyCommand),
    Release(Vec<InputEvent>),
    Open(OpenRequest),
    Produce(StageRequest),
    Capture {
        frame_id: u64,
    },
    /// The controlling viewer's input (SCR-M6).
    Input(BoundInput),
    SetCaptureActive(bool),
    Close,
}

impl WorkerRequest {
    pub(crate) fn encode(&self) -> io::Result<(u8, Vec<u8>)> {
        Ok(match self {
            Self::OpenFamily(open) => (OPEN_FAMILY, to_json(open)?),
            Self::Family(command) => (FAMILY_COMMAND, to_json(command)?),
            Self::Release(events) => (RELEASE_INPUT, to_json(&InputWire {events:events.clone()})?),
            Self::Open(open) => (OPEN, to_json(open)?),
            Self::Produce(request) => {
                let wire = ProduceWire {
                    frame_id: request.frame_id,
                    frame_contract: request.frame_contract,
                    visible_rects: request.visible_rects.clone(),
                    keyframe: request.keyframe,
                    target_bps: request.target_bps,
                    size: request.size,
                    codec: request.choice.codec,
                    profile: request.choice.profile,
                    encoder_hardware: request.choice.encoder_hardware,
                    decoder_hardware: request.choice.decoder_hardware,
                    max_size: request.choice.max_size,
                    max_pixels: request.choice.max_pixels,
                    max_macroblocks_per_second: request.choice.max_macroblocks_per_second,
                    encoder_epoch: request.encoder_epoch,
                    fps: request.fps,
                };
                (PRODUCE, to_json(&wire)?)
            }
            Self::Capture { frame_id } => (
                CAPTURE,
                to_json(&CaptureWire {
                    frame_id: *frame_id,
                })?,
            ),
            Self::Input(input) => (
                BOUND_INPUT,
                to_json(input)?,
            ),
            Self::SetCaptureActive(active) => (SET_CAPTURE_ACTIVE,vec![u8::from(*active)]),
            Self::Close => (CLOSE, Vec::new()),
        })
    }

    pub(crate) fn decode(tag: u8, payload: &[u8]) -> io::Result<Self> {
        match tag {
            OPEN_FAMILY => Ok(Self::OpenFamily(from_json(payload)?)),
            FAMILY_COMMAND => Ok(Self::Family(from_json(payload)?)),
            RELEASE_INPUT => { let wire:InputWire=from_json(payload)?; Ok(Self::Release(wire.events)) },
            OPEN => Ok(Self::Open(from_json(payload)?)),
            PRODUCE => {
                let wire: ProduceWire = from_json(payload)?;
                if let Some(contract) = wire.frame_contract {
                    contract.validate()?;
                    contract.limits.check(wire.size)?;
                } else {
                    wire.size.check_frame()?;
                }
                if wire.max_pixels == 0 || wire.max_pixels > FrameLimits::RESOURCE.max_pixels
                    || wire.max_macroblocks_per_second == Some(0)
                    || wire.size.pixels() > wire.max_pixels
                    || wire.size.width > wire.max_size.width || wire.size.height > wire.max_size.height {
                    return Err(invalid("worker frame exceeds codec pixel or dimension bound"));
                }
                if wire.visible_rects.as_ref().is_some_and(|rects|rects.len()>super::visibility::MAX_VISIBLE_RECTS || rects.iter().any(|r|
                    !super::pixel_convert::Rect {x:r.x,y:r.y,width:r.width,height:r.height}.fits(wire.size))) {return Err(invalid("invalid visible rectangles"));}
                Ok(Self::Produce(StageRequest {
                    frame_id: wire.frame_id,
                    frame_contract: wire.frame_contract,
                    visible_rects: wire.visible_rects,
                    keyframe: wire.keyframe,
                    target_bps: wire.target_bps,
                    size: wire.size,
                    choice: CodecChoice {
                        codec: wire.codec,
                        profile: wire.profile,
                        encoder_hardware: wire.encoder_hardware,
                        decoder_hardware: wire.decoder_hardware,
                        max_size: wire.max_size,
                        max_pixels: wire.max_pixels,
                        max_macroblocks_per_second: wire.max_macroblocks_per_second,
                    },
                    encoder_epoch: wire.encoder_epoch,
                    fps: wire.fps,
                }))
            }
            CAPTURE => {
                let wire: CaptureWire = from_json(payload)?;
                Ok(Self::Capture {
                    frame_id: wire.frame_id,
                })
            }
            BOUND_INPUT => {
                Ok(Self::Input(from_json(payload)?))
            }
            SET_CAPTURE_ACTIVE if payload==[0] || payload==[1] => Ok(Self::SetCaptureActive(payload[0]!=0)),
            CLOSE if payload.is_empty() => Ok(Self::Close),
            _ => Err(invalid("unexpected host process request")),
        }
    }
}

/// An answer of the host process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkerReply {
    Family(super::worker_family::FamilyReply),
    /// Generation is visible before any enclosed answer is consumed. No nesting.
    Reference {generation:u64,regions:bool,limits:FrameLimits,family_layout_revision:Option<u64>,reply:Box<WorkerReply>},
    Opened {
        size: FrameSize,
    },
    Refused {
        reason: String,
    },
    Frame(StageFrame),
    Failed {
        code: ScreenErrorCode,
        message: String,
    },
    Raw(BgraImage),
    CaptureActive(bool),
    /// `INPUT` was injected (`None`) or blocked with this reason.
    InputDone {
        blocked: Option<ControlBlock>,
        legacy_reconnect:bool,
    },
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

impl WorkerReply {
    pub(crate) fn encode(&self) -> io::Result<(u8, Vec<u8>)> {
        match self {
            Self::Family(reply) => Ok((FAMILY_REPLY, to_json(reply)?)),
            Self::Reference {generation,regions,limits,family_layout_revision,reply}=>frame_wire::encode_reference(*generation,*regions,*limits,*family_layout_revision,reply),
            Self::CaptureActive(active) => Ok((CAPTURE_ACTIVE,vec![u8::from(*active)])),
            Self::Opened { size } => Ok((OPENED, to_json(&SizeWire { size: *size })?)),
            Self::Refused { reason } => Ok((
                REFUSED,
                to_json(&RefusedWire {
                    reason: reason.clone(),
                })?,
            )),
            Self::Frame(frame) => encode_frame(frame),
            Self::Failed { code, message } => {
                let wire = FailedWire {
                    code: code.key().into(),
                    message: message.chars().take(1024).collect(),
                };
                Ok((FAILED, to_json(&wire)?))
            }
            Self::Raw(image) => {
                let mut out = Vec::with_capacity(image.data().len() + 8);
                push_size(&mut out, image.size());
                out.extend_from_slice(image.data());
                Ok((RAW, out))
            }
            Self::InputDone { blocked,legacy_reconnect } => {
                Ok((INPUT_DONE, to_json(&InputDoneWire { blocked: *blocked,legacy_reconnect:*legacy_reconnect })?))
            }
        }
    }

    pub(crate) fn decode(tag: u8, payload: &[u8]) -> io::Result<Self> {
        match tag {
            FAMILY_REPLY => Ok(Self::Family(from_json(payload)?)),
            REFERENCE=>frame_wire::decode_reference(payload),
            OPENED => {
                let wire: SizeWire = from_json(payload)?;
                if wire.size.width == 0 || wire.size.height == 0 {
                    return Err(invalid("host process names an empty source"));
                }
                Ok(Self::Opened { size: wire.size })
            }
            REFUSED => {
                let wire: RefusedWire = from_json(payload)?;
                Ok(Self::Refused {
                    reason: wire.reason,
                })
            }
            ENCODED => Ok(Self::Frame(decode_encoded(payload)?)),
            UNCHANGED => {
                let mut reader = BeReader::new(payload);
                let source_size = read_size(&mut reader)?;
                reader.finish()?;
                Ok(Self::Frame(StageFrame::Unchanged { source_size }))
            }
            PAUSED => {
                let wire: PausedWire = from_json(payload)?;
                Ok(Self::Frame(StageFrame::Paused(wire.state)))
            }
            FAILED => {
                let wire: FailedWire = from_json(payload)?;
                Ok(Self::Failed {
                    code: ScreenErrorCode::from_key(&wire.code)
                        .unwrap_or(ScreenErrorCode::CaptureFailed),
                    message: wire.message,
                })
            }
            RAW => {
                let mut reader = BeReader::new(payload);
                let size = read_size(&mut reader)?;
                let limits=if FrameLimits::RESOURCE.check(size).is_ok() {FrameLimits::RESOURCE} else {FrameLimits::LEGACY};
                limits.check(size)?;
                if reader.remaining()!=size.checked_bgra_len()? {return Err(invalid("worker raw pixels contradict their dimensions"));}
                let data = reader.bytes(reader.remaining())?.to_vec();
                Ok(Self::Raw(BgraImage::from_vec_with_limits(size, data, limits)?))
            }
            CAPTURE_ACTIVE if payload==[0] || payload==[1] => Ok(Self::CaptureActive(payload[0]!=0)),
            INPUT_DONE => {
                let wire: InputDoneWire = from_json(payload)?;
                Ok(Self::InputDone {
                    blocked: wire.blocked,
                    legacy_reconnect:wire.legacy_reconnect,
                })
            }
            _ => Err(invalid("unexpected host process answer")),
        }
    }
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
