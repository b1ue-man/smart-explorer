//! Private reply model; picture bytes require admitted transport.
use super::*;
/// An answer of the host process.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WorkerReply {
    CaptureControl(super::super::worker_capture_wire::CaptureReply),
    Family(super::super::worker_family::FamilyReply),
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
    Raw(super::super::capture_image::CapturedImage),
    CaptureActive(bool),
    /// `INPUT` was injected (`None`) or blocked with this reason.
    InputDone {
        blocked: Option<ControlBlock>,
        legacy_reconnect:bool,
    },
}

impl WorkerReply {
    pub(crate) fn encode(&self) -> io::Result<(u8, Vec<u8>)> {
        match self {
            Self::CaptureControl(reply)=>Ok((super::super::worker_capture_wire::CONTROL_REPLY,to_json(reply)?)),
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
            Self::Frame(StageFrame::Encoded {..}) => Err(invalid("worker records require owned capture transport")),
            Self::Frame(frame) => encode_frame(frame),
            Self::Failed { code, message } => {
                let wire = FailedWire {
                    code: code.key().into(),
                    message: message.chars().take(1024).collect(),
                };
                Ok((FAILED, to_json(&wire)?))
            }
            Self::Raw(_) => Err(invalid("worker raw reply requires owned capture transport")),
            Self::InputDone { blocked,legacy_reconnect } => {
                Ok((INPUT_DONE, to_json(&InputDoneWire { blocked: *blocked,legacy_reconnect:*legacy_reconnect })?))
            }
        }
    }

    pub(crate) fn decode(tag: u8, payload: &[u8]) -> io::Result<Self> {
        match tag {
            super::super::worker_capture_wire::CONTROL_REPLY=>Ok(Self::CaptureControl(from_json(payload)?)),
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
            RAW => Err(invalid("worker raw reply requires a prior capture ticket")),
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

