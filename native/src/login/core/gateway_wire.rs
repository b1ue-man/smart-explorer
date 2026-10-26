//! Bounded lossless RGB8 PNG network frames; the broker/helper retain raw BGRA.
//! Each PNG stands alone. There is no image baseline across sessions or routes.
use super::{
    gateway_types::GatewayReply,
    model::{MAX_CONTROL, MAX_PIXELS},
};
use super::{
    image_receive::{self, ImageReceiver},
    image_send,
};
use crate::screen::core::{
    capture_bytes::FrozenBytes, capture_image::CapturedImage, image_admission::ImageAdmission,
    pixel_convert::FrameSize,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{io, sync::Arc};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
const CONTROL: u8 = 1;
const PNG: u8 = 3;
// The existing raw BGRA frame budget also caps encoded data. The screen PNG
// encoder emits RGB8, whose worst-case pixel data uses three bytes per pixel.
const MAX_PNG_BYTES: usize = MAX_PIXELS;
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ImageEncoding {
    None,
    PngRgb8,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    packet: GatewayReply,
    image_encoding: ImageEncoding,
    image_bytes: usize,
}
impl Header {
    fn validate(&self) -> io::Result<()> {
        self.packet.validate_pixels(self.packet.reply.pixel_bytes)?;
        if self.packet.reply.pixel_bytes == 0 {
            if self.image_encoding != ImageEncoding::None || self.image_bytes != 0 {
                return Err(invalid("control-only login reply contains image payload"));
            }
        } else {
            FrameSize::new(self.packet.reply.width, self.packet.reply.height).check_frame()?;
            if self.image_encoding != ImageEncoding::PngRgb8
                || self.image_bytes == 0
                || self.image_bytes > MAX_PNG_BYTES
            {
                return Err(invalid(
                    "login image must be a bounded independent RGB8 PNG",
                ));
            }
        }
        Ok(())
    }
}
pub(crate) struct EncodedReply {
    pub packet: GatewayReply,
    png: Option<FrozenBytes>,
    _image: Option<CapturedImage>,
}
pub(crate) async fn prepare_reply(
    packet: GatewayReply,
    image: Option<CapturedImage>,
    admission: Arc<ImageAdmission>,
) -> io::Result<EncodedReply> {
    let Some(image) = image else {
        packet.validate_pixels(0)?;
        return Ok(EncodedReply {
            packet,
            png: None,
            _image: None,
        });
    };
    tokio::task::spawn_blocking(move || {
        let png = image_send::encode(&packet, &image, &admission)?;
        Ok(EncodedReply {
            packet,
            png: Some(png),
            _image: Some(image),
        })
    })
    .await
    .map_err(io::Error::other)?
}
async fn send_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    tag: u8,
    bytes: &[u8],
) -> io::Result<()> {
    let limit = if tag == CONTROL {
        MAX_CONTROL
    } else {
        MAX_PNG_BYTES
    };
    if bytes.len() > limit {
        return Err(invalid("login network frame exceeds limit"));
    }
    writer.write_all(&[tag]).await?;
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await
}
async fn receive_frame(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let mut head = [0u8; 5];
    reader.read_exact(&mut head).await?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if head[0] != CONTROL || len > MAX_CONTROL {
        return Err(invalid("unexpected login control frame or length"));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(io::Error::other)?;
    bytes.resize(len, 0);
    reader.read_exact(&mut bytes).await?;
    Ok(bytes)
}
pub(crate) async fn send<T: Serialize>(
    writer: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> io::Result<()> {
    send_frame(
        writer,
        CONTROL,
        &serde_json::to_vec(value).map_err(io::Error::other)?,
    )
    .await
}
pub(crate) async fn receive<T: DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin),
) -> io::Result<T> {
    serde_json::from_slice(&receive_frame(reader).await?).map_err(io::Error::other)
}
pub(crate) async fn send_prepared(
    writer: &mut (impl AsyncWrite + Unpin),
    encoded: &EncodedReply,
) -> io::Result<()> {
    let header = Header {
        packet: encoded.packet.clone(),
        image_encoding: if encoded.png.is_none() {
            ImageEncoding::None
        } else {
            ImageEncoding::PngRgb8
        },
        image_bytes: encoded.png.as_ref().map_or(0, FrozenBytes::len),
    };
    header.validate()?;
    send(writer, &header).await?;
    if let Some(png) = &encoded.png {
        send_frame(writer, PNG, png.as_slice()).await?;
    }
    Ok(())
}
/// Control-only shortcut; callers never pass raw image bytes to this method.
pub(crate) async fn send_reply(
    writer: &mut (impl AsyncWrite + Unpin),
    packet: &GatewayReply,
    pixels: &[u8],
) -> io::Result<()> {
    if !pixels.is_empty() {
        return Err(invalid(
            "login pixels must be encoded outside the privileged broker",
        ));
    }
    packet.validate_pixels(0)?;
    send_prepared(
        writer,
        &EncodedReply {
            packet: packet.clone(),
            png: None,
            _image: None,
        },
    )
    .await
}
pub(crate) async fn receive_reply(
    reader: &mut (impl AsyncRead + Unpin),
    owner: &ImageReceiver,
) -> io::Result<(GatewayReply, Option<CapturedImage>)> {
    let header: Header = receive(reader).await?;
    header.validate()?;
    if header.image_bytes == 0 {
        return Ok((header.packet, None));
    }
    // P, N and the RGB row are all funded before the first PNG body read.
    let mut buffers = owner.prepare(&header.packet, header.image_bytes)?;
    let mut head = [0u8; 5];
    reader.read_exact(&mut head).await?;
    let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if head[0] != PNG || length != header.image_bytes {
        return Err(invalid("unexpected login PNG frame or length"));
    }
    reader.read_exact(buffers.png.as_mut_slice()).await?;
    let owner = owner.clone();
    // Dropping the awaiting future does not cancel this closure. It owns all
    // allocations and the registry anchor; its result keeps that same anchor.
    tokio::task::spawn_blocking(move || image_receive::decode(header.packet, buffers, owner))
        .await
        .map_err(io::Error::other)?
}
fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}
