//! A picture of FFmpeg's D3D11VA stage on its way to the screen (plan
//! 2026-11-17 P8): one reference (`av_frame_ref`) on the decoded frame, so
//! the decoder's pool surface (one slice of its texture array) is not handed
//! to the decoder again before the presenter drops this; the texture and its
//! array slice (`AVFrame.data[0]` and `data[1]`,
//! `libavutil/hwcontext_d3d11va.h:123-148`) on the viewer's device, and the
//! colour facts of the stream. The presenter (`G/os/windows/
//! screen_d3d11_present.rs`) reads it through `GpuPicture::surface`.
use std::sync::Arc;

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::ffmpeg_ffi::{Ffmpeg, Raw};
use super::viewer_d3d11::ViewerDevice;
use crate::screen::core::ffmpeg_plan::layout;
use crate::screen::core::pixel_convert::{FrameSize, YuvMatrix};

pub struct D3d11Frame {
    frame: Raw,
    ffmpeg: Arc<Ffmpeg>,
    device: Arc<ViewerDevice>,
    texture: ID3D11Texture2D,
    index: u32,
    size: FrameSize,
    matrix: YuvMatrix,
    full_range: bool,
}

// SAFETY: the frame reference is owned by this value alone and only
// released in `drop`; FFmpeg's buffer references are thread-safe; the
// texture belongs to the free-threaded viewer device.
unsafe impl Send for D3d11Frame {}

impl D3d11Frame {
    /// A new reference on `decoded`, a frame of format `d3d11`.
    ///
    /// # Safety
    /// `decoded` must be a live decoded `AVFrame` of the D3D11VA stage whose
    /// device context uses `device`.
    pub(super) unsafe fn reference(
        ffmpeg: &Arc<Ffmpeg>,
        device: &Arc<ViewerDevice>,
        decoded: Raw,
        size: FrameSize,
        matrix: YuvMatrix,
        full_range: bool,
    ) -> std::io::Result<Self> {
        // SAFETY: per the contract; the new frame is freed on every path.
        unsafe {
            let mut frame = (ffmpeg.frame_alloc)();
            if frame.is_null() {
                return Err(std::io::Error::other("av_frame_alloc failed"));
            }
            if let Err(error) = ffmpeg.check((ffmpeg.frame_ref)(frame, decoded), "av_frame_ref") {
                (ffmpeg.frame_free)(&mut frame);
                return Err(error);
            }
            let base = frame as *const u8;
            let texture_raw = std::ptr::read_unaligned(base.add(layout::FRAME_DATA) as *const Raw);
            let index = std::ptr::read_unaligned(base.add(layout::FRAME_DATA + 8) as *const isize);
            let texture = ID3D11Texture2D::from_raw_borrowed(&texture_raw).cloned();
            let (Some(texture), Ok(index)) = (texture, u32::try_from(index)) else {
                (ffmpeg.frame_free)(&mut frame);
                return Err(std::io::Error::other("D3D11VA frame without a texture"));
            };
            Ok(Self {
                frame,
                ffmpeg: ffmpeg.clone(),
                device: device.clone(),
                texture,
                index,
                size,
                matrix,
                full_range,
            })
        }
    }

    /// The decoder's texture array.
    pub fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }

    /// The picture's slice of `texture`.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The device the texture lives on (the presenter's own).
    pub fn device(&self) -> &Arc<ViewerDevice> {
        &self.device
    }

    /// The picture's size, without the decoder's padding.
    pub fn size(&self) -> FrameSize {
        self.size
    }

    pub fn matrix(&self) -> YuvMatrix {
        self.matrix
    }

    pub fn full_range(&self) -> bool {
        self.full_range
    }
}

impl Drop for D3d11Frame {
    fn drop(&mut self) {
        // SAFETY: the reference made in `reference`, released once; the pool
        // surface goes back to the decoder.
        unsafe { (self.ffmpeg.frame_free)(&mut self.frame) };
    }
}
