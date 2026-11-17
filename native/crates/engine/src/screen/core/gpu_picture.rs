//! A decoded picture that stays on the GPU (plan 2026-11-17 „Windows-Bild“
//! P8): the FFmpeg decoder's D3D11VA stage hands its texture to the viewer's
//! Direct3D 11 presenter, which draws it straight into the swap chain, with
//! no read-back and no upload. The surface is the platform's own type
//! (Windows: `screen::os::ffmpeg_frame::D3d11Frame`, which holds a reference
//! on the decoder's pool surface, so the decoder does not reuse it before the
//! presenter is done); this core type only carries it, so the portable
//! decode thread can pass it on without naming Windows.
use std::any::Any;

use super::pixel_convert::FrameSize;

pub struct GpuPicture {
    /// The picture's size (the stream's, without the decoder's padding).
    pub size: FrameSize,
    /// The platform surface; the presenter downcasts it.
    pub surface: Box<dyn Any + Send>,
}

impl std::fmt::Debug for GpuPicture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuPicture")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}
