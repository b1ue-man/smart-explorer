//! The CPU path of the desktop viewer (SCR-M7, U25-5): a BGRA picture becomes
//! an egui texture. It serves the picture of the tile codec (lossless view,
//! tiles of the group view) and every picture when there is no wgpu surface
//! or its shader failed.
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureId, TextureOptions};

use crate::screen::core::pixel_convert::BgraImage;

/// A BGRA picture as an opaque egui image.
pub(crate) fn color_image(image: &BgraImage) -> ColorImage {
    let size = image.size();
    let pixels = image
        .data()
        .chunks_exact(4)
        .map(|bgra| Color32::from_rgb(bgra[2], bgra[1], bgra[0]))
        .collect();
    ColorImage {
        size: [size.width as usize, size.height as usize],
        pixels,
    }
}

/// The texture of the newest picture; replaced in place.
#[derive(Default)]
pub(crate) struct CpuTexture {
    handle: Option<TextureHandle>,
}

impl CpuTexture {
    /// Uploads `image`. The filter is linear; at 1:1 the texels meet the
    /// screen pixels exactly, so it shows the same as nearest.
    pub(crate) fn update(&mut self, ctx: &egui::Context, image: &BgraImage) {
        let options = TextureOptions::LINEAR;
        let color = color_image(image);
        match self.handle.as_mut() {
            Some(handle) => handle.set(color, options),
            None => {
                self.handle = Some(ctx.load_texture("screen-viewer", color, options));
            }
        }
    }

    /// The texture to draw, once a picture was uploaded.
    pub(crate) fn id(&self) -> Option<TextureId> {
        self.handle.as_ref().map(TextureHandle::id)
    }
}
