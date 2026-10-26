//! Bounded lossless RGB8 PNG network frames; the broker/helper retain raw BGRA.
//! Each PNG stands alone. There is no image baseline across sessions or routes.
use std::io::{self, Cursor};
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use crate::screen::core::{pixel_convert::{BgraImage, FrameSize}, snapshot};
use super::{gateway_types::GatewayReply, model::{MAX_CONTROL, MAX_PIXELS}};
const CONTROL: u8 = 1;
const PNG: u8 = 3;
// The existing raw BGRA frame budget also caps encoded data. The screen PNG
// encoder emits RGB8, whose worst-case pixel data uses three bytes per pixel.
const MAX_PNG_BYTES: usize = MAX_PIXELS;
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ImageEncoding { None, PngRgb8 }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header { packet: GatewayReply, image_encoding: ImageEncoding, image_bytes: usize }
impl Header {
    fn validate(&self) -> io::Result<()> {
        self.packet.validate_pixels(self.packet.reply.pixel_bytes)?;
        if self.packet.reply.pixel_bytes == 0 {
            if self.image_encoding != ImageEncoding::None || self.image_bytes != 0 { return Err(invalid("control-only login reply contains image payload")); }
        } else {
            FrameSize::new(self.packet.reply.width, self.packet.reply.height).check_frame()?;
            if self.image_encoding != ImageEncoding::PngRgb8 || self.image_bytes == 0 || self.image_bytes > MAX_PNG_BYTES {
                return Err(invalid("login image must be a bounded independent RGB8 PNG"));
            }
        }
        Ok(())
    }
}
pub(crate) struct EncodedReply { pub packet: GatewayReply, png: Vec<u8> }
pub(crate) async fn prepare_reply(packet: GatewayReply, pixels: Vec<u8>) -> io::Result<EncodedReply> {
    if pixels.is_empty() { packet.validate_pixels(0)?; return Ok(EncodedReply { packet, png: Vec::new() }); }
    tokio::task::spawn_blocking(move || {
        packet.validate_pixels(pixels.len())?;
        let size = FrameSize::new(packet.reply.width, packet.reply.height); size.check_frame()?;
        let image = BgraImage::from_vec(size, pixels)?;
        let png = snapshot::encode_png(&image)?;
        if png.len() > MAX_PNG_BYTES { return Err(invalid("lossless login PNG exceeds its encoded byte budget")); }
        Ok(EncodedReply { packet, png })
    }).await.map_err(io::Error::other)?
}
async fn send_frame(writer: &mut (impl AsyncWrite + Unpin), tag: u8, bytes: &[u8]) -> io::Result<()> {
    let limit = if tag == CONTROL { MAX_CONTROL } else { MAX_PNG_BYTES };
    if bytes.len() > limit { return Err(invalid("login network frame exceeds limit")); }
    writer.write_all(&[tag]).await?;
    writer.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    writer.write_all(bytes).await?; writer.flush().await
}
async fn receive_frame(reader: &mut (impl AsyncRead + Unpin), tag: u8, expected: Option<usize>) -> io::Result<Vec<u8>> {
    let mut head = [0u8; 5]; reader.read_exact(&mut head).await?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    let limit = if tag == CONTROL { MAX_CONTROL } else { MAX_PNG_BYTES };
    if head[0] != tag || len > limit || expected.is_some_and(|expected| expected != len) { return Err(invalid("unexpected login network frame or length")); }
    let mut bytes = vec![0u8; len]; reader.read_exact(&mut bytes).await?; Ok(bytes)
}
pub(crate) async fn send<T: Serialize>(writer: &mut (impl AsyncWrite + Unpin), value: &T) -> io::Result<()> {
    send_frame(writer, CONTROL, &serde_json::to_vec(value).map_err(io::Error::other)?).await
}
pub(crate) async fn receive<T: DeserializeOwned>(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<T> {
    serde_json::from_slice(&receive_frame(reader, CONTROL, None).await?).map_err(io::Error::other)
}
pub(crate) async fn send_prepared(writer: &mut (impl AsyncWrite + Unpin), encoded: &EncodedReply) -> io::Result<()> {
    let header = Header { packet: encoded.packet.clone(), image_encoding: if encoded.png.is_empty() { ImageEncoding::None } else { ImageEncoding::PngRgb8 }, image_bytes: encoded.png.len() };
    header.validate()?; send(writer, &header).await?;
    if !encoded.png.is_empty() { send_frame(writer, PNG, &encoded.png).await?; }
    Ok(())
}
/// Control-only shortcut; callers never pass raw image bytes to this method.
pub(crate) async fn send_reply(writer: &mut (impl AsyncWrite + Unpin), packet: &GatewayReply, pixels: &[u8]) -> io::Result<()> {
    if !pixels.is_empty() { return Err(invalid("login pixels must be encoded outside the privileged broker")); }
    packet.validate_pixels(0)?;
    send_prepared(writer, &EncodedReply { packet: packet.clone(), png: Vec::new() }).await
}
pub(crate) async fn receive_reply(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<(GatewayReply, Vec<u8>)> {
    let header: Header = receive(reader).await?;
    // Check declared dimensions, decoded bytes and encoded bytes before the
    // encoded-image allocation; PNG IHDR is checked again before pixel buffers.
    header.validate()?;
    if header.image_bytes == 0 { return Ok((header.packet, Vec::new())); }
    let png = receive_frame(reader, PNG, Some(header.image_bytes)).await?;
    tokio::task::spawn_blocking(move || decode_png(header.packet, png)).await.map_err(io::Error::other)?
}
fn decode_png(packet: GatewayReply, png: Vec<u8>) -> io::Result<(GatewayReply, Vec<u8>)> {
    let expected = FrameSize::new(packet.reply.width, packet.reply.height); expected.check_frame()?;
    let raw_len = expected.checked_bgra_len()?;
    if raw_len > MAX_PIXELS || raw_len != packet.reply.pixel_bytes || png.len() > MAX_PNG_BYTES { return Err(invalid("login decoded PNG exceeds the pixel budget")); }
    validate_png_chunks(&png)?;
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(png), png::Limits { bytes: MAX_PIXELS });
    let header = decoder.read_header_info().map_err(io::Error::other)?;
    if header.width != expected.width || header.height != expected.height || header.bit_depth != png::BitDepth::Eight
        || header.color_type != png::ColorType::Rgb || header.interlaced { return Err(invalid("login PNG IHDR differs from its bounded RGB8 frame contract")); }
    // read_info allocates decoder state only after the IHDR dimensions passed.
    let mut reader = decoder.read_info().map_err(io::Error::other)?;
    if reader.info().animation_control.is_some() || reader.info().frame_control.is_some() { return Err(invalid("animated login PNG is not permitted")); }
    let rgb_len = raw_len / 4 * 3;
    if reader.output_buffer_size() != Some(rgb_len) { return Err(invalid("login PNG decoder output size differs from RGB8 dimensions")); }
    let mut rgb = vec![0u8; rgb_len];
    let output = reader.next_frame(&mut rgb).map_err(io::Error::other)?;
    if output.width != expected.width || output.height != expected.height || output.bit_depth != png::BitDepth::Eight
        || output.color_type != png::ColorType::Rgb || output.buffer_size() != rgb_len { return Err(invalid("login PNG output differs from the declared frame")); }
    reader.finish().map_err(io::Error::other)?;
    let mut bgra = Vec::with_capacity(raw_len);
    for rgb in rgb.chunks_exact(3) { bgra.extend_from_slice(&[rgb[2], rgb[1], rgb[0], 255]); }
    packet.validate_pixels(bgra.len())?; Ok((packet, bgra))
}
// The existing snapshot encoder emits only IHDR/IDAT/IEND. Refuse text,
// colour-profile and animation chunks before the PNG library can allocate
// ancillary decompression buffers; this protocol is not a general PNG import.
fn validate_png_chunks(bytes: &[u8]) -> io::Result<()> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Err(invalid("login PNG signature is invalid")); }
    let mut offset = 8usize; let mut header = false; let mut data = false;
    while offset < bytes.len() {
        let prefix = bytes.get(offset..offset.checked_add(8).ok_or_else(|| invalid("PNG chunk offset overflow"))?)
            .ok_or_else(|| invalid("truncated login PNG chunk"))?;
        let len = u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]) as usize;
        let end = offset.checked_add(12).and_then(|start| start.checked_add(len)).filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("login PNG chunk exceeds the bounded payload"))?;
        match &prefix[4..8] {
            b"IHDR" if !header && offset == 8 && len == 13 => header = true,
            b"IDAT" if header => data = true,
            b"IEND" if header && data && len == 0 && end == bytes.len() => return Ok(()),
            _ => return Err(invalid("login PNG contains unsupported ancillary, animation or reordered chunks")),
        }
        offset = end;
    }
    Err(invalid("login PNG has no final IEND"))
}
fn invalid(reason: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, reason) }
