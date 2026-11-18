//! NVENC H.264 encoder for Linux hosts with an NVIDIA GPU (BK4, B-24),
//! through the CUDA device of the driver (`NV_ENC_DEVICE_TYPE_CUDA`, the
//! way FFmpeg's `nvenc.c` opens NVENC on Linux). Settings as B-21 and R01,
//! shared with Windows since merge W2 (`native_ffi/nvenc_settings.rs`):
//! preset P1 with tuning "ultra low latency", CBR, VBV buffer of one frame,
//! no B pictures, no look-ahead, infinite GOP with an IDR only on request,
//! SPS and PPS repeated at every IDR, BT.709 limited range in the VUI,
//! `lowDelayKeyFrameScale` 1. The input is the CPU's I420 written as NV12
//! into an NVENC input buffer (a DMA-BUF capture is not offered while NVENC
//! ranks first, `linux_hw_codecs::dmabuf_import_target`).
//!
//! B-36 (merge W2, as `windows/nvenc_encode.rs`): where the GPU can
//! invalidate references (`NV_ENC_CAPS_SUPPORT_REF_PIC_INVALIDATION` and
//! several reference frames) the session keeps a DPB of 5 pictures (8 for
//! AV1) with one reference each, stamps every picture with its frame id
//! (`inputTimeStamp`) and, after a stream reset, invalidates every picture
//! after the one the viewer confirmed (`NvEncInvalidateRefFrames`) instead
//! of encoding a key frame (`RefHistory`, Sunshine `nvenc_base.cpp:799-834`).
use std::io;
use std::sync::Arc;

use super::hw_encode_common::{HwH264Encoder, HwH264Session, HwInput, RateParams};
use super::nvenc_device::{
    destroy_session, invalidation, missing, open_session, status, CudaContext,
};
use super::nvenc_ffi::{
    NvApi, NvEncConfig, NvEncCreateBitstreamBuffer, NvEncCreateInputBuffer, NvEncInitializeParams,
    NvEncLockBitstream, NvEncLockInputBuffer, NvEncPicParams, NvEncPresetConfig,
    NvEncReconfigureParams, Ptr, Zeroed, NV_ENC_BUFFER_FORMAT_NV12, NV_ENC_CODEC_H264_GUID,
    NV_ENC_CONFIG_VER, NV_ENC_CREATE_BITSTREAM_BUFFER_VER, NV_ENC_CREATE_INPUT_BUFFER_VER,
    NV_ENC_INITIALIZE_PARAMS_VER, NV_ENC_LOCK_BITSTREAM_VER, NV_ENC_LOCK_INPUT_BUFFER_VER,
    NV_ENC_MEMORY_HEAP_SYSMEM_CACHED, NV_ENC_PIC_FLAG_FORCEIDR, NV_ENC_PIC_FLAG_OUTPUT_SPSPPS,
    NV_ENC_PIC_PARAMS_VER, NV_ENC_PIC_STRUCT_FRAME, NV_ENC_PIC_TYPE_IDR, NV_ENC_PRESET_CONFIG_VER,
    NV_ENC_PRESET_P1_GUID, NV_ENC_RECONFIGURE_PARAMS_VER, NV_ENC_SUCCESS,
    NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY,
};
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::codec_registry::EncoderFactory;
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::{codec_ceiling_bps, START_DIRECT_BPS};
use crate::screen::core::ref_recovery::{Recovery, RefHistory};
use crate::screen::core::wire::invalid;

// Merge W2: names and settings shared with Windows
// (`native_ffi/nvenc_settings.rs`); every `nvenc_encode::` path stays.
pub(super) use crate::screen::os::nvenc_shared::settings::{
    apply_codec_references, apply_codec_settings, NvRate, NVENC_NAME,
};
#[cfg(test)]
pub(super) use crate::screen::os::nvenc_shared::settings::{apply_settings, RFI_DPB};

impl NvRate for RateParams {
    fn cbr(&self) -> (u32, u32, u32) {
        (
            self.bits_per_second,
            self.buffer_bits,
            self.initial_fullness,
        )
    }
}

/// The rate of a session with its constant quality of „Scharf (GPU)“ (P9,
/// rule „Wahl gilt“ of 2026-11-18: Linux applies it as Windows does, VBR
/// with `targetQuality` under the rate as its peak, `nvenc_settings.rs`).
pub(super) struct SessionRate<'a>(pub(super) &'a RateParams, pub(super) Option<u8>);

impl NvRate for SessionRate<'_> {
    fn cbr(&self) -> (u32, u32, u32) {
        self.0.cbr()
    }

    fn quality(&self) -> Option<u8> {
        self.1
    }
}

/// The quality target an admitted session opens with (sharp mode, P9); the
/// run-time check and `bench --local` (no admission) measure CBR, as on
/// Windows (`vendor_codecs.rs`).
pub(super) fn admitted_quality(grant: Option<&CaptureGrant>) -> Option<u8> {
    grant.and_then(|_| crate::screen::core::rate_control::model::session_quality())
}

type Deliver<'a> = &'a mut dyn FnMut(&[&[u8]]) -> io::Result<()>;

/// The NVENC encoder of this system.
pub(super) struct NvencEncoderFactory {
    pub max: FrameSize,
}

impl NvencEncoderFactory {
    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        if choice.profile != Some(H264Profile::High) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "NVENC offers High only",
            ));
        }
        let fits = size.width <= self.max.width && size.height <= self.max.height;
        if size != size.even() || !fits || size.width < 16 || size.height < 16 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("NVENC cannot encode {}x{}", size.width, size.height),
            ));
        }
        let session = NvencSession::open(size, fps, admitted_quality(grant.as_ref()))?;
        let encoder = HwH264Encoder::new(session, NVENC_NAME.into(), size, fps, grant);
        // Merge W2: the bit rate up to the pairing's level, no fixed ceiling.
        Ok(Box::new(
            encoder.with_ceiling(codec_ceiling_bps(choice, size, fps)),
        ))
    }
}

impl EncoderFactory for NvencEncoderFactory {
    fn name(&self) -> String {
        NVENC_NAME.into()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::h264(H264Profile::High, true).with_max(self.max)]
    }

    fn open(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.create(choice, size, fps, None)
    }

    fn open_admitted(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.create(choice, size, fps, Some(grant))
    }
}

/// One NVENC session of a fixed size.
pub(super) struct NvencSession {
    api: Arc<NvApi>,
    encoder: Ptr,
    input: Ptr,
    output: Ptr,
    size: FrameSize,
    init: Box<NvEncInitializeParams>,
    config: Box<NvEncConfig>,
    rate: RateParams,
    /// The constant quality of sharp mode (P9); `None`: CBR.
    quality: Option<u8>,
    /// A rate the driver refused (plan 2026-11-17): the session keeps the
    /// running one and does not ask for this one again.
    refused: Option<RateParams>,
    frames: u64,
    codec: CodecKind,
    /// B-36: the GPU can invalidate references (merge W2).
    invalidation: bool,
    history: RefHistory,
    /// Frame id of the next picture (`set_frame_id`), its `inputTimeStamp`.
    frame_id: u64,
    // Dropped last: the session lives on this context.
    context: CudaContext,
}

// SAFETY: the session's pointers are NVENC/CUDA handles used by one owner
// at a time; every call pushes the CUDA context on the calling thread.
unsafe impl Send for NvencSession {}

impl NvencSession {
    fn open(size: FrameSize, fps: u32, quality: Option<u8>) -> io::Result<Self> {
        Self::open_codec(size, fps, CodecKind::H264, quality)
    }

    /// A session of `codec` (H.264, HEVC or AV1, BK6) with the constant
    /// `quality` of sharp mode (`None`: CBR).
    pub(super) fn open_codec(
        size: FrameSize,
        fps: u32,
        codec: CodecKind,
        quality: Option<u8>,
    ) -> io::Result<Self> {
        let guid = super::nvenc_codecs::encode_guid(codec)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "NVENC has no tiles"))?;
        let api = super::nvenc_ffi::load().map_err(io::Error::other)?;
        let context = CudaContext::new(api.clone())?;
        let (encoder, invalidation) = {
            let _current = context.push();
            let encoder = open_session(&api, context.context)?;
            let invalidation = invalidation(&api, encoder, guid);
            (encoder, invalidation)
        };
        let mut session = Self {
            api: api.clone(),
            encoder,
            input: std::ptr::null_mut(),
            output: std::ptr::null_mut(),
            size,
            init: Box::new(NvEncInitializeParams::zeroed()),
            config: Box::new(NvEncConfig::zeroed()),
            rate: RateParams::for_target(START_DIRECT_BPS, fps),
            quality,
            refused: None,
            frames: 0,
            codec,
            invalidation,
            history: RefHistory::new(if invalidation {
                crate::screen::os::nvenc_shared::codecs::rfi_dpb(codec)
            } else {
                1
            }),
            frame_id: 0,
            context,
        };
        let _current = session.context.push();
        let f = &api.functions;
        let mut preset = Box::new(NvEncPresetConfig::zeroed());
        preset.version = NV_ENC_PRESET_CONFIG_VER;
        preset.preset_cfg.version = NV_ENC_CONFIG_VER;
        let get = f.get_encode_preset_config_ex.ok_or_else(missing)?;
        // SAFETY: valid session; versioned, zeroed preset for the driver.
        let result = unsafe {
            get(
                encoder,
                guid,
                NV_ENC_PRESET_P1_GUID,
                NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY,
                &mut *preset,
            )
        };
        status(&api, encoder, result, "nvEncGetEncodePresetConfigEx")?;
        *session.config = preset.preset_cfg;
        let rate = SessionRate(&session.rate, session.quality);
        apply_codec_settings(&mut session.config, &rate, codec);
        apply_codec_references(&mut session.config, codec, invalidation);
        session.fill_init();
        let initialize = f.initialize_encoder.ok_or_else(missing)?;
        // SAFETY: versioned parameters whose config pointer is `session.config`.
        let result = unsafe { initialize(encoder, &mut *session.init) };
        status(&api, encoder, result, "nvEncInitializeEncoder")?;
        let mut input = NvEncCreateInputBuffer::zeroed();
        input.version = NV_ENC_CREATE_INPUT_BUFFER_VER;
        input.width = size.width;
        input.height = size.height;
        input.memory_heap = NV_ENC_MEMORY_HEAP_SYSMEM_CACHED;
        input.buffer_fmt = NV_ENC_BUFFER_FORMAT_NV12;
        let create = f.create_input_buffer.ok_or_else(missing)?;
        // SAFETY: valid session and versioned parameters.
        let result = unsafe { create(encoder, &mut input) };
        status(&api, encoder, result, "nvEncCreateInputBuffer")?;
        session.input = input.input_buffer;
        let mut output = NvEncCreateBitstreamBuffer::zeroed();
        output.version = NV_ENC_CREATE_BITSTREAM_BUFFER_VER;
        let create = f.create_bitstream_buffer.ok_or_else(missing)?;
        // SAFETY: valid session and versioned parameters.
        let result = unsafe { create(encoder, &mut output) };
        status(&api, encoder, result, "nvEncCreateBitstreamBuffer")?;
        session.output = output.bitstream_buffer;
        Ok(session)
    }

    fn fill_init(&mut self) {
        let init = &mut *self.init;
        init.version = NV_ENC_INITIALIZE_PARAMS_VER;
        init.encode_guid =
            super::nvenc_codecs::encode_guid(self.codec).unwrap_or(NV_ENC_CODEC_H264_GUID);
        init.preset_guid = NV_ENC_PRESET_P1_GUID;
        init.encode_width = self.size.width;
        init.encode_height = self.size.height;
        init.dar_width = self.size.width;
        init.dar_height = self.size.height;
        init.frame_rate_num = self.rate.fps;
        init.frame_rate_den = 1;
        init.enable_ptd = 1;
        init.encode_config = &mut *self.config;
        init.max_encode_width = self.size.width;
        init.max_encode_height = self.size.height;
        init.tuning_info = NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY;
        init.buffer_format = NV_ENC_BUFFER_FORMAT_NV12;
    }

    /// A new rate without a new IDR (`nvEncReconfigureEncoder`). A refused
    /// rate keeps the running one (plan 2026-11-17): the configuration goes
    /// back to it and the error only reports the refusal.
    fn retune(&mut self, rate: &RateParams) -> io::Result<()> {
        let reconfigure = self.api.functions.reconfigure_encoder.ok_or_else(missing)?;
        let previous = self.rate;
        self.rate = *rate;
        apply_codec_settings(
            &mut self.config,
            &SessionRate(rate, self.quality),
            self.codec,
        );
        apply_codec_references(&mut self.config, self.codec, self.invalidation);
        self.fill_init();
        let mut params = Box::new(NvEncReconfigureParams::zeroed());
        params.version = NV_ENC_RECONFIGURE_PARAMS_VER;
        params.re_init_encode_params = *self.init;
        // SAFETY: valid session; the config pointer inside stays valid.
        let result = unsafe { reconfigure(self.encoder, &mut *params) };
        let checked = status(&self.api, self.encoder, result, "nvEncReconfigureEncoder");
        if checked.is_err() {
            self.rate = previous;
            let restored = SessionRate(&previous, self.quality);
            apply_codec_settings(&mut self.config, &restored, self.codec);
            apply_codec_references(&mut self.config, self.codec, self.invalidation);
            self.fill_init();
            self.refused = Some(*rate);
        } else {
            self.refused = None;
        }
        checked
    }

    /// Writes I420 planes as NV12 into the input buffer.
    fn upload(&mut self, yuv: &crate::screen::core::pixel_convert::I420Image) -> io::Result<u32> {
        let f = &self.api.functions;
        let mut lock = NvEncLockInputBuffer::zeroed();
        lock.version = NV_ENC_LOCK_INPUT_BUFFER_VER;
        lock.input_buffer = self.input;
        let lock_input = f.lock_input_buffer.ok_or_else(missing)?;
        // SAFETY: valid session and input buffer.
        let result = unsafe { lock_input(self.encoder, &mut lock) };
        status(&self.api, self.encoder, result, "nvEncLockInputBuffer")?;
        let pitch = lock.pitch as usize;
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        let chroma = self.size.chroma();
        let (chroma_width, rows) = (chroma.width as usize, chroma.height as usize);
        let fits = pitch >= chroma_width * 2
            && pitch >= width
            && yuv.y.len() >= width * height
            && yuv.u.len() >= chroma_width * rows
            && yuv.v.len() >= chroma_width * rows;
        if fits && !lock.buffer_data_ptr.is_null() {
            // SAFETY: the locked NV12 buffer of `pitch` × (height × 3 / 2)
            // bytes (luma rows, then the interleaved chroma rows at
            // `pitch × height`, the NVENC NV12 layout) is ours until unlocked.
            let target = unsafe {
                std::slice::from_raw_parts_mut(
                    lock.buffer_data_ptr.cast::<u8>(),
                    pitch * (height + rows),
                )
            };
            for row in 0..height {
                target[row * pitch..][..width].copy_from_slice(&yuv.y[row * width..][..width]);
            }
            for row in 0..rows {
                let line = &mut target[(height + row) * pitch..][..chroma_width * 2];
                let u = &yuv.u[row * chroma_width..][..chroma_width];
                let v = &yuv.v[row * chroma_width..][..chroma_width];
                for (pair, (u, v)) in line.chunks_exact_mut(2).zip(u.iter().zip(v)) {
                    pair[0] = *u;
                    pair[1] = *v;
                }
            }
        }
        let unlock = f.unlock_input_buffer.ok_or_else(missing)?;
        // SAFETY: the buffer was locked above.
        let result = unsafe { unlock(self.encoder, self.input) };
        status(&self.api, self.encoder, result, "nvEncUnlockInputBuffer")?;
        if !fits || lock.buffer_data_ptr.is_null() {
            return Err(invalid("NVENC input buffer smaller than the picture"));
        }
        Ok(lock.pitch)
    }
}

impl HwH264Session for NvencSession {
    fn wants_bgra(&self) -> bool {
        false
    }

    fn encode(
        &mut self,
        input: HwInput<'_>,
        idr: bool,
        rate: &RateParams,
        deliver: Deliver<'_>,
    ) -> io::Result<()> {
        let HwInput::I420(yuv) = input else {
            return Err(invalid("NVENC takes I420"));
        };
        let api = self.api.clone();
        let _current = self.context.push();
        if self.rate.differs(rate) && self.refused != Some(*rate) {
            if let Err(error) = self.retune(rate) {
                eprintln!(
                    "screen encoder nvenc: rate {} bit/s refused, keeps {} bit/s: {error}",
                    rate.bits_per_second, self.rate.bits_per_second
                );
            }
        }
        let pitch = self.upload(yuv)?;
        let f = &api.functions;
        let mut picture = Box::new(NvEncPicParams::zeroed());
        picture.version = NV_ENC_PIC_PARAMS_VER;
        picture.input_width = self.size.width;
        picture.input_height = self.size.height;
        picture.input_pitch = pitch;
        picture.frame_idx = self.frames as u32;
        // B-36: the frame id names the picture to `NvEncInvalidateRefFrames`.
        picture.input_time_stamp = self.frame_id;
        picture.input_buffer = self.input;
        picture.output_bitstream = self.output;
        picture.buffer_fmt = NV_ENC_BUFFER_FORMAT_NV12;
        picture.picture_struct = NV_ENC_PIC_STRUCT_FRAME;
        if idr {
            picture.encode_pic_flags = NV_ENC_PIC_FLAG_FORCEIDR | NV_ENC_PIC_FLAG_OUTPUT_SPSPPS;
            picture.picture_type = NV_ENC_PIC_TYPE_IDR;
        }
        let encode = f.encode_picture.ok_or_else(missing)?;
        // SAFETY: valid session, input and output buffers.
        let result = unsafe { encode(self.encoder, &mut *picture) };
        status(&api, self.encoder, result, "nvEncEncodePicture")?;
        self.frames += 1;
        let mut lock = Box::new(NvEncLockBitstream::zeroed());
        lock.version = NV_ENC_LOCK_BITSTREAM_VER;
        lock.output_bitstream = self.output;
        let lock_bitstream = f.lock_bitstream.ok_or_else(missing)?;
        // SAFETY: the picture was submitted synchronously; blocking lock.
        let result = unsafe { lock_bitstream(self.encoder, &mut *lock) };
        status(&api, self.encoder, result, "nvEncLockBitstream")?;
        let idr_coded = lock.picture_type == NV_ENC_PIC_TYPE_IDR;
        let delivered = if lock.bitstream_buffer_ptr.is_null() {
            Err(invalid("NVENC gave no bitstream"))
        } else {
            let length = lock.bitstream_size_in_bytes as usize;
            // SAFETY: the locked bitstream holds `length` bytes until unlocked.
            let bytes: &[u8] =
                unsafe { std::slice::from_raw_parts(lock.bitstream_buffer_ptr.cast(), length) };
            deliver(&[bytes])
        };
        let unlock = f.unlock_bitstream.ok_or_else(missing)?;
        // SAFETY: the bitstream was locked above.
        let result = unsafe { unlock(self.encoder, self.output) };
        delivered?;
        status(&api, self.encoder, result, "nvEncUnlockBitstream")?;
        self.history.encoded(self.frame_id, idr_coded || idr);
        Ok(())
    }

    fn set_frame_id(&mut self, frame_id: u64) {
        self.frame_id = frame_id;
    }

    /// B-36 as Windows' NVENC session: every picture after `last_good` is
    /// invalidated, the next one references `last_good` or older.
    fn recover_references(&mut self, last_good: u64) -> Recovery {
        let invalidate = self.api.functions.invalidate_ref_frames;
        let (Some(invalidate), true) = (invalidate, self.invalidation) else {
            return Recovery::KeyframeNeeded;
        };
        let Some(lost) = self.history.invalidate_after(last_good) else {
            return Recovery::KeyframeNeeded;
        };
        let _current = self.context.push();
        for frame in lost {
            // SAFETY: valid session; `frame` is the timestamp of a picture
            // it encoded (`input_time_stamp`).
            if unsafe { invalidate(self.encoder, frame) } != NV_ENC_SUCCESS {
                return Recovery::KeyframeNeeded;
            }
        }
        Recovery::Continued
    }
}

impl Drop for NvencSession {
    fn drop(&mut self) {
        let _current = self.context.push();
        let f = &self.api.functions;
        // SAFETY: buffers and session were created here and are idle.
        unsafe {
            if let (Some(destroy), false) = (f.destroy_input_buffer, self.input.is_null()) {
                destroy(self.encoder, self.input);
            }
            if let (Some(destroy), false) = (f.destroy_bitstream_buffer, self.output.is_null()) {
                destroy(self.encoder, self.output);
            }
        }
        destroy_session(&self.api, self.encoder);
    }
}
