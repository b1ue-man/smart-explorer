//! What the FFmpeg decoder makes of a decoded frame (plan 2026-11-17 P8):
//! a D3D11VA frame for a Direct3D 11 presenter stays a texture
//! (`GpuPicture`, no copy); any other hardware frame is read back with
//! `av_hwframe_transfer_data` (NV12), as the Media Foundation decoder does
//! for a window that draws with wgpu or glow; a software frame (NV12 or
//! I420) is copied once into the plane storage the viewer gave back. The
//! planes stay planes for the viewer's GPU path (`prefer_yuv`) or become
//! BGRA. A hardware stage that hands out a software frame fell back inside
//! FFmpeg and is refused (`ffmpeg_plan::frame_fits_stage`).
use std::io;

use super::super::ffmpeg_frame::D3d11Frame;
use super::FfmpegDecoder;
use crate::screen::core::ffmpeg_plan::{self as plan, layout, FrameKind};
use crate::screen::core::gpu_picture::GpuPicture;
use crate::screen::core::host_backend::{Chroma, YuvPicture};
use crate::screen::core::pixel_convert::{
    i420_to_bgra, nv12_to_bgra, FrameSize, I420Image, Nv12View,
};

/// Planes and row pitches of a software frame, read from its fields.
struct Planes {
    data: [*const u8; 3],
    pitch: [usize; 3],
}

fn read_i32(frame: *const u8, at: usize) -> i32 {
    // SAFETY: callers pass a live AVFrame and an offset of the pinned layout.
    unsafe { std::ptr::read_unaligned(frame.add(at) as *const i32) }
}

/// Copies `rows` rows of `width` bytes at `pitch` from `source` into `out`.
///
/// # Safety
/// `source` must hold `rows` rows of at least `width` bytes, `pitch` apart.
unsafe fn copy_rows(source: *const u8, pitch: usize, width: usize, rows: usize, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(width * rows);
    for row in 0..rows {
        // SAFETY: per the contract.
        out.extend_from_slice(unsafe {
            std::slice::from_raw_parts(source.add(row * pitch), width)
        });
    }
}

impl FfmpegDecoder {
    /// Reads the stream's colour facts from the context (`colorspace`,
    /// `color_range`, set by the decoder from the stream's VUI).
    fn colour(&mut self) {
        // SAFETY: the opened context.
        let (space, range) = unsafe {
            (
                self.ffmpeg.get_int(self.context, "colorspace"),
                self.ffmpeg.get_int(self.context, "color_range"),
            )
        };
        self.matrix = plan::yuv_matrix(space.unwrap_or(1));
        self.full_range = plan::full_range(range.unwrap_or(0));
    }

    /// Takes the frame `self.frame` received last.
    pub(super) fn take_output(&mut self) -> io::Result<()> {
        let base = self.frame as *const u8;
        let kind = self
            .ffmpeg
            .formats
            .kind(read_i32(base, layout::FRAME_FORMAT));
        if !plan::frame_fits_stage(kind, self.stage) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{}: FFmpeg delivered a {kind:?} frame", self.name),
            ));
        }
        let size = FrameSize::new(
            read_i32(base, layout::FRAME_WIDTH).max(0) as u32,
            read_i32(base, layout::FRAME_HEIGHT).max(0) as u32,
        );
        if size != self.size {
            return Err(io::Error::other(format!(
                "FFmpeg picture {}x{} instead of {}x{}",
                size.width, size.height, self.size.width, self.size.height
            )));
        }
        self.colour();
        let presenter_device = self.device.clone().filter(|_| self.gpu);
        if let (FrameKind::D3d11, Some(device)) = (kind, presenter_device) {
            // SAFETY: a decoded D3D11VA frame of this decoder's device.
            let frame = unsafe {
                D3d11Frame::reference(
                    &self.ffmpeg,
                    &device,
                    self.frame,
                    size,
                    self.matrix,
                    self.full_range,
                )?
            };
            self.gpu_picture = Some(GpuPicture {
                size,
                surface: Box::new(frame),
            });
            return Ok(());
        }
        match kind {
            FrameKind::D3d11 | FrameKind::Dxva2 => {
                // SAFETY: `transfer` is the decoder's own frame, emptied
                // first; FFmpeg allocates its planes (NV12).
                unsafe {
                    (self.ffmpeg.frame_unref)(self.transfer);
                    let code = (self.ffmpeg.hwframe_transfer_data)(self.transfer, self.frame, 0);
                    self.ffmpeg.check(code, "av_hwframe_transfer_data")?;
                }
                let source = self.transfer as *const u8;
                let kind = self
                    .ffmpeg
                    .formats
                    .kind(read_i32(source, layout::FRAME_FORMAT));
                let result = self.keep_planes(source, kind);
                // SAFETY: the read-back's planes are copied.
                unsafe { (self.ffmpeg.frame_unref)(self.transfer) };
                result
            }
            FrameKind::Nv12 | FrameKind::Yuv420p => self.keep_planes(base, kind),
            FrameKind::Other => Err(io::Error::other("FFmpeg frame of an unexpected format")),
        }
    }

    fn planes_of(&self, frame: *const u8, kind: FrameKind) -> io::Result<Planes> {
        let count = if kind == FrameKind::Nv12 { 2 } else { 3 };
        let mut planes = Planes {
            data: [std::ptr::null(); 3],
            pitch: [0; 3],
        };
        let chroma = self.size.chroma();
        let widths = [
            self.size.width as usize,
            if kind == FrameKind::Nv12 {
                chroma.width as usize * 2
            } else {
                chroma.width as usize
            },
            chroma.width as usize,
        ];
        for plane in 0..count {
            // SAFETY: `data[plane]` and `linesize[plane]` of a live frame.
            let data = unsafe {
                std::ptr::read_unaligned(
                    frame.add(layout::FRAME_DATA + 8 * plane) as *const *const u8
                )
            };
            let pitch = read_i32(frame, layout::FRAME_LINESIZE + 4 * plane);
            let pitch = usize::try_from(pitch).unwrap_or(0);
            if data.is_null() || pitch < widths[plane] {
                return Err(io::Error::other(
                    "FFmpeg frame planes do not fit the picture",
                ));
            }
            planes.data[plane] = data;
            planes.pitch[plane] = pitch;
        }
        Ok(planes)
    }

    /// Copies the planes of an NV12 or I420 frame into the kept storage and
    /// makes them the output (planes) or converts them (BGRA).
    fn keep_planes(&mut self, frame: *const u8, kind: FrameKind) -> io::Result<()> {
        if !matches!(kind, FrameKind::Nv12 | FrameKind::Yuv420p) {
            return Err(io::Error::other(
                "FFmpeg read-back is neither NV12 nor I420",
            ));
        }
        let planes = self.planes_of(frame, kind)?;
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        let chroma = self.size.chroma();
        let (chroma_width, chroma_rows) = (chroma.width as usize, chroma.height as usize);
        let mut out = self
            .spare
            .take()
            .or_else(|| self.planes.take())
            .unwrap_or_else(|| YuvPicture {
                y: Vec::new(),
                y_stride: 0,
                uv: Vec::new(),
                uv_stride: 0,
                chroma: Chroma::Nv12,
                matrix: self.matrix,
            });
        // SAFETY: `planes_of` checked every pointer and that each pitch
        // holds the row; FFmpeg's planes have the picture's rows.
        unsafe {
            copy_rows(planes.data[0], planes.pitch[0], width, height, &mut out.y);
            if kind == FrameKind::Nv12 {
                copy_rows(
                    planes.data[1],
                    planes.pitch[1],
                    chroma_width * 2,
                    chroma_rows,
                    &mut out.uv,
                );
                out.chroma = Chroma::Nv12;
                out.uv_stride = chroma_width * 2;
            } else {
                let (mut u, mut v) = match std::mem::replace(&mut out.chroma, Chroma::Nv12) {
                    Chroma::I420 { u, v } => (u, v),
                    Chroma::Nv12 => (Vec::new(), Vec::new()),
                };
                copy_rows(
                    planes.data[1],
                    planes.pitch[1],
                    chroma_width,
                    chroma_rows,
                    &mut u,
                );
                copy_rows(
                    planes.data[2],
                    planes.pitch[2],
                    chroma_width,
                    chroma_rows,
                    &mut v,
                );
                out.uv.clear();
                out.chroma = Chroma::I420 { u, v };
                out.uv_stride = chroma_width;
            }
        }
        out.y_stride = width;
        out.matrix = self.matrix;
        if self.yuv {
            self.planes = Some(out);
            return Ok(());
        }
        let result = match &out.chroma {
            Chroma::Nv12 => Nv12View::new_with_limits(
                self.size,
                &out.y,
                out.y_stride,
                &out.uv,
                out.uv_stride,
                self.limits,
            )
            .and_then(|view| nv12_to_bgra(&view, self.matrix, &mut self.picture)),
            Chroma::I420 { u, v } => {
                let image = I420Image {
                    size: self.size,
                    y: std::mem::take(&mut out.y),
                    u: u.clone(),
                    v: v.clone(),
                };
                let converted = i420_to_bgra(&image, self.matrix, &mut self.picture);
                out.y = image.y;
                converted
            }
        };
        self.spare = Some(out);
        result
    }
}
