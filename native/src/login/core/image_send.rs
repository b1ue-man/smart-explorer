//! Sender BGRA, RGB row and PNG capacities own promises before allocation.
use std::{io::{self, Write}, sync::Arc};
use crate::screen::core::{capture_bytes::{AdmittedBytes, FrozenBytes}, capture_image::CapturedImage,
    image_admission::{ImageAdmission, ImageOwnerLimits, ImageRole}};
use super::{gateway_types::GatewayReply, image_receive::MAX_ROW, model::MAX_PIXELS};
pub(crate) fn limits() -> io::Result<ImageOwnerLimits> {
    // One raw image plus old/new PNG allocations during bounded growth.
    // Admission claims actual capacities, never this whole envelope upfront.
    ImageOwnerLimits::new(3 * MAX_PIXELS as u64 + MAX_ROW)?
        .with_role(ImageRole::DecodedInput, MAX_PIXELS as u64, 1)?
        .with_role(ImageRole::EncodedInput, MAX_PIXELS as u64, 2)?
        .with_role(ImageRole::FilterScratch, MAX_ROW, 1)
}
struct Output {bytes: Option<AdmittedBytes>, admission: Arc<ImageAdmission>}
impl Write for Output {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {return Ok(0);}
        let old_length = self.bytes.as_ref().map_or(0, AdmittedBytes::len);
        let length = old_length.checked_add(data.len()).filter(|n|*n<=MAX_PIXELS)
            .ok_or_else(||io::Error::new(io::ErrorKind::InvalidData,"lossless login PNG exceeds encoded budget"))?;
        let capacity = self.bytes.as_ref().map_or(0, |bytes|bytes.capacity() as usize);
        if length > capacity {
            let next_capacity = capacity.saturating_mul(2).max(length).min(MAX_PIXELS);
            // Both old and replacement buffers are charged during the copy.
            // This avoids an unbounded Vec reallocation or a 36 MiB minimum
            // for a tiny PNG. The fixed replacement never grows afterwards.
            let slice = self.admission.try_claim(ImageRole::EncodedInput,next_capacity as u64)?;
            let mut next = AdmittedBytes::buffer_from_slice(slice)?;
            if let Some(old) = &self.bytes {next.extend_from_slice(old.as_slice())?;}
            self.bytes = Some(next);
        }
        self.bytes.as_mut().ok_or_else(||io::Error::other("login PNG buffer missing"))?.extend_from_slice(data)?;
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {Ok(())}
}
pub(crate) fn encode(packet: &GatewayReply, image: &CapturedImage, admission: &Arc<ImageAdmission>) -> io::Result<FrozenBytes> {
    packet.validate_pixels(image.image().data().len())?;
    let size = image.size(); size.check_frame()?;
    if size.width != packet.reply.width || size.height != packet.reply.height {
        return Err(io::Error::new(io::ErrorKind::InvalidData,"login sender image differs from reply geometry"));
    }
    let mut row = AdmittedBytes::from_slice(admission.try_claim(ImageRole::FilterScratch,u64::from(size.width)*3)?)?;
    let mut output = Output {bytes:None,admission:admission.clone()};
    {
        let mut encoder = png::Encoder::new(&mut output,size.width,size.height);
        encoder.set_color(png::ColorType::Rgb);encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::default());
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        {
            let mut stream = writer.stream_writer().map_err(io::Error::other)?;
            for y in 0..size.height {
                for (rgb,bgra) in row.as_mut_slice().chunks_exact_mut(3).zip(image.view().row(y).chunks_exact(4)) {
                    rgb.copy_from_slice(&[bgra[2],bgra[1],bgra[0]]);
                }
                stream.write_all(row.as_slice())?;
            }
            stream.finish().map_err(io::Error::other)?;
        }
        writer.finish().map_err(io::Error::other)?;
    }
    Ok(output.bytes.ok_or_else(||io::Error::other("login PNG encoder produced no data"))?.freeze())
}
