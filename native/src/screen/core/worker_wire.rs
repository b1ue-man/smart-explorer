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

// Daemon to host process.
pub(crate) const OPEN: u8 = 1;
pub(crate) const PRODUCE: u8 = 2;
pub(crate) const CAPTURE: u8 = 3;
pub(crate) const CLOSE: u8 = 4;
pub(crate) const INPUT: u8 = 5;
// Host process to daemon.
pub(crate) const OPENED: u8 = 16;
pub(crate) const REFUSED: u8 = 17;
pub(crate) const ENCODED: u8 = 18;
pub(crate) const UNCHANGED: u8 = 19;
pub(crate) const PAUSED: u8 = 20;
pub(crate) const FAILED: u8 = 21;
pub(crate) const RAW: u8 = 22;
pub(crate) const INPUT_DONE: u8 = 23;

/// Largest `ENCODED`: the records of a 4096 × 2304 tile key frame (37.7 MB
/// raw) with room for LZ4's worst case and the record heads.
pub(crate) const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;
/// Most records of one `ENCODED` (a tile key frame of 4096 × 2304 needs 10).
pub(crate) const MAX_RECORDS: usize = 4096;
/// Largest `RAW`: a 4096 × 2304 BGRA picture and its two sides.
pub(crate) const MAX_RAW_BYTES: usize = 4096 * 2304 * 4 + 8;
/// Longest encoder name.
const MAX_NAME_BYTES: usize = 256;

/// The payload limit of `tag`; `None` for a tag this version does not know.
pub(crate) fn tag_limit(tag: u8) -> Option<usize> {
    match tag {
        OPEN | PRODUCE | CAPTURE | INPUT | OPENED | REFUSED | PAUSED | FAILED | INPUT_DONE => {
            Some(MAX_CONTROL_BYTES)
        }
        CLOSE => Some(0),
        UNCHANGED => Some(8),
        ENCODED => Some(MAX_ENCODED_BYTES),
        RAW => Some(MAX_RAW_BYTES),
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

#[derive(Serialize, Deserialize)]
struct InputDoneWire {
    #[serde(default)]
    blocked: Option<ControlBlock>,
}

/// `PRODUCE` as JSON (`CodecChoice` has no serde form of its own).
#[derive(Serialize, Deserialize)]
struct ProduceWire {
    frame_id: u64,
    keyframe: bool,
    target_bps: u64,
    size: FrameSize,
    codec: CodecKind,
    #[serde(default)]
    profile: Option<H264Profile>,
    encoder_hardware: bool,
    decoder_hardware: bool,
    max_size: FrameSize,
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
    Open(OpenRequest),
    Produce(StageRequest),
    Capture {
        frame_id: u64,
    },
    /// The controlling viewer's input (SCR-M6).
    Input(Vec<InputEvent>),
    Close,
}

impl WorkerRequest {
    pub(crate) fn encode(&self) -> io::Result<(u8, Vec<u8>)> {
        Ok(match self {
            Self::Open(open) => (OPEN, to_json(open)?),
            Self::Produce(request) => {
                let wire = ProduceWire {
                    frame_id: request.frame_id,
                    keyframe: request.keyframe,
                    target_bps: request.target_bps,
                    size: request.size,
                    codec: request.choice.codec,
                    profile: request.choice.profile,
                    encoder_hardware: request.choice.encoder_hardware,
                    decoder_hardware: request.choice.decoder_hardware,
                    max_size: request.choice.max_size,
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
            Self::Input(events) => (
                INPUT,
                to_json(&InputWire {
                    events: events.clone(),
                })?,
            ),
            Self::Close => (CLOSE, Vec::new()),
        })
    }

    pub(crate) fn decode(tag: u8, payload: &[u8]) -> io::Result<Self> {
        match tag {
            OPEN => Ok(Self::Open(from_json(payload)?)),
            PRODUCE => {
                let wire: ProduceWire = from_json(payload)?;
                wire.size.check_frame()?;
                Ok(Self::Produce(StageRequest {
                    frame_id: wire.frame_id,
                    keyframe: wire.keyframe,
                    target_bps: wire.target_bps,
                    size: wire.size,
                    choice: CodecChoice {
                        codec: wire.codec,
                        profile: wire.profile,
                        encoder_hardware: wire.encoder_hardware,
                        decoder_hardware: wire.decoder_hardware,
                        max_size: wire.max_size,
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
            INPUT => {
                let wire: InputWire = from_json(payload)?;
                Ok(Self::Input(wire.events))
            }
            CLOSE if payload.is_empty() => Ok(Self::Close),
            _ => Err(invalid("unexpected host process request")),
        }
    }
}

/// An answer of the host process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkerReply {
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
    /// `INPUT` was injected (`None`) or blocked with this reason.
    InputDone {
        blocked: Option<ControlBlock>,
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

fn encode_frame(frame: &StageFrame) -> io::Result<(u8, Vec<u8>)> {
    match frame {
        StageFrame::Encoded {
            records,
            keyframe,
            capture_ts_us,
            encoder,
            source_size,
        } => {
            let name = encoder.name.as_bytes();
            if name.len() > MAX_NAME_BYTES || records.is_empty() || records.len() > MAX_RECORDS {
                return Err(invalid("encoded picture does not fit a host process frame"));
            }
            let total: usize = records.iter().map(|record| record.len() + 4).sum();
            let mut out = Vec::with_capacity(total + name.len() + 32);
            out.extend_from_slice(&capture_ts_us.to_be_bytes());
            out.push(u8::from(*keyframe));
            out.push(u8::from(encoder.hardware));
            push_size(&mut out, *source_size);
            out.extend_from_slice(&(name.len() as u16).to_be_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&(records.len() as u32).to_be_bytes());
            for record in records {
                if record.len() > MAX_RECORD_BYTES {
                    return Err(invalid("encoded record exceeds its limit"));
                }
                out.extend_from_slice(&(record.len() as u32).to_be_bytes());
                out.extend_from_slice(record);
            }
            Ok((ENCODED, out))
        }
        StageFrame::Unchanged { source_size } => {
            let mut out = Vec::with_capacity(8);
            push_size(&mut out, *source_size);
            Ok((UNCHANGED, out))
        }
        StageFrame::Paused(state) => Ok((PAUSED, to_json(&PausedWire { state: *state })?)),
    }
}

fn decode_encoded(payload: &[u8]) -> io::Result<StageFrame> {
    let mut reader = BeReader::new(payload);
    let capture_ts_us = reader.u64()?;
    let keyframe = reader.u8()? != 0;
    let hardware = reader.u8()? != 0;
    let source_size = read_size(&mut reader)?;
    let name_len = usize::from(reader.u16()?);
    if name_len > MAX_NAME_BYTES {
        return Err(invalid("encoder name exceeds its limit"));
    }
    let name = String::from_utf8(reader.bytes(name_len)?.to_vec())
        .map_err(|_| invalid("encoder name is not UTF-8"))?;
    let count = reader.u32()? as usize;
    if count == 0 || count > MAX_RECORDS {
        return Err(invalid("encoded picture has no or too many records"));
    }
    // Every record needs at least its length field.
    let mut records = Vec::with_capacity(count.min(reader.remaining() / 4));
    for _ in 0..count {
        let len = reader.u32()? as usize;
        if len > MAX_RECORD_BYTES {
            return Err(invalid("encoded record exceeds its limit"));
        }
        records.push(reader.bytes(len)?.to_vec());
    }
    reader.finish()?;
    Ok(StageFrame::Encoded {
        records,
        keyframe,
        capture_ts_us,
        encoder: StageEncoder { name, hardware },
        source_size,
    })
}

impl WorkerReply {
    pub(crate) fn encode(&self) -> io::Result<(u8, Vec<u8>)> {
        match self {
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
            Self::InputDone { blocked } => {
                Ok((INPUT_DONE, to_json(&InputDoneWire { blocked: *blocked })?))
            }
        }
    }

    pub(crate) fn decode(tag: u8, payload: &[u8]) -> io::Result<Self> {
        match tag {
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
                size.check_frame()?;
                let data = reader.bytes(reader.remaining())?.to_vec();
                Ok(Self::Raw(BgraImage::from_vec(size, data)?))
            }
            INPUT_DONE => {
                let wire: InputDoneWire = from_json(payload)?;
                Ok(Self::InputDone {
                    blocked: wire.blocked,
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
