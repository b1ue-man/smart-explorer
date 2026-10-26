//! Private request parsing and serialization.
use super::*;
/// A request of the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkerRequest {
    CaptureControl(super::super::worker_capture_wire::CaptureRequest),
    OpenFamily(super::super::worker_family::OpenFamily),
    Family(super::super::worker_family::FamilyCommand),
    Release(Vec<InputEvent>),
    Open(OpenRequest),
    Produce(StageRequest),
    Capture {
        frame_id: u64,
        frame_contract:Option<super::super::frame_budget::FrameContract>,
    },
    /// The controlling viewer's input (SCR-M6).
    Input(BoundInput),
    SetCaptureActive(bool),
    Close,
}

impl WorkerRequest {
    pub(crate) fn encode(&self) -> io::Result<(u8, Vec<u8>)> {
        Ok(match self {
            Self::CaptureControl(request)=>(super::super::worker_capture_wire::CONTROL,to_json(request)?),
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
            Self::Capture { frame_id,frame_contract } => (
                CAPTURE,
                to_json(&CaptureWire {
                    frame_id: *frame_id,frame_contract:*frame_contract,
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
            super::super::worker_capture_wire::CONTROL=>Ok(Self::CaptureControl(from_json(payload)?)),
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
                if wire.visible_rects.as_ref().is_some_and(|rects|rects.len()>super::super::visibility::MAX_VISIBLE_RECTS || rects.iter().any(|r|
                    !super::super::pixel_convert::Rect {x:r.x,y:r.y,width:r.width,height:r.height}.fits(wire.size))) {return Err(invalid("invalid visible rectangles"));}
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
                if let Some(contract)=wire.frame_contract {contract.validate()?;}
                Ok(Self::Capture {
                    frame_id: wire.frame_id,frame_contract:wire.frame_contract,
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

