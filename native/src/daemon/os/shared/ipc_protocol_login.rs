//! Token-authenticated login control, followed by bounded binary BGRA chunks.
use crate::login::{
    Challenge, GatewayAccess, GatewayReply, GatewayTarget, InstallationStatus, Operation,
};
use crate::screen::core::{
    capture_admission::{CaptureGeneration, SourceKey},
    capture_image::{CaptureImageBuilder, CapturedImage},
    frame_budget::FrameLimits,
    image_admission::{ImageAdmission, ImageRole},
    input_model::InputEvent,
    pixel_convert::FrameSize,
};
use crate::share::login_transport::RouteStatus;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    sync::Arc,
};
pub(crate) const MAX_CONTROL: usize = crate::login::core::model::MAX_CONTROL;
pub(crate) const MAX_CHUNK: usize = 1024 * 1024;
pub(crate) const HELLO: u8 = 1;
pub(crate) const REPLY: u8 = 2;
pub(crate) const BGRA: u8 = 3;
pub(crate) const END: u8 = 4;
pub(crate) const COMMAND: u8 = 16;
pub(crate) const CLOSE: u8 = 17;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LoginIpcRequest {
    Status,
    Sessions,
    Session {
        session: String,
    },
    Stop {
        session: Option<String>,
    },
    Stream {
        target: GatewayTarget,
        target_user: String,
        acknowledge_machine_pin: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LoginIpcReply {
    Status { installation: InstallationStatus },
    Sessions { sessions: Vec<LoginSessionView> },
    Session { session: Option<LoginSessionView> },
    Stopped { count: usize },
    StreamOpened { session: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LoginSessionView {
    pub session: String,
    pub target: GatewayTarget,
    pub target_user: String,
    pub state: String,
    pub challenge: Option<Challenge>,
    pub access: Option<GatewayAccess>,
    pub route: Option<RouteStatus>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LoginCommand {
    Status,
    Challenge,
    Start,
    Capture { frame_id: u64 },
    Input { events: Vec<InputEvent> },
    ReleaseInputs,
    SecureAttention,
    Stop,
}
impl LoginCommand {
    pub(crate) fn operation(&self) -> Option<Operation> {
        Some(match self {
            Self::Status | Self::Challenge => return None,
            Self::Start => Operation::Start,
            Self::Capture { frame_id } => Operation::Capture {
                frame_id: *frame_id,
            },
            Self::Input { events } => Operation::Input {
                events: events.clone(),
            },
            Self::ReleaseInputs => Operation::ReleaseInputs,
            Self::SecureAttention => Operation::SecureAttention,
            Self::Stop => Operation::Stop,
        })
    }
    pub(crate) fn validate(&self) -> io::Result<()> {
        if let Some(operation) = self.operation() {
            operation.validate().map_err(invalid)?;
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginRelayCommand {
    pub sequence: u64,
    pub command: LoginCommand,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginRelayReply {
    pub sequence: u64,
    pub view: LoginSessionView,
    pub packet: Option<GatewayReply>,
}
/// Pixel bytes deliberately have no Serialize implementation.
pub(crate) struct LoginFrame {
    pub reply: LoginRelayReply,
    pub image: Option<CapturedImage>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginEnd {
    pub code: String,
    pub detail: Option<String>,
}

pub(crate) fn write_frame(writer: &mut impl Write, tag: u8, bytes: &[u8]) -> io::Result<()> {
    let limit = if tag == BGRA { MAX_CHUNK } else { MAX_CONTROL };
    if bytes.len() > limit {
        return Err(invalid("login IPC frame exceeds its byte limit"));
    }
    writer.write_all(&[tag])?;
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}
pub(crate) fn read_frame(reader: &mut impl Read, limit: usize) -> io::Result<(u8, Vec<u8>)> {
    let (tag, len) = read_header(reader, limit)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(invalid)?;
    bytes.resize(len, 0);
    reader.read_exact(&mut bytes)?;
    Ok((tag, bytes))
}
pub(crate) fn read_header(reader: &mut impl Read, limit: usize) -> io::Result<(u8, usize)> {
    let mut head = [0u8; 5];
    reader.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > limit {
        return Err(invalid("login IPC frame exceeds its byte limit"));
    }
    Ok((head[0], len))
}
pub(crate) fn write_json(
    writer: &mut impl Write,
    tag: u8,
    value: &impl Serialize,
) -> io::Result<()> {
    write_frame(writer, tag, &serde_json::to_vec(value).map_err(invalid)?)
}
pub(crate) fn write_reply(writer: &mut impl Write, frame: &LoginFrame) -> io::Result<()> {
    let pixels = frame
        .image
        .as_ref()
        .map_or(&[][..], |image| image.image().data());
    validate_reply(&frame.reply, pixels.len())?;
    write_json(writer, REPLY, &frame.reply)?;
    for chunk in pixels.chunks(MAX_CHUNK) {
        write_frame(writer, BGRA, chunk)?;
    }
    Ok(())
}
pub(crate) fn read_pixels(
    reader: &mut impl Read,
    reply: &LoginRelayReply,
    images: &Arc<ImageAdmission>,
    source: &SourceKey,
) -> io::Result<Option<CapturedImage>> {
    let bytes = reply
        .packet
        .as_ref()
        .map_or(0, |packet| packet.reply.pixel_bytes);
    validate_reply(reply, bytes)?;
    if bytes == 0 {
        return Ok(None);
    }
    let packet = reply
        .packet
        .as_ref()
        .ok_or_else(|| invalid("login pixels have no reply"))?;
    let size = FrameSize::new(packet.reply.width, packet.reply.height);
    let mut image = CaptureImageBuilder::new_local(
        size,
        FrameLimits::LEGACY,
        images,
        ImageRole::ViewerInput,
        source.clone(),
        CaptureGeneration::new(reply.sequence),
    )?;
    let mut offset = 0usize;
    while offset < bytes {
        let (tag, len) = read_header(reader, MAX_CHUNK.min(bytes - offset))?;
        if tag != BGRA || len == 0 {
            return Err(invalid("unexpected or empty login BGRA chunk"));
        }
        reader.read_exact(&mut image.pixels_mut()[offset..offset + len])?;
        offset += len;
    }
    Ok(Some(image.freeze()))
}
pub(crate) fn validate_reply(reply: &LoginRelayReply, bytes: usize) -> io::Result<()> {
    if let Some(packet) = &reply.packet {
        packet.validate_pixels(bytes)?;
        if bytes != 0 {
            FrameSize::new(packet.reply.width, packet.reply.height).check_frame()?;
        }
    } else if bytes != 0 {
        return Err(invalid("login control reply cannot contain pixels"));
    }
    Ok(())
}
pub(crate) fn valid_session(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
pub(crate) fn invalid(reason: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason.to_string())
}
