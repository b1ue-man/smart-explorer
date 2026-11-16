//! NVENC session of Windows hosts with an NVIDIA GPU (BK12, B-22; HEVC and
//! AV1 by merge W1 through `nvenc_codecs.rs`, as BK6's Linux session does
//! with `apply_codec_settings`) on the capture's D3D11 device. Settings as
//! B-21 and R01
//! (`nvenc_base.cpp:258-283`, `:351-381`, `:641-643`), identical to BK4's
//! `linux_os/nvenc_encode.rs` (`apply_settings`): preset P1 with tuning
//! "ultra low latency", CBR, VBV buffer of one frame, no B pictures
//! (`frameIntervalP` 1, `zeroReorderDelay`), no look-ahead, infinite GOP with
//! an IDR only on request, SPS and PPS at every IDR, BT.709 limited range in
//! the VUI. In addition (`apply_reference_settings`, since merge W2 in the
//! module both platforms share, `native_ffi/nvenc_settings.rs`):
//! `lowDelayKeyFrameScale` 1 (Sunshine `:263`) and for reference
//! invalidation a DPB of 5 pictures with one reference per picture
//! (Sunshine `:291-298`, `:372`).
//!
//! Input: the NV12 texture of BK2a's ring (or the encoder-size texture),
//! registered once per texture and mapped per picture
//! (`NvEncMapInputResource`, Sunshine `:728-739`). Sunshine also hands NVENC
//! NV12 converted by its own shader, not BGRA: one BT.709 conversion for
//! every encoder of the rank, matching the VUI. Asynchronous mode (W4, B1;
//! Sunshine `nvenc_d3d11.cpp:16,50`, `nvenc_base.cpp:219,492-499,685-709,
//! 750-769`): one Win32 auto-reset completion event registered with
//! `nvEncRegisterAsyncEvent`, waited on for at most four frame intervals (at
//! least 100 ms) before `nvEncLockBitstream` with `doNotWait = 1`, so a hung
//! driver fails this encoder and hands over to the next rank stage instead of
//! stalling until the 10 s frame timeout.
use std::io;
use std::sync::Arc;

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::nvenc_codecs::{encode_guid, rfi_dpb};
use super::nvenc_device::{
    async_encode, destroy_session, evictions, invalidation, missing, open_session, register,
    status, unregister, Registered,
};
use super::nvenc_ffi::{
    NvApi, NvEncConfig, NvEncCreateBitstreamBuffer, NvEncEventParams, NvEncInitializeParams,
    NvEncLockBitstream, NvEncMapInputResource, NvEncPicParams, NvEncPresetConfig,
    NvEncReconfigureParams, NvTable, Ptr, Zeroed, LOCK_DO_NOT_WAIT, NV_ENC_BUFFER_FORMAT_NV12,
    NV_ENC_CODEC_H264_GUID, NV_ENC_CONFIG_VER, NV_ENC_CREATE_BITSTREAM_BUFFER_VER,
    NV_ENC_EVENT_PARAMS_VER, NV_ENC_INITIALIZE_PARAMS_VER, NV_ENC_LOCK_BITSTREAM_VER,
    NV_ENC_MAP_INPUT_RESOURCE_VER, NV_ENC_PIC_FLAG_FORCEIDR, NV_ENC_PIC_FLAG_OUTPUT_SPSPPS,
    NV_ENC_PIC_PARAMS_VER, NV_ENC_PIC_STRUCT_FRAME, NV_ENC_PIC_TYPE_IDR, NV_ENC_PRESET_CONFIG_VER,
    NV_ENC_PRESET_P1_GUID, NV_ENC_RECONFIGURE_PARAMS_VER, NV_ENC_SUCCESS,
    NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY,
};
use super::vendor_encode::{RateParams, Recovery, RefHistory, VendorPicture, VendorSession};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::START_DIRECT_BPS;
use crate::screen::core::wire::invalid;

// Merge W2: names and settings shared with Linux
// (`native_ffi/nvenc_settings.rs`); every `nvenc_encode::` path stays.
pub(super) use crate::screen::os::nvenc_shared::settings::{
    apply_codec_references, apply_codec_settings, NvRate,
};
#[cfg(test)]
pub(super) use crate::screen::os::nvenc_shared::settings::{
    apply_reference_settings, apply_settings, RFI_DPB,
};

/// Shortest completion wait for one asynchronous NVENC picture (W4, B1).
pub(super) const MIN_ASYNC_TIMEOUT_MS: u32 = 100;

/// Completion deadline in milliseconds: four frame intervals, at least 100 ms.
pub(super) fn async_timeout_ms(fps: u32) -> u32 {
    4_000u64
        .div_ceil(u64::from(fps.max(1)))
        .max(u64::from(MIN_ASYNC_TIMEOUT_MS)) as u32
}

fn create_completion_event() -> io::Result<HANDLE> {
    // SAFETY: unnamed auto-reset unsignaled kernel event (`nvenc_d3d11.cpp:16`).
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
    if event.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(event)
    }
}

impl NvRate for RateParams {
    fn cbr(&self) -> (u32, u32, u32) {
        (
            self.bits_per_second,
            self.buffer_bits,
            self.initial_fullness,
        )
    }
}

/// Device-independent asynchronous NVENC session over an `NvTable`.
pub(super) struct Core<A: NvTable> {
    pub(super) api: A,
    pub(super) encoder: Ptr,
    output: Ptr,
    event: HANDLE,
    event_registered: bool,
    size: FrameSize,
    codec: CodecKind,
    init: Box<NvEncInitializeParams>,
    config: Box<NvEncConfig>,
    pub(super) rate: RateParams,
    invalidation: bool,
    pub(super) history: RefHistory,
    frames: u64,
}

impl<A: NvTable> Core<A> {
    pub(super) fn open(
        api: A,
        encoder: Ptr,
        codec: CodecKind,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Self> {
        let guid = encode_guid(codec)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "NVENC has no tiles"))?;
        let event = match create_completion_event() {
            Ok(event) => event,
            Err(error) => {
                destroy_session(&api, encoder);
                return Err(error);
            }
        };
        let invalidation = invalidation(&api, encoder, guid);
        let mut core = Self {
            api,
            encoder,
            output: std::ptr::null_mut(),
            event,
            event_registered: false,
            size,
            codec,
            init: Box::new(NvEncInitializeParams::zeroed()),
            config: Box::new(NvEncConfig::zeroed()),
            rate: RateParams::for_target(START_DIRECT_BPS, fps),
            invalidation,
            history: RefHistory::new(if invalidation { rfi_dpb(codec) } else { 1 }),
            frames: 0,
        };
        if !async_encode(&core.api, encoder, guid) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "NVENC driver lacks asynchronous encode support",
            ));
        }
        let mut preset = Box::new(NvEncPresetConfig::zeroed());
        preset.version = NV_ENC_PRESET_CONFIG_VER;
        preset.preset_cfg.version = NV_ENC_CONFIG_VER;
        let get = core
            .api
            .table()
            .get_encode_preset_config_ex
            .ok_or_else(missing)?;
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
        status(&core.api, encoder, result, "nvEncGetEncodePresetConfigEx")?;
        *core.config = preset.preset_cfg;
        apply_codec_settings(&mut core.config, &core.rate, codec);
        apply_codec_references(&mut core.config, codec, invalidation);
        core.fill_init();
        let initialize = core.api.table().initialize_encoder.ok_or_else(missing)?;
        // SAFETY: versioned parameters whose config pointer is `core.config`.
        let result = unsafe { initialize(encoder, &mut *core.init) };
        status(&core.api, encoder, result, "nvEncInitializeEncoder")?;
        let register_event = core.api.table().register_async_event.ok_or_else(missing)?;
        let mut event_params = Box::new(NvEncEventParams::zeroed());
        event_params.version = NV_ENC_EVENT_PARAMS_VER;
        event_params.completion_event = core.event;
        // SAFETY: initialized async session and a live Win32 event handle.
        let result = unsafe { register_event(encoder, &mut *event_params) };
        status(&core.api, encoder, result, "nvEncRegisterAsyncEvent")?;
        core.event_registered = true;
        let mut output = NvEncCreateBitstreamBuffer::zeroed();
        output.version = NV_ENC_CREATE_BITSTREAM_BUFFER_VER;
        let create = core
            .api
            .table()
            .create_bitstream_buffer
            .ok_or_else(missing)?;
        // SAFETY: valid session and versioned parameters.
        let result = unsafe { create(encoder, &mut output) };
        status(&core.api, encoder, result, "nvEncCreateBitstreamBuffer")?;
        core.output = output.bitstream_buffer;
        Ok(core)
    }

    fn fill_init(&mut self) {
        let init = &mut *self.init;
        init.version = NV_ENC_INITIALIZE_PARAMS_VER;
        init.encode_guid = encode_guid(self.codec).unwrap_or(NV_ENC_CODEC_H264_GUID);
        init.preset_guid = NV_ENC_PRESET_P1_GUID;
        init.encode_width = self.size.width;
        init.encode_height = self.size.height;
        init.dar_width = self.size.width;
        init.dar_height = self.size.height;
        init.frame_rate_num = self.rate.fps;
        init.frame_rate_den = 1;
        init.enable_encode_async = 1;
        init.enable_ptd = 1;
        init.encode_config = &mut *self.config;
        init.max_encode_width = self.size.width;
        init.max_encode_height = self.size.height;
        init.tuning_info = NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY;
        init.buffer_format = NV_ENC_BUFFER_FORMAT_NV12;
    }

    /// A new bit rate or frame rate without a new IDR
    /// (`nvEncReconfigureEncoder` without `resetEncoder`/`forceIDR`).
    pub(super) fn retune(&mut self, rate: &RateParams) -> io::Result<()> {
        self.rate = *rate;
        apply_codec_settings(&mut self.config, rate, self.codec);
        apply_codec_references(&mut self.config, self.codec, self.invalidation);
        self.fill_init();
        let mut params = Box::new(NvEncReconfigureParams::zeroed());
        params.version = NV_ENC_RECONFIGURE_PARAMS_VER;
        params.re_init_encode_params = *self.init;
        let reconfigure = self.api.table().reconfigure_encoder.ok_or_else(missing)?;
        // SAFETY: valid session; the config pointer inside stays valid.
        let result = unsafe { reconfigure(self.encoder, &mut *params) };
        status(&self.api, self.encoder, result, "nvEncReconfigureEncoder")
    }

    /// Encodes the mapped input, waits on the completion event up to the
    /// frame-rate deadline, and delivers the access unit without blocking in
    /// `nvEncLockBitstream`.
    pub(super) fn submit(
        &mut self,
        mapped: &NvEncMapInputResource,
        frame_id: u64,
        idr: bool,
        deliver: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<bool> {
        let f = self.api.table();
        let mut params = Box::new(NvEncPicParams::zeroed());
        params.version = NV_ENC_PIC_PARAMS_VER;
        params.input_width = self.size.width;
        params.input_height = self.size.height;
        params.frame_idx = self.frames as u32;
        params.input_time_stamp = frame_id;
        params.input_buffer = mapped.mapped_resource;
        params.output_bitstream = self.output;
        params.completion_event = self.event;
        params.buffer_fmt = mapped.mapped_buffer_fmt;
        params.picture_struct = NV_ENC_PIC_STRUCT_FRAME;
        if idr {
            params.encode_pic_flags = NV_ENC_PIC_FLAG_FORCEIDR | NV_ENC_PIC_FLAG_OUTPUT_SPSPPS;
            params.picture_type = NV_ENC_PIC_TYPE_IDR;
        }
        let encode = f.encode_picture.ok_or_else(missing)?;
        // SAFETY: valid async session, mapped input, output buffer and event.
        let result = unsafe { encode(self.encoder, &mut *params) };
        status(&self.api, self.encoder, result, "nvEncEncodePicture")?;
        self.frames += 1;
        let timeout_ms = async_timeout_ms(self.rate.fps);
        // SAFETY: `self.event` is a live kernel event owned by this session.
        let waited = unsafe { WaitForSingleObject(self.event, timeout_ms) };
        if waited != WAIT_OBJECT_0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("NVENC completion event timed out after {timeout_ms} ms (wait {waited})"),
            ));
        }
        let f = self.api.table();
        let mut lock = Box::new(NvEncLockBitstream::zeroed());
        lock.version = NV_ENC_LOCK_BITSTREAM_VER;
        lock.flags = LOCK_DO_NOT_WAIT;
        lock.output_bitstream = self.output;
        let lock_bitstream = f.lock_bitstream.ok_or_else(missing)?;
        // SAFETY: the completion event fired and `doNotWait` forbids blocking.
        let result = unsafe { lock_bitstream(self.encoder, &mut *lock) };
        status(&self.api, self.encoder, result, "nvEncLockBitstream")?;
        let delivered = if lock.bitstream_buffer_ptr.is_null() {
            Err(invalid("NVENC gave no bitstream"))
        } else {
            let length = lock.bitstream_size_in_bytes as usize;
            // SAFETY: the locked bitstream holds `length` bytes until unlocked.
            let bytes: &[u8] =
                unsafe { std::slice::from_raw_parts(lock.bitstream_buffer_ptr.cast(), length) };
            deliver(bytes)
        };
        let unlock = self.api.table().unlock_bitstream.ok_or_else(missing)?;
        // SAFETY: the bitstream was locked above.
        let result = unsafe { unlock(self.encoder, self.output) };
        delivered?;
        status(&self.api, self.encoder, result, "nvEncUnlockBitstream")?;
        Ok(lock.picture_type == NV_ENC_PIC_TYPE_IDR)
    }

    pub(super) fn recover_references(&mut self, last_good: u64) -> Recovery {
        let invalidate = self.api.table().invalidate_ref_frames;
        let (Some(invalidate), true) = (invalidate, self.invalidation) else {
            return Recovery::KeyframeNeeded;
        };
        let Some(lost) = self.history.invalidate_after(last_good) else {
            return Recovery::KeyframeNeeded;
        };
        for frame in lost {
            // SAFETY: valid session; `frame` is the timestamp of a picture
            // it encoded (`input_time_stamp`).
            if unsafe { invalidate(self.encoder, frame) } != NV_ENC_SUCCESS {
                return Recovery::KeyframeNeeded;
            }
        }
        Recovery::Continued
    }

    /// Destroys the bitstream buffer and unregisters the completion event
    /// before input resources and the session go (`nvenc_base.cpp:685-697`).
    fn release_outputs(&mut self) {
        if !self.output.is_null() {
            if let Some(destroy) = self.api.table().destroy_bitstream_buffer {
                // SAFETY: the buffer was created here.
                unsafe { destroy(self.encoder, self.output) };
            }
            self.output = std::ptr::null_mut();
        }
        if self.event_registered {
            if let Some(unregister_event) = self.api.table().unregister_async_event {
                let mut params = Box::new(NvEncEventParams::zeroed());
                params.version = NV_ENC_EVENT_PARAMS_VER;
                params.completion_event = self.event;
                // SAFETY: registered on this session; only cleared on success
                // so a handle still held by a wedged driver is never recycled.
                if unsafe { unregister_event(self.encoder, &mut *params) } == NV_ENC_SUCCESS {
                    self.event_registered = false;
                }
            }
        }
    }
}

impl<A: NvTable> Drop for Core<A> {
    fn drop(&mut self) {
        self.release_outputs();
        if !self.encoder.is_null() {
            destroy_session(&self.api, self.encoder);
            self.encoder = std::ptr::null_mut();
        }
        if !self.event.is_null() && !self.event_registered {
            // SAFETY: owned kernel event no longer registered with NVENC.
            unsafe { CloseHandle(self.event) };
            self.event = std::ptr::null_mut();
        }
    }
}

/// One NVENC session of a fixed size on the capture device.
pub(super) struct NvencSession {
    core: Core<Arc<NvApi>>,
    registered: Vec<Registered>,
    tick: u64,
    // Dropped last: the session lives on this device.
    _device: ID3D11Device,
}

impl NvencSession {
    pub(super) fn open(
        device: &ID3D11Device,
        codec: CodecKind,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Self> {
        let api = super::nvenc_ffi::load().map_err(io::Error::other)?;
        let encoder = open_session(&api, device)?;
        let core = Core::open(api, encoder, codec, size, fps)?;
        Ok(Self {
            core,
            registered: Vec::new(),
            tick: 0,
            _device: device.clone(),
        })
    }

    /// The registration of `texture`, made on first use.
    fn registration(&mut self, texture: &ID3D11Texture2D) -> io::Result<Ptr> {
        self.tick += 1;
        let tick = self.tick;
        let raw = texture.as_raw();
        if let Some(entry) = self
            .registered
            .iter_mut()
            .find(|entry| entry.texture.as_raw() == raw)
        {
            entry.used = tick;
            return Ok(entry.handle);
        }
        let used: Vec<u64> = self.registered.iter().map(|entry| entry.used).collect();
        for index in evictions(&used, tick).into_iter().rev() {
            let entry = self.registered.swap_remove(index);
            unregister(&self.core.api, self.core.encoder, entry);
        }
        let entry = register(
            &self.core.api,
            self.core.encoder,
            texture,
            self.core.size,
            tick,
        )?;
        let handle = entry.handle;
        self.registered.push(entry);
        Ok(handle)
    }
}

impl VendorSession for NvencSession {
    fn encode(
        &mut self,
        picture: VendorPicture<'_>,
        rate: &RateParams,
        deliver: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<bool> {
        if self.core.rate.differs(rate) {
            self.core.retune(rate)?;
        }
        let handle = self.registration(picture.texture)?;
        let mut mapped = NvEncMapInputResource::zeroed();
        mapped.version = NV_ENC_MAP_INPUT_RESOURCE_VER;
        mapped.registered_resource = handle;
        let map = self
            .core
            .api
            .functions
            .map_input_resource
            .ok_or_else(missing)?;
        // SAFETY: valid session and a registration of it.
        let result = unsafe { map(self.core.encoder, &mut mapped) };
        status(
            &self.core.api,
            self.core.encoder,
            result,
            "nvEncMapInputResource",
        )?;
        let coded = self
            .core
            .submit(&mapped, picture.frame_id, picture.idr, deliver);
        if let Some(unmap) = self.core.api.functions.unmap_input_resource {
            // SAFETY: the resource was mapped above; the picture is coded or timed out.
            unsafe { unmap(self.core.encoder, mapped.mapped_resource) };
        }
        let keyframe = coded?;
        self.core.history.encoded(picture.frame_id, keyframe);
        Ok(keyframe)
    }

    fn recover_references(&mut self, last_good: u64) -> Recovery {
        self.core.recover_references(last_good)
    }
}

impl Drop for NvencSession {
    fn drop(&mut self) {
        self.core.release_outputs();
        for entry in std::mem::take(&mut self.registered) {
            unregister(&self.core.api, self.core.encoder, entry);
        }
    }
}
