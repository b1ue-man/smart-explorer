//! The D3D11 video processor of the Windows GPU path (plan 2026-11-14
//! bildkette B-17, package BK2a): BGRA → NV12 and scaling on the device that
//! captured the picture, NV12 textures for the encoder, and the ring of two
//! NV12 pictures the capture thread writes and the encoder reads
//! (`d3d_video_ring.rs`).
//!
//! Colour: the CPU reference (`core/pixel_convert.rs`, `YuvMatrix::Bt709`)
//! takes full-range RGB and writes BT.709 limited range (Y 16–235, chroma
//! 16–240 around 128). The processor gets the same pair: input RGB full
//! range, output YCbCr BT.709 with nominal range 16–235
//! (`D3D11_VIDEO_PROCESSOR_COLOR_SPACE`). Chroma siting and the scaling
//! filter are the driver's; the CPU path averages 2 × 2 for chroma and uses a
//! box filter for scaling.
//!
//! Plan 2026-11-17 P9 (`overlay`): a second pass of the same processor
//! draws a small BGRA patch (the pointer blended onto the screen under it)
//! into an area of the picture the first pass wrote. The output target
//! rectangle limits the pass to that area ("If this method is never called,
//! or if the Enable parameter is FALSE, the video processor writes to the
//! entire destination surface", `VideoProcessorSetOutputTargetRect`), so the
//! rest of the picture stays.
use std::io;
use std::mem::ManuallyDrop;

use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice,
    ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
    ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET, D3D11_BIND_VIDEO_ENCODER,
    D3D11_CPU_ACCESS_FLAG, D3D11_MAP, D3D11_MAPPED_SUBRESOURCE, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT,
    D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};

use crate::screen::core::pixel_convert::{FrameSize, Rect};

#[path = "d3d_video_ring.rs"]
mod ring;
pub(in crate::screen::os) use ring::Ring;

/// `D3D11_VIDEO_PROCESSOR_COLOR_SPACE` bits (LSB first: Usage, RGB_Range,
/// YCbCr_Matrix, YCbCr_xvYCC, Nominal_Range ×2): RGB, full range 0–255.
pub(in crate::screen::os) const RGB_FULL_RANGE: u32 = 0;
/// YCbCr_Matrix 1 (BT.709) and Nominal_Range 1 (16–235).
pub(in crate::screen::os) const YCBCR_BT709_LIMITED: u32 = (1 << 2) | (1 << 4);

/// Bytes of an NV12 picture of `size`: luma plus interleaved 4:2:0 chroma
/// (the size the codec plan admits per `CodecInput` image).
pub(in crate::screen::os) fn nv12_len(size: FrameSize) -> u64 {
    size.pixels() + size.chroma().pixels() * 2
}

/// NV12 needs even sides: the slots hold the largest even picture inside
/// the capture (an odd column or row of a window is dropped by scaling).
pub(in crate::screen::os) fn slot_size(native: FrameSize) -> FrameSize {
    native.even()
}

fn rect(size: FrameSize) -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: size.width as i32,
        bottom: size.height as i32,
    }
}

fn area(area: Rect) -> RECT {
    RECT {
        left: area.x as i32,
        top: area.y as i32,
        right: (area.x + area.width) as i32,
        bottom: (area.y + area.height) as i32,
    }
}

fn unsupported(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, what.to_string())
}

/// One video processor from `input` (BGRA or NV12) to NV12 `output`.
pub(in crate::screen::os) struct VideoConverter {
    video: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    pub input: FrameSize,
    output: FrameSize,
}

// SAFETY: the converter is used by one thread at a time (it lives behind a
// mutex or in one encoder); the immediate context it wraps is switched to
// multithread protection by `d3d_device::create_on`.
unsafe impl Send for VideoConverter {}

impl VideoConverter {
    pub(in crate::screen::os) fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        input: FrameSize,
        format: DXGI_FORMAT,
        output: FrameSize,
    ) -> io::Result<Self> {
        let video: ID3D11VideoDevice = device.cast().map_err(io::Error::other)?;
        let vcontext: ID3D11VideoContext = context.cast().map_err(io::Error::other)?;
        let rate = DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: input.width,
            InputHeight: input.height,
            OutputFrameRate: rate,
            OutputWidth: output.width,
            OutputHeight: output.height,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        // SAFETY: `desc` lives through the call.
        let enumerator =
            unsafe { video.CreateVideoProcessorEnumerator(&desc) }.map_err(io::Error::other)?;
        let supports = |format: DXGI_FORMAT, flag: i32| {
            // SAFETY: a plain query of the enumerator.
            unsafe { enumerator.CheckVideoProcessorFormat(format) }
                .is_ok_and(|bits| bits & flag as u32 != 0)
        };
        if !supports(format, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0)
            || !supports(
                DXGI_FORMAT_NV12,
                D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0,
            )
        {
            return Err(unsupported("the video processor cannot convert to NV12"));
        }
        // SAFETY: a live enumerator of this device.
        let processor =
            unsafe { video.CreateVideoProcessor(&enumerator, 0) }.map_err(io::Error::other)?;
        let source = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
            _bitfield: if format == DXGI_FORMAT_NV12 {
                YCBCR_BT709_LIMITED
            } else {
                RGB_FULL_RANGE
            },
        };
        let target = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
            _bitfield: YCBCR_BT709_LIMITED,
        };
        let (from, to) = (rect(input), rect(output));
        // SAFETY: state setters of the processor just created; every pointer
        // addresses a local that lives through its call.
        unsafe {
            vcontext.VideoProcessorSetStreamColorSpace(&processor, 0, &source);
            vcontext.VideoProcessorSetOutputColorSpace(&processor, &target);
            vcontext.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            // No driver "enhancement": the picture is converted, nothing else.
            vcontext.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            vcontext.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&from));
            vcontext.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&to));
            vcontext.VideoProcessorSetOutputTargetRect(&processor, true, Some(&to));
        }
        Ok(Self {
            video,
            context: vcontext,
            enumerator,
            processor,
            input,
            output,
        })
    }

    /// The output view of an NV12 texture of the output size.
    pub(in crate::screen::os) fn output_view(
        &self,
        texture: &ID3D11Texture2D,
    ) -> io::Result<ID3D11VideoProcessorOutputView> {
        let desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut view = None;
        // SAFETY: a live texture of this device; `desc` and `view` are locals.
        unsafe {
            self.video.CreateVideoProcessorOutputView(
                texture,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )
        }
        .map_err(io::Error::other)?;
        view.ok_or_else(|| io::Error::other("no video processor output view"))
    }

    /// Converts (and scales) the top left `input` area of `source` into the
    /// texture behind `target`. The input view lives only for this call, so
    /// no reference to a capture surface outlives its frame.
    pub(in crate::screen::os) fn convert(
        &self,
        source: &ID3D11Texture2D,
        target: &ID3D11VideoProcessorOutputView,
    ) -> io::Result<()> {
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut view: Option<ID3D11VideoProcessorInputView> = None;
        // SAFETY: a live texture of this device; `desc` and `view` are locals.
        unsafe {
            self.video.CreateVideoProcessorInputView(
                source,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )
        }
        .map_err(io::Error::other)?;
        let mut streams = [D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(view),
            ..Default::default()
        }];
        // SAFETY: one enabled stream with a live input view; the output view
        // belongs to a texture of the same device.
        let result = unsafe {
            self.context
                .VideoProcessorBlt(&self.processor, target, 0, &streams)
        };
        // SAFETY: the view put into the stream above is released once.
        unsafe { ManuallyDrop::drop(&mut streams[0].pInputSurface) };
        result.map_err(io::Error::other)
    }

    /// P9: draws the BGRA `patch` (of `at`'s size) at `at` of the picture
    /// behind `target`, which `convert` wrote before; nothing outside `at`
    /// changes. `at` lies inside the output and has even edges, so every
    /// 2 × 2 chroma pair is written whole. The rectangles of the whole
    /// picture are set again afterwards, also when the pass fails.
    pub(in crate::screen::os) fn overlay(
        &self,
        patch: &ID3D11Texture2D,
        at: Rect,
        target: &ID3D11VideoProcessorOutputView,
    ) -> io::Result<()> {
        if !at.fits(self.output) {
            return Err(unsupported("pointer patch lies outside the picture"));
        }
        let (from, to) = (rect(FrameSize::new(at.width, at.height)), area(at));
        // SAFETY: state setters of this converter's processor; the rects are
        // locals that live through each call.
        unsafe {
            self.context
                .VideoProcessorSetStreamSourceRect(&self.processor, 0, true, Some(&from));
            self.context
                .VideoProcessorSetStreamDestRect(&self.processor, 0, true, Some(&to));
            self.context
                .VideoProcessorSetOutputTargetRect(&self.processor, true, Some(&to));
        }
        let result = self.convert(patch, target);
        let (whole_in, whole_out) = (rect(self.input), rect(self.output));
        // SAFETY: as above; restores the rectangles `new` set.
        unsafe {
            self.context.VideoProcessorSetStreamSourceRect(
                &self.processor,
                0,
                true,
                Some(&whole_in),
            );
            self.context.VideoProcessorSetStreamDestRect(
                &self.processor,
                0,
                true,
                Some(&whole_out),
            );
            self.context
                .VideoProcessorSetOutputTargetRect(&self.processor, true, Some(&whole_out));
        }
        result
    }
}

/// An NV12 texture of `size`: a video processor target the encoder can take
/// (`cpu` none), or a staging texture with `cpu` access.
pub(in crate::screen::os) fn nv12_texture(
    device: &ID3D11Device,
    size: FrameSize,
    cpu: Option<D3D11_CPU_ACCESS_FLAG>,
) -> io::Result<ID3D11Texture2D> {
    if size != size.even() {
        return Err(unsupported("NV12 textures need even sides"));
    }
    let desc = |bind: i32| D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: if cpu.is_some() {
            D3D11_USAGE_STAGING
        } else {
            D3D11_USAGE_DEFAULT
        },
        BindFlags: bind as u32,
        CPUAccessFlags: cpu.map_or(0, |flag| flag.0 as u32),
        MiscFlags: 0,
    };
    let create = |desc: D3D11_TEXTURE2D_DESC| {
        let mut texture = None;
        // SAFETY: checked even dimensions and a live out pointer.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .map_err(io::Error::other)?;
        texture.ok_or_else(|| io::Error::other("no NV12 texture"))
    };
    if cpu.is_some() {
        return create(desc(0));
    }
    // Encoder input where the runtime knows the flag, else a plain target.
    create(desc(
        D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_VIDEO_ENCODER.0,
    ))
    .or_else(|_| create(desc(D3D11_BIND_RENDER_TARGET.0)))
}

/// Maps NV12 staging `texture` and hands its luma plane, its chroma plane
/// and the row pitch to `body`. The chroma plane starts `pitch × height`
/// bytes after the luma plane (layout of a mapped NV12 subresource).
pub(in crate::screen::os) fn with_mapped<T>(
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    size: FrameSize,
    mode: D3D11_MAP,
    body: impl FnOnce(&mut [u8], &mut [u8], usize) -> io::Result<T>,
) -> io::Result<T> {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: a staging texture with CPU access of `mode`.
    unsafe { context.Map(texture, 0, mode, 0, Some(&mut mapped)) }.map_err(io::Error::other)?;
    let pitch = mapped.RowPitch as usize;
    let luma = pitch.checked_mul(size.height as usize);
    let chroma = pitch.checked_mul(size.chroma().height as usize);
    let result = match (luma, chroma) {
        (Some(luma), Some(chroma))
            if !mapped.pData.is_null()
                && pitch >= size.width as usize
                && luma
                    .checked_add(chroma)
                    .is_some_and(|n| n <= isize::MAX as usize) =>
        {
            // SAFETY: Map gives `RowPitch` bytes for each luma row followed
            // by the chroma rows; the spans were checked above and end
            // before Unmap.
            let all =
                unsafe { std::slice::from_raw_parts_mut(mapped.pData.cast::<u8>(), luma + chroma) };
            let (y, uv) = all.split_at_mut(luma);
            body(y, uv, pitch)
        }
        _ => Err(io::Error::other(
            "mapped NV12 texture has an unusable pitch",
        )),
    };
    // SAFETY: Map succeeded above.
    unsafe { context.Unmap(texture, 0) };
    result
}
