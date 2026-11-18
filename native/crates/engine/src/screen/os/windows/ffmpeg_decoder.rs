//! FFmpeg's H.264, HEVC and AV1 decoders for the Windows viewer (plan
//! 2026-11-17 P8), one stage per decoder (`ffmpeg_plan::FfmpegStage`):
//! D3D11VA on the viewer's device (`viewer_d3d11.rs`, the device handed to
//! FFmpeg in `AVD3D11VADeviceContext` with the device's own lock), DXVA2 on a
//! device FFmpeg makes itself (`av_hwdevice_ctx_create`), or software. Each
//! record is one access unit or temporal unit: it goes in with
//! `avcodec_send_packet` (its counter as `pts`), and every frame ready now
//! comes out with `avcodec_receive_frame`; with `low_delay`, one thread for
//! the hardware and slice threads for software (`decoder_options`) the
//! picture of a record comes out for that record (se hosts send no B
//! frames). The output's `pts` names the record it belongs to
//! (`FrameIdentities`), as the Media Foundation decoder does with sample
//! times. Output paths: `ffmpeg_output.rs`.
use std::ffi::{c_int, CString};
use std::io;
use std::sync::Arc;

use windows::core::Interface;

use super::ffmpeg_ffi::{Ffmpeg, Raw};
use super::viewer_d3d11::{self, ViewerDevice};
use crate::screen::core::decoded_frame::{DecodedFrame, DecoderFallback, FrameIdentities};
use crate::screen::core::ffmpeg_plan::{self as plan, layout, FfmpegStage};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::gpu_picture::GpuPicture;
use crate::screen::core::host_backend::{ScreenDecoder, YuvPicture};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize, YuvMatrix};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};

#[path = "ffmpeg_output.rs"]
mod output;

pub(super) struct FfmpegDecoder {
    ffmpeg: Arc<Ffmpeg>,
    context: Raw,
    packet: Raw,
    frame: Raw,
    /// Target of `av_hwframe_transfer_data` (read-back of a hardware frame).
    transfer: Raw,
    codec: CodecKind,
    stage: FfmpegStage,
    /// The viewer device of the D3D11VA stage.
    device: Option<Arc<ViewerDevice>>,
    name: String,
    size: FrameSize,
    limits: FrameLimits,
    frames: i64,
    identities: FrameIdentities,
    decoded: Option<DecodedFrame>,
    produced: bool,
    yuv: bool,
    gpu: bool,
    matrix: YuvMatrix,
    full_range: bool,
    planes: Option<YuvPicture>,
    spare: Option<YuvPicture>,
    gpu_picture: Option<GpuPicture>,
    picture: BgraImage,
    spare_bgra: Option<BgraImage>,
}

// SAFETY: one thread at a time (the decoder is borrowed mutably); FFmpeg's
// context is used by that thread only, the viewer device is free-threaded.
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    /// Opens `codec` for the group of `header` at `stage`.
    pub(super) fn open(
        ffmpeg: Arc<Ffmpeg>,
        header: &GopHeader,
        stage: FfmpegStage,
    ) -> io::Result<Self> {
        let limits = header
            .frame_contract
            .map_or(FrameLimits::LEGACY, |c| c.limits);
        limits.check(header.size)?;
        let name = plan::decoder_name(header.codec, stage)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "FFmpeg decodes no tiles"))?;
        let device = match stage {
            FfmpegStage::D3d11va => {
                let device = viewer_d3d11::shared()?;
                if !device.decodes(header.codec, Some(header.size)) {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "the viewer's GPU does not decode this stream",
                    ));
                }
                Some(device)
            }
            _ => None,
        };
        let picture = BgraImage::new_with_limits(header.size, limits)?;
        let c_name = CString::new(name).map_err(io::Error::other)?;
        // SAFETY: a name lookup; the codec lives in the DLL.
        let codec = unsafe { (ffmpeg.find_decoder_by_name)(c_name.as_ptr()) };
        if codec.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("FFmpeg has no decoder {name}"),
            ));
        }
        // SAFETY: allocations of the loaded FFmpeg; `Drop` frees each one,
        // including the ones a failed open leaves behind.
        let mut decoder = unsafe {
            Self {
                context: (ffmpeg.alloc_context3)(codec),
                packet: (ffmpeg.packet_alloc)(),
                frame: (ffmpeg.frame_alloc)(),
                transfer: (ffmpeg.frame_alloc)(),
                ffmpeg,
                codec: header.codec,
                stage,
                device,
                name: plan::stage_name(header.codec, stage),
                size: header.size,
                limits,
                frames: 0,
                identities: FrameIdentities::default(),
                decoded: None,
                produced: false,
                yuv: false,
                gpu: false,
                matrix: YuvMatrix::Bt709,
                full_range: false,
                planes: None,
                spare: None,
                gpu_picture: None,
                picture,
                spare_bgra: None,
            }
        };
        if [
            decoder.context,
            decoder.packet,
            decoder.frame,
            decoder.transfer,
        ]
        .iter()
        .any(|pointer| pointer.is_null())
        {
            return Err(io::Error::other("FFmpeg allocation failed"));
        }
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        // SAFETY: `context` is a fresh AVCodecContext (AVClass first).
        unsafe {
            decoder
                .ffmpeg
                .set_options(decoder.context, &plan::decoder_options(stage, cpus))?;
        }
        if stage.hardware() {
            decoder.attach_device()?;
        }
        // SAFETY: the configured context and its codec; no options dict.
        let code = unsafe { (decoder.ffmpeg.open2)(decoder.context, codec, std::ptr::null_mut()) };
        decoder.ffmpeg.check(code, "avcodec_open2")?;
        Ok(decoder)
    }

    /// The hardware device of the stage as `AVCodecContext.hw_device_ctx`.
    fn attach_device(&mut self) -> io::Result<()> {
        let ffmpeg = self.ffmpeg.clone();
        // SAFETY: `context` is a live, unopened AVCodecContext.
        let (hwaccel_flags, extra_hw_frames) = unsafe {
            (
                ffmpeg.option_offset(self.context, "hwaccel_flags"),
                ffmpeg.option_offset(self.context, "extra_hw_frames"),
            )
        };
        let offset = hwaccel_flags
            .zip(extra_hw_frames)
            .and_then(|(flags, extra)| plan::hw_device_offset(flags, extra))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the FFmpeg codec context has no hw_device_ctx where the headers put it",
                )
            })?;
        let mut device_ref: Raw = std::ptr::null_mut();
        // SAFETY: FFmpeg calls with the documented signatures; the
        // AVD3D11VADeviceContext is filled before `av_hwdevice_ctx_init` as
        // `libavutil/hwcontext_d3d11va.h:45-111` describes, with references
        // FFmpeg releases itself (`device`, `device_context`).
        unsafe {
            match (self.stage, &self.device) {
                (FfmpegStage::D3d11va, Some(device)) => {
                    device_ref = (ffmpeg.hwdevice_ctx_alloc)(ffmpeg.d3d11va);
                    if device_ref.is_null() {
                        return Err(io::Error::other("av_hwdevice_ctx_alloc failed"));
                    }
                    let context = read_ptr(device_ref, layout::BUFFER_DATA);
                    let hwctx = read_ptr(context, layout::HWDEVICE_HWCTX);
                    write_ptr(
                        hwctx,
                        layout::D3D11VA_DEVICE,
                        device.device.clone().into_raw(),
                    );
                    write_ptr(
                        hwctx,
                        layout::D3D11VA_DEVICE_CONTEXT,
                        device.context.clone().into_raw(),
                    );
                    write_ptr(
                        hwctx,
                        layout::D3D11VA_LOCK,
                        viewer_d3d11::lock_callback as usize as Raw,
                    );
                    write_ptr(
                        hwctx,
                        layout::D3D11VA_UNLOCK,
                        viewer_d3d11::unlock_callback as usize as Raw,
                    );
                    write_ptr(hwctx, layout::D3D11VA_LOCK_CTX, device.lock_context());
                    if let Err(error) = ffmpeg.check(
                        (ffmpeg.hwdevice_ctx_init)(device_ref),
                        "av_hwdevice_ctx_init",
                    ) {
                        (ffmpeg.buffer_unref)(&mut device_ref);
                        return Err(error);
                    }
                }
                (FfmpegStage::Dxva2, _) => {
                    let code = (ffmpeg.hwdevice_ctx_create)(
                        &mut device_ref,
                        ffmpeg.dxva2,
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        0,
                    );
                    ffmpeg.check(code, "av_hwdevice_ctx_create(dxva2)")?;
                }
                _ => return Err(io::Error::other("hardware stage without its device")),
            }
            if !read_ptr(self.context, offset).is_null() {
                (ffmpeg.buffer_unref)(&mut device_ref);
                return Err(io::Error::other("hw_device_ctx was already set"));
            }
            // The reference passes to the context, which frees it.
            write_ptr(self.context, offset, device_ref);
        }
        Ok(())
    }

    fn send(&mut self, record: &FrameRecord, token: i64) -> io::Result<c_int> {
        let length = c_int::try_from(record.payload.len())
            .map_err(|_| io::Error::other("FFmpeg packet too large"))?;
        let ffmpeg = self.ffmpeg.clone();
        // SAFETY: `av_new_packet` gives `length` writable bytes (plus zeroed
        // padding) at `data`; pts and flags are fields of the pinned layout.
        unsafe {
            ffmpeg.check((ffmpeg.new_packet)(self.packet, length), "av_new_packet")?;
            let data = read_ptr(self.packet, layout::PACKET_DATA) as *mut u8;
            if data.is_null() && length > 0 {
                (ffmpeg.packet_unref)(self.packet);
                return Err(io::Error::other("av_new_packet gave no data"));
            }
            if length > 0 {
                std::ptr::copy_nonoverlapping(record.payload.as_ptr(), data, record.payload.len());
            }
            let base = self.packet as *mut u8;
            std::ptr::write_unaligned(base.add(layout::PACKET_PTS) as *mut i64, token);
            let flags = if record.is_keyframe() {
                plan::AV_PKT_FLAG_KEY
            } else {
                0
            };
            std::ptr::write_unaligned(base.add(layout::PACKET_FLAGS) as *mut c_int, flags);
            let code = (ffmpeg.send_packet)(self.context, self.packet);
            (ffmpeg.packet_unref)(self.packet);
            Ok(code)
        }
    }

    /// Takes every frame ready now; whether a picture came.
    fn drain(&mut self) -> io::Result<bool> {
        let mut shown = false;
        loop {
            // SAFETY: the opened context and the decoder's own frame.
            let code = unsafe { (self.ffmpeg.receive_frame)(self.context, self.frame) };
            if code == plan::AVERROR_EAGAIN || code == plan::AVERROR_EOF {
                return Ok(shown);
            }
            self.ffmpeg.check(code, "avcodec_receive_frame")?;
            // SAFETY: a decoded frame; pts is a field of the pinned layout.
            let pts = unsafe {
                std::ptr::read_unaligned(
                    (self.frame as *const u8).add(layout::FRAME_PTS) as *const i64
                )
            };
            let result = u64::try_from(pts)
                .map_err(|_| DecoderFallback::OutputIdentityUnavailable.error())
                .and_then(|token| self.identities.take(token))
                .and_then(|identity| self.take_output().map(|()| identity));
            // SAFETY: the frame's references go; a kept picture holds its own.
            unsafe { (self.ffmpeg.frame_unref)(self.frame) };
            self.decoded = Some(result?);
            self.produced = true;
            shown = true;
        }
    }

    fn decode_record(&mut self, record: &FrameRecord) -> io::Result<bool> {
        crate::screen::core::frame_size_caps::check_video_dimensions(
            self.codec,
            &record.payload,
            self.size,
            self.limits,
        )?;
        let token = self.frames;
        let next = token
            .checked_add(1)
            .ok_or_else(|| io::Error::other("FFmpeg frame counter overflow"))?;
        let mut shown = false;
        let mut code = self.send(record, token)?;
        if code == plan::AVERROR_EAGAIN {
            shown = self.drain()?;
            code = self.send(record, token)?;
        }
        self.ffmpeg.check(code, "avcodec_send_packet")?;
        // Registered after acceptance: earlier output cannot claim this unit.
        self.identities
            .insert(token as u64, DecodedFrame::from_record(record))?;
        self.frames = next;
        Ok(self.drain()? || shown)
    }

    /// A hardware failure ends this decoder's stage of its codec for the
    /// process when it is the device's or comes before the first picture
    /// (`ffmpeg_codecs`, P9: never the hardware of the other codecs).
    fn on_error(&self) {
        let lost = self.device.as_ref().is_some_and(|device| device.lost());
        if self.stage.hardware() && (lost || !self.produced) {
            super::ffmpeg_codecs::give_up(self.codec, self.stage);
        }
    }
}

/// The pointer at `base + at`.
///
/// # Safety
/// `base + at` must be a readable pointer field.
unsafe fn read_ptr(base: Raw, at: usize) -> Raw {
    unsafe { std::ptr::read_unaligned((base as *const u8).add(at) as *const Raw) }
}

/// # Safety
/// `base + at` must be a writable pointer field.
unsafe fn write_ptr(base: Raw, at: usize, value: Raw) {
    unsafe { std::ptr::write_unaligned((base as *mut u8).add(at) as *mut Raw, value) }
}

impl ScreenDecoder for FfmpegDecoder {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        self.gpu_picture = None;
        let result = self.decode_record(record);
        if result.is_err() {
            self.on_error();
        }
        result
    }

    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.decoded
    }

    fn picture(&self) -> &BgraImage {
        &self.picture
    }

    fn prefer_yuv(&mut self, on: bool) {
        self.yuv = on;
        if !on {
            self.planes = None;
        }
    }

    fn yuv_picture(&self) -> Option<YuvPicture> {
        self.planes.clone().filter(|_| self.yuv)
    }

    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.planes.take().filter(|_| self.yuv)
    }

    fn take_picture(&mut self) -> BgraImage {
        match self.spare_bgra.take() {
            Some(next) if next.size() == self.picture.size() && next.regions().is_none() => {
                std::mem::replace(&mut self.picture, next)
            }
            _ => self.picture.clone(),
        }
    }

    fn recycle_yuv(&mut self, planes: YuvPicture) {
        self.spare = Some(planes);
    }

    fn recycle_picture(&mut self, image: BgraImage) {
        self.spare_bgra = Some(image);
    }

    /// B-36 as for Media Foundation (`mf_decoder_planes::keeps_references`):
    /// HEVC and AV1 name their references, H.264 asks for a key frame.
    fn keeps_references(&self) -> bool {
        self.codec != CodecKind::H264
    }

    fn prefer_gpu(&mut self, on: bool) {
        self.gpu = on && self.stage == FfmpegStage::D3d11va;
        if !on {
            self.gpu_picture = None;
        }
    }

    fn take_gpu_picture(&mut self) -> Option<GpuPicture> {
        self.gpu_picture.take()
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        // Pictures handed out keep their own references (`D3d11Frame`).
        self.gpu_picture = None;
        // SAFETY: each pointer was allocated by FFmpeg in `open` (or is null,
        // which the free functions accept) and is freed once.
        unsafe {
            (self.ffmpeg.frame_free)(&mut self.transfer);
            (self.ffmpeg.frame_free)(&mut self.frame);
            (self.ffmpeg.packet_free)(&mut self.packet);
            (self.ffmpeg.free_context)(&mut self.context);
        }
    }
}
