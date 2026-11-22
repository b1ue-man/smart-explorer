//! Private request parsing and serialization.
use super::*;
/// A request of the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerRequest {
    CaptureControl(super::super::worker_capture_wire::CaptureRequest),
    OpenFamily(super::super::worker_family::OpenFamily),
    Family(super::super::worker_family::FamilyCommand),
    Release(Vec<InputEvent>),
    Open(OpenRequest),
    Produce(StageRequest),
    Capture {
        frame_id: u64,
        frame_contract: Option<super::super::frame_budget::FrameContract>,
    },
    /// The controlling viewer's input (SCR-M6).
    Input(BoundInput),
    SetCaptureActive(bool),
    /// Settings and credit of the self-paced picture stream (B-27).
    Stream(StreamOrder),
    Close,
}

/// `STREAM` (plan 2026-11-14 „Bildkette“ B-27): the daemon tells a host
/// process that announced `STREAM_VERSION` what to stream and how much.
/// `request` carries the settings of the picture stream (size, codec,
/// encoder epoch, contract, target rate, visible parts) and in `frame_id`
/// the id of the next frame the daemon sends; its `keyframe` is not used,
/// `key_serial` is. `granted` counts the answers the daemon asked for so
/// far: the process answers each one exactly once (a picture, unchanged,
/// paused or failed). `ahead` (0 or 1) pictures may be captured and encoded
/// before their credit arrives; they wait in the process until it does.
/// A higher `key_serial` asks for a key frame. `cadence_fps` is the capture
/// clock of the process. A higher `recover_serial` (plan „Bildkette“ B-36)
/// asks that the next picture reference only `last_good` (a frame the viewer
/// confirmed) or older: the encoding stage calls
/// `ScreenEncoder::recover_references` and encodes without a key frame when
/// the encoder continued, else a key frame of a new encoder; pictures
/// encoded before the wish are dropped. Both are 0 from a daemon without it.
/// Between wishes `last_good` carries the newest frame the viewer confirmed
/// (Gemini 8/3.1), which the encoding stage hands to the encoder's
/// long-term references (`ScreenEncoder::confirm_reference`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamOrder {
    pub request: StageRequest,
    pub granted: u64,
    pub ahead: u32,
    pub key_serial: u64,
    pub cadence_fps: u32,
    pub recover_serial: u64,
    pub last_good: u64,
    /// P9: the quality target of sharp mode (`StreamHint::quality_target`);
    /// a change opens a new encoder.
    pub quality_target: Option<u8>,
}

/// Pictures a host process may encode ahead of their credit.
pub const MAX_AHEAD: u32 = 1;

#[derive(Serialize, Deserialize)]
struct StreamWire {
    request: ProduceWire,
    granted: u64,
    #[serde(default)]
    ahead: u32,
    #[serde(default)]
    key_serial: u64,
    cadence_fps: u32,
    #[serde(default)]
    recover_serial: u64,
    #[serde(default)]
    last_good: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quality_target: Option<u8>,
}

fn produce_wire(request: &StageRequest) -> ProduceWire {
    ProduceWire {
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
    }
}

/// A `PRODUCE` (or the settings of a `STREAM`) within the bounds of its
/// contract and codec.
fn stage_request(wire: ProduceWire) -> io::Result<StageRequest> {
    if let Some(contract) = wire.frame_contract {
        contract.validate()?;
        contract.limits.check(wire.size)?;
    } else {
        wire.size.check_frame()?;
    }
    if wire.max_pixels == 0
        || wire.max_pixels > FrameLimits::RESOURCE.max_pixels
        || wire.max_macroblocks_per_second == Some(0)
        || wire.size.pixels() > wire.max_pixels
        || wire.size.width > wire.max_size.width
        || wire.size.height > wire.max_size.height
    {
        return Err(invalid(
            "worker frame exceeds codec pixel or dimension bound",
        ));
    }
    if wire.visible_rects.as_ref().is_some_and(|rects| {
        rects.len() > super::super::visibility::MAX_VISIBLE_RECTS
            || rects.iter().any(|r| {
                !super::super::pixel_convert::Rect {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                }
                .fits(wire.size)
            })
    }) {
        return Err(invalid("invalid visible rectangles"));
    }
    Ok(StageRequest {
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
    })
}

impl WorkerRequest {
    pub fn encode(&self) -> io::Result<(u8, Vec<u8>)> {
        Ok(match self {
            Self::CaptureControl(request) => (
                super::super::worker_capture_wire::CONTROL,
                to_json(request)?,
            ),
            Self::OpenFamily(open) => (OPEN_FAMILY, to_json(open)?),
            Self::Family(command) => (FAMILY_COMMAND, to_json(command)?),
            Self::Release(events) => (
                RELEASE_INPUT,
                to_json(&InputWire {
                    events: events.clone(),
                })?,
            ),
            Self::Open(open) => (OPEN, to_json(open)?),
            Self::Produce(request) => (PRODUCE, to_json(&produce_wire(request))?),
            Self::Stream(order) => (
                STREAM,
                to_json(&StreamWire {
                    request: produce_wire(&order.request),
                    granted: order.granted,
                    ahead: order.ahead,
                    key_serial: order.key_serial,
                    cadence_fps: order.cadence_fps,
                    recover_serial: order.recover_serial,
                    last_good: order.last_good,
                    quality_target: order.quality_target,
                })?,
            ),
            Self::Capture {
                frame_id,
                frame_contract,
            } => (
                CAPTURE,
                to_json(&CaptureWire {
                    frame_id: *frame_id,
                    frame_contract: *frame_contract,
                })?,
            ),
            Self::Input(input) => (BOUND_INPUT, to_json(input)?),
            Self::SetCaptureActive(active) => (SET_CAPTURE_ACTIVE, vec![u8::from(*active)]),
            Self::Close => (CLOSE, Vec::new()),
        })
    }

    pub fn decode(tag: u8, payload: &[u8]) -> io::Result<Self> {
        match tag {
            super::super::worker_capture_wire::CONTROL => {
                Ok(Self::CaptureControl(from_json(payload)?))
            }
            OPEN_FAMILY => Ok(Self::OpenFamily(from_json(payload)?)),
            FAMILY_COMMAND => Ok(Self::Family(from_json(payload)?)),
            RELEASE_INPUT => {
                let wire: InputWire = from_json(payload)?;
                Ok(Self::Release(wire.events))
            }
            OPEN => Ok(Self::Open(from_json(payload)?)),
            PRODUCE => Ok(Self::Produce(stage_request(from_json(payload)?)?)),
            STREAM => {
                let wire: StreamWire = from_json(payload)?;
                if wire.ahead > MAX_AHEAD {
                    return Err(invalid("stream order asks for too many pictures ahead"));
                }
                Ok(Self::Stream(StreamOrder {
                    request: stage_request(wire.request)?,
                    granted: wire.granted,
                    ahead: wire.ahead,
                    key_serial: wire.key_serial,
                    cadence_fps: wire.cadence_fps.clamp(1, super::super::pacing::MAX_FPS),
                    recover_serial: wire.recover_serial,
                    last_good: wire.last_good,
                    quality_target: wire.quality_target.filter(|q| (1..=51).contains(q)),
                }))
            }
            CAPTURE => {
                let wire: CaptureWire = from_json(payload)?;
                if let Some(contract) = wire.frame_contract {
                    contract.validate()?;
                }
                Ok(Self::Capture {
                    frame_id: wire.frame_id,
                    frame_contract: wire.frame_contract,
                })
            }
            BOUND_INPUT => Ok(Self::Input(from_json(payload)?)),
            SET_CAPTURE_ACTIVE if payload == [0] || payload == [1] => {
                Ok(Self::SetCaptureActive(payload[0] != 0))
            }
            CLOSE if payload.is_empty() => Ok(Self::Close),
            _ => Err(invalid("unexpected host process request")),
        }
    }
}
