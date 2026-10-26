//! Bounded raster conversion. Remote requests never choose source paths.
use crate::screen::core::app_icon::{self, IconReply, MAX_ICON_BYTES};
use std::io;

pub(crate) fn rgba_png(width: u32, height: u32, rgba: &[u8]) -> io::Result<Vec<u8>> {
    let pixels = app_icon::checked_pixels(width, height)?;
    if rgba.len() != pixels * 4 {
        return Err(app_icon::invalid("invalid icon RGBA length"));
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|_| app_icon::invalid("icon PNG encoder failed"))?;
        writer
            .write_image_data(rgba)
            .map_err(|_| app_icon::invalid("icon PNG encoding failed"))?;
        writer
            .finish()
            .map_err(|_| app_icon::invalid("icon PNG completion failed"))?;
    }
    if bytes.len() > MAX_ICON_BYTES {
        return Err(app_icon::invalid("encoded icon exceeds budget"));
    }
    Ok(bytes)
}

pub(crate) fn raster_png(app_id: &str, bytes: &[u8]) -> io::Result<IconReply> {
    app_icon::decode_png(bytes)?;
    IconReply::new(app_id, bytes)
}
