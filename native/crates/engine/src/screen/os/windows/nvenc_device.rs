//! The D3D11 device side of the Windows NVENC encoder (BK12, B-22; the
//! counterpart of BK4's CUDA `linux_os/nvenc_device.rs`): opening and
//! closing a session on a D3D11 device (`nvEncOpenEncodeSessionEx` with
//! `NV_ENC_DEVICE_TYPE_DIRECTX`, as Sunshine's `nvenc_d3d11_native.cpp:14-18`
//! and `nvenc_base.cpp:590-599`), the probe of the largest picture and the
//! registration of input textures (`NvEncRegisterResource` with
//! `NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX`, Sunshine
//! `nvenc_d3d11_native.cpp:53-68`). Sources as in `nvenc_ffi.rs`. The
//! driver's error text and the capability queries (largest picture,
//! reference invalidation) are shared with Linux since merge W2
//! (`native_ffi/nvenc_api.rs`, re-exported below).
use std::io;

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::CreateEventW;

use super::nvenc_codecs::encode_guid;
use super::nvenc_ffi::{
    NvApi, NvEncOpenSessionParams, NvEncRegisterResource, Ptr, Zeroed, NVENCAPI_VERSION,
    NV_ENC_BUFFER_FORMAT_NV12, NV_ENC_DEVICE_TYPE_DIRECTX, NV_ENC_INPUT_IMAGE,
    NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX, NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER,
    NV_ENC_REGISTER_RESOURCE_VER,
};
// Merge W2: the calls without a device are shared with Linux.
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;
pub(super) use crate::screen::os::nvenc_shared::api::{
    async_encode, destroy_session, invalidation, largest_picture, missing, status,
};

/// Shortest completion wait for one asynchronous NVENC picture (W4, B1).
pub(super) const MIN_ASYNC_TIMEOUT_MS: u32 = 100;

/// Completion deadline in milliseconds: four frame intervals, at least 100 ms.
pub(super) fn async_timeout_ms(fps: u32) -> u32 {
    4_000u64
        .div_ceil(u64::from(fps.max(1)))
        .max(u64::from(MIN_ASYNC_TIMEOUT_MS)) as u32
}

/// The completion event of an asynchronous session (moved here from
/// `nvenc_encode.rs` on 2026-11-17).
pub(super) fn create_completion_event() -> io::Result<HANDLE> {
    // SAFETY: unnamed auto-reset unsignaled kernel event (`nvenc_d3d11.cpp:16`).
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
    if event.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(event)
    }
}

/// At most this many registered input textures per session (2 ring slots,
/// the encoder-size texture, one ring of the next capture generation).
pub(super) const REGISTERED_SLOTS: usize = 4;
/// A registration unused for this many pictures is released (a ring of an
/// earlier capture generation; ring slots alternate every picture).
pub(super) const REGISTERED_IDLE: u64 = 8;

/// Opens an NVENC session on the D3D11 `device`.
pub(super) fn open_session(api: &NvApi, device: &ID3D11Device) -> io::Result<Ptr> {
    let open = api.functions.open_encode_session_ex.ok_or_else(missing)?;
    let mut params = NvEncOpenSessionParams::zeroed();
    params.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
    params.device_type = NV_ENC_DEVICE_TYPE_DIRECTX;
    params.device = device.as_raw();
    params.api_version = NVENCAPI_VERSION;
    let mut encoder = std::ptr::null_mut();
    // SAFETY: versioned parameters naming a live device (the caller keeps
    // it for the session's life) and a writable session slot.
    let result = unsafe { open(&mut params, &mut encoder) };
    status(
        api,
        std::ptr::null_mut(),
        result,
        "nvEncOpenEncodeSessionEx",
    )?;
    if encoder.is_null() {
        return Err(io::Error::other("nvEncOpenEncodeSessionEx gave no session"));
    }
    Ok(encoder)
}

/// The largest NVENC picture of `device` for H.264, HEVC and AV1 (merge W1:
/// the codecs of BK6's Linux `probe_all`); `None` without the driver
/// library, a session or the codec (a codec the GPU lacks answers
/// `nvEncGetEncodeCaps` with an error).
pub(super) fn probe(device: &ID3D11Device) -> [Option<FrameSize>; 3] {
    let Ok(api) = super::nvenc_ffi::load() else {
        return [None; 3];
    };
    let Ok(encoder) = open_session(&api, device) else {
        return [None; 3];
    };
    let size = |codec| {
        let guid = encode_guid(codec)?;
        async_encode(&api, encoder, guid)
            .then(|| largest_picture(&api, encoder, guid))
            .flatten()
    };
    let found = [
        size(CodecKind::H264),
        size(CodecKind::Hevc),
        size(CodecKind::Av1),
    ];
    destroy_session(&api, encoder);
    found
}

/// A registered input texture; the clone keeps the texture alive (and its
/// address unique) while NVENC holds the registration.
pub(super) struct Registered {
    pub texture: ID3D11Texture2D,
    pub handle: Ptr,
    pub used: u64,
}

/// Registers `texture` (NV12 of `size`) with the session.
pub(super) fn register(
    api: &NvApi,
    encoder: Ptr,
    texture: &ID3D11Texture2D,
    size: FrameSize,
    tick: u64,
) -> io::Result<Registered> {
    let register = api.functions.register_resource.ok_or_else(missing)?;
    let mut params = NvEncRegisterResource::zeroed();
    params.version = NV_ENC_REGISTER_RESOURCE_VER;
    params.resource_type = NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX;
    params.width = size.width;
    params.height = size.height;
    params.resource_to_register = texture.as_raw();
    params.buffer_format = NV_ENC_BUFFER_FORMAT_NV12;
    params.buffer_usage = NV_ENC_INPUT_IMAGE;
    // SAFETY: valid session; a live NV12 texture of the session's device.
    let result = unsafe { register(encoder, &mut params) };
    status(api, encoder, result, "nvEncRegisterResource")?;
    Ok(Registered {
        texture: texture.clone(),
        handle: params.registered_resource,
        used: tick,
    })
}

pub(super) fn unregister(api: &NvApi, encoder: Ptr, entry: Registered) {
    if let Some(unregister) = api.functions.unregister_resource {
        // SAFETY: a registration of this session that is not mapped.
        unsafe { unregister(encoder, entry.handle) };
    }
}

/// Which registrations to release before a new one at `tick`: the idle
/// ones, then the least recently used while the table is full.
pub(super) fn evictions(used: &[u64], tick: u64) -> Vec<usize> {
    let mut gone: Vec<usize> = (0..used.len())
        .filter(|index| used[*index].saturating_add(REGISTERED_IDLE) < tick)
        .collect();
    let mut kept: Vec<usize> = (0..used.len()).filter(|i| !gone.contains(i)).collect();
    kept.sort_by_key(|index| used[*index]);
    while kept.len() >= REGISTERED_SLOTS {
        gone.push(kept.remove(0));
    }
    gone.sort_unstable();
    gone
}
