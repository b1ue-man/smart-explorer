//! Exact local PNG/decoder owners, retained through blocking work and results.
use std::{io::{self, Cursor}, sync::Arc};
use crate::screen::core::{capture_admission::{CaptureGeneration, SourceKey},
    capture_bytes::AdmittedBytes, capture_grant::CaptureCreditLifetime,
    capture_image::{CaptureImageBuilder, CapturedImage}, frame_budget::FrameLimits,
    image_admission::{ImageAdmission, ImageOwnerLimits, ImageRole}, pixel_convert::FrameSize};
use super::{gateway_types::GatewayReply, model::MAX_PIXELS};
pub(crate) const MAX_ROW: u64 = 4096 * 3;
#[derive(Clone)]
pub(crate) struct ImageReceiver {
    pub admission: Arc<ImageAdmission>, pub source: SourceKey,
    pub cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    pub lifetime: Option<Arc<dyn CaptureCreditLifetime>>,
}
impl ImageReceiver {
    pub(crate) fn limits() -> io::Result<ImageOwnerLimits> {
        ImageOwnerLimits::new(2 * MAX_PIXELS as u64 + MAX_ROW)?
            .with_role(ImageRole::EncodedInput, MAX_PIXELS as u64, 1)?
            .with_role(ImageRole::DecodedInput, MAX_PIXELS as u64, 1)?
            .with_role(ImageRole::FilterScratch, MAX_ROW, 1)
    }
    pub(crate) fn check_cancelled(&self) -> io::Result<()> {
        if (self.cancelled)() {Err(io::Error::new(io::ErrorKind::Interrupted, "login image receive cancelled"))} else {Ok(())}
    }
    pub(crate) fn prepare(&self, packet: &GatewayReply, encoded: usize) -> io::Result<ReceiveBuffers> {
        self.check_cancelled()?;
        if packet.request_sequence == 0 {return Err(invalid("login startup reply cannot contain pixels"));}
        let size = FrameSize::new(packet.reply.width, packet.reply.height); size.check_frame()?;
        let raw = size.checked_bgra_len()?; packet.validate_pixels(raw)?;
        if encoded == 0 || encoded > MAX_PIXELS {return Err(invalid("login PNG exceeds declared payload limit"));}
        let png = self.admission.try_claim(ImageRole::EncodedInput, encoded as u64)?;
        let pixels = self.admission.try_claim(ImageRole::DecodedInput, raw as u64)?;
        let row = self.admission.try_claim(ImageRole::FilterScratch, u64::from(size.width) * 3)?;
        // Every required promise exists before the first product allocation.
        Ok(ReceiveBuffers {png: AdmittedBytes::from_slice(png)?,
            image: CaptureImageBuilder::from_local_bytes(size, FrameLimits::LEGACY, &self.admission,
                self.source.clone(), CaptureGeneration::new(packet.request_sequence), AdmittedBytes::from_slice(pixels)?)?,
            row: AdmittedBytes::from_slice(row)?, _lifetime: self.lifetime.clone()})
    }
}
pub(crate) struct ReceiveBuffers {
    pub png: AdmittedBytes, image: CaptureImageBuilder, row: AdmittedBytes,
    // Error/cancel unwinding destroys every product buffer before this anchor.
    _lifetime: Option<Arc<dyn CaptureCreditLifetime>>,
}
pub(crate) fn decode(packet: GatewayReply, mut buffers: ReceiveBuffers, owner: ImageReceiver)
    -> io::Result<(GatewayReply, Option<CapturedImage>)> {
    owner.check_cancelled()?;
    validate_png_chunks(buffers.png.as_slice())?;
    let size = buffers.image.size();
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(buffers.png.as_slice()), png::Limits {bytes: MAX_PIXELS});
    let header = decoder.read_header_info().map_err(io::Error::other)?;
    if header.width != size.width || header.height != size.height || header.bit_depth != png::BitDepth::Eight
        || header.color_type != png::ColorType::Rgb || header.interlaced {
        return Err(invalid("login PNG IHDR differs from its bounded RGB8 frame contract"));
    }
    let mut reader = decoder.read_info().map_err(io::Error::other)?;
    if reader.info().animation_control.is_some() || reader.info().frame_control.is_some()
        || reader.output_line_size(size.width) != Some(buffers.row.len())
        || reader.output_buffer_size() != Some(size.checked_bgra_len()? / 4 * 3) {
        return Err(invalid("login PNG decoder output differs from RGB8 dimensions"));
    }
    for y in 0..size.height {
        owner.check_cancelled()?;
        if reader.read_row(buffers.row.as_mut_slice()).map_err(io::Error::other)?.is_none() {
            return Err(invalid("login PNG ended before its declared final row"));
        }
        let target = buffers.image.row_mut(y).ok_or_else(|| invalid("login BGRA row is absent"))?;
        for (pixel, rgb) in target.chunks_exact_mut(4).zip(buffers.row.as_slice().chunks_exact(3)) {
            pixel.copy_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
        }
    }
    if reader.read_row(buffers.row.as_mut_slice()).map_err(io::Error::other)?.is_some() {
        return Err(invalid("login PNG contains too many rows"));
    }
    reader.finish().map_err(io::Error::other)?;
    drop(reader);
    owner.check_cancelled()?;
    packet.validate_pixels(size.checked_bgra_len()?)?;
    let image = buffers.image.freeze();
    if let Some(lifetime) = owner.lifetime.clone() {image.retain_lifetime(lifetime)?;}
    Ok((packet, Some(image)))
}
// This protocol deliberately accepts only the snapshot encoder's independent
// RGB8 images. Ancillary and APNG chunks never reach library decompression.
fn validate_png_chunks(bytes: &[u8]) -> io::Result<()> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {return Err(invalid("login PNG signature is invalid"));}
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
fn invalid(reason: &str) -> io::Error {io::Error::new(io::ErrorKind::InvalidData, reason)}
