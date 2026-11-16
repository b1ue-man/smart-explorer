//! The CUDA device and NVENC sessions of the Linux NVENC encoder (BK4,
//! B-24): a CUDA context on device 0 (`cuCtxCreate_v2`, made current
//! around every NVENC call as FFmpeg's `nvenc.c` does), opening and closing
//! an NVENC session (`nvEncOpenEncodeSessionEx` with
//! `NV_ENC_DEVICE_TYPE_CUDA`) and the probe of the largest picture per
//! codec. The driver's error text, closing a session and the capability
//! queries (`NV_ENC_CAPS_WIDTH_MAX`, `NV_ENC_CAPS_HEIGHT_MAX`, reference
//! invalidation) are shared with Windows since merge W2
//! (`native_ffi/nvenc_api.rs`, re-exported below). Sources as in
//! `nvenc_ffi.rs`.
use std::io;
use std::sync::{Arc, OnceLock};

use super::nvenc_ffi::{
    NvApi, NvEncOpenSessionParams, Ptr, Zeroed, NVENCAPI_VERSION, NV_ENC_CODEC_H264_GUID,
    NV_ENC_DEVICE_TYPE_CUDA, NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER,
};
// Merge W2: the calls without a device are shared with Windows.
use crate::screen::core::pixel_convert::FrameSize;
pub(super) use crate::screen::os::nvenc_shared::api::{
    destroy_session, invalidation, largest_picture, missing, status,
};

/// A CUDA context on device 0, destroyed when dropped.
pub(super) struct CudaContext {
    pub api: Arc<NvApi>,
    pub context: Ptr,
}

impl CudaContext {
    pub(super) fn new(api: Arc<NvApi>) -> io::Result<Self> {
        let (mut count, mut device, mut context) = (0, 0, std::ptr::null_mut());
        // SAFETY: CUDA is initialized (`nvenc_ffi::load`); writable slots.
        let ok = unsafe {
            (api.cu_device_get_count)(&mut count) == 0
                && count > 0
                && (api.cu_device_get)(&mut device, 0) == 0
                && (api.cu_ctx_create)(&mut context, 0, device) == 0
        };
        if !ok || context.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no CUDA device for NVENC",
            ));
        }
        // SAFETY: `cuCtxCreate` made the context current; it is popped so
        // every later call pushes it explicitly (`Pushed`).
        unsafe { (api.cu_ctx_pop)(&mut std::ptr::null_mut()) };
        Ok(Self { api, context })
    }

    /// Makes the context current until the guard is dropped.
    pub(super) fn push(&self) -> Pushed {
        // SAFETY: a valid context of this process.
        unsafe { (self.api.cu_ctx_push)(self.context) };
        Pushed(self.api.clone())
    }
}

impl Drop for CudaContext {
    fn drop(&mut self) {
        // SAFETY: the context was created here and every session on it is gone.
        unsafe { (self.api.cu_ctx_destroy)(self.context) };
    }
}

pub(super) struct Pushed(Arc<NvApi>);

impl Drop for Pushed {
    fn drop(&mut self) {
        // SAFETY: pops the context pushed by `CudaContext::push`.
        unsafe { (self.0.cu_ctx_pop)(&mut std::ptr::null_mut()) };
    }
}

/// Opens an NVENC session on `context` (pushed by the caller).
pub(super) fn open_session(api: &NvApi, context: Ptr) -> io::Result<Ptr> {
    let open = api.functions.open_encode_session_ex.ok_or_else(missing)?;
    let mut params = NvEncOpenSessionParams::zeroed();
    params.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
    params.device_type = NV_ENC_DEVICE_TYPE_CUDA;
    params.device = context;
    params.api_version = NVENCAPI_VERSION;
    let mut encoder = std::ptr::null_mut();
    // SAFETY: versioned parameters and a writable session slot.
    let result = unsafe { open(&mut params, &mut encoder) };
    if let Err(err) = status(
        api,
        std::ptr::null_mut(),
        result,
        "nvEncOpenEncodeSessionEx",
    ) {
        destroy_session(api, encoder);
        return Err(err);
    }
    Ok(encoder)
}

/// The largest picture of NVENC H.264 on this system; `None` without an
/// NVIDIA driver, CUDA device or free session. Probed once per process.
pub(super) fn probe() -> Option<FrameSize> {
    probe_all()[0]
}

/// The largest picture of NVENC `codec` (BK6: HEVC, AV1); a codec the GPU
/// lacks answers `nvEncGetEncodeCaps` with an error and stays `None`.
pub(super) fn probe_codec(codec: crate::screen::core::negotiate::CodecKind) -> Option<FrameSize> {
    use crate::screen::core::negotiate::CodecKind;
    match codec {
        CodecKind::H264 => probe_all()[0],
        CodecKind::Hevc => probe_all()[1],
        CodecKind::Av1 => probe_all()[2],
        CodecKind::Tiles => None,
    }
}

/// H.264, HEVC and AV1 sizes of one session, once per process.
fn probe_all() -> [Option<FrameSize>; 3] {
    static PROBE: OnceLock<[Option<FrameSize>; 3]> = OnceLock::new();
    *PROBE.get_or_init(|| {
        let Some(api) = super::nvenc_ffi::load().ok() else {
            return [None; 3];
        };
        let Some(context) = CudaContext::new(api.clone()).ok() else {
            return [None; 3];
        };
        let _current = context.push();
        let Some(encoder) = open_session(&api, context.context).ok() else {
            return [None; 3];
        };
        let size = |guid| largest_picture(&api, encoder, guid);
        let found = [
            size(NV_ENC_CODEC_H264_GUID),
            size(super::nvenc_codecs::NV_ENC_CODEC_HEVC_GUID),
            size(super::nvenc_codecs::NV_ENC_CODEC_AV1_GUID),
        ];
        destroy_session(&api, encoder);
        found
    })
}
