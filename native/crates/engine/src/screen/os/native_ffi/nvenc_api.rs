//! The NVENC interface both NVENC encoders share (Linux BK4 through CUDA,
//! Windows BK12 through D3D11; one module since merge W2, plan 2026-11-14
//! „Bildkette“): versions, constants, GUIDs, the function list, the check of
//! the driver's API version and the calls that need no device (the driver's
//! error text, closing a session, the capability queries). Loading the
//! library and the device of a session stay platform code
//! (`linux_os/nvenc_ffi.rs` with `nvenc_device.rs`, `windows/nvenc_ffi.rs`
//! with `nvenc_device.rs`).
//!
//! Source of every constant, signature and the function list: `nvEncodeAPI.h`
//! of NVENC API 12.1 (`NVENCAPI_MAJOR_VERSION 12`, `NVENCAPI_MINOR_VERSION 1`,
//! lines 118-126) as published in FFmpeg's nv-codec-headers tag
//! `n12.1.14.0`, read on 2026-11-14. `NVENCAPI` is `__stdcall` on Windows
//! (lines 70-71), so every entry is `extern "system"`; on Linux and on
//! Windows x86_64 that is the C convention.
// Every field of these C layouts exists for its offset, read or not.
#![allow(dead_code)]
use std::ffi::{c_char, c_int};
use std::io;

use crate::screen::core::negotiate::CodecKind;

use super::abi::{
    Guid, NvEncCapsParam, NvEncCreateBitstreamBuffer, NvEncCreateInputBuffer, NvEncEventParams,
    NvEncInitializeParams, NvEncLockBitstream, NvEncLockInputBuffer, NvEncMapInputResource,
    NvEncOpenSessionParams, NvEncPicParams, NvEncPresetConfig, NvEncReconfigureParams,
    NvEncRegisterResource, Ptr, Zeroed,
};

pub(in crate::screen::os) type NvEncStatus = c_int;
pub(in crate::screen::os) const NV_ENC_SUCCESS: NvEncStatus = 0;
/// `NV_ENC_ERR_INVALID_VERSION`: position 15 of `NVENCSTATUS`.
pub(in crate::screen::os) const NV_ENC_ERR_INVALID_VERSION: NvEncStatus = 15;

/// The names of `NVENCSTATUS` in enum order (`NV_ENC_SUCCESS` = 0 to
/// `NV_ENC_ERR_NEED_MORE_OUTPUT` = 26; no member has an explicit value).
const STATUS_NAMES: [&str; 27] = [
    "NV_ENC_SUCCESS",
    "NV_ENC_ERR_NO_ENCODE_DEVICE",
    "NV_ENC_ERR_UNSUPPORTED_DEVICE",
    "NV_ENC_ERR_INVALID_ENCODERDEVICE",
    "NV_ENC_ERR_INVALID_DEVICE",
    "NV_ENC_ERR_DEVICE_NOT_EXIST",
    "NV_ENC_ERR_INVALID_PTR",
    "NV_ENC_ERR_INVALID_EVENT",
    "NV_ENC_ERR_INVALID_PARAM",
    "NV_ENC_ERR_INVALID_CALL",
    "NV_ENC_ERR_OUT_OF_MEMORY",
    "NV_ENC_ERR_ENCODER_NOT_INITIALIZED",
    "NV_ENC_ERR_UNSUPPORTED_PARAM",
    "NV_ENC_ERR_LOCK_BUSY",
    "NV_ENC_ERR_NOT_ENOUGH_BUFFER",
    "NV_ENC_ERR_INVALID_VERSION",
    "NV_ENC_ERR_MAP_FAILED",
    "NV_ENC_ERR_NEED_MORE_INPUT",
    "NV_ENC_ERR_ENCODER_BUSY",
    "NV_ENC_ERR_EVENT_NOT_REGISTERD",
    "NV_ENC_ERR_GENERIC",
    "NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY",
    "NV_ENC_ERR_UNIMPLEMENTED",
    "NV_ENC_ERR_RESOURCE_REGISTER_FAILED",
    "NV_ENC_ERR_RESOURCE_NOT_REGISTERED",
    "NV_ENC_ERR_RESOURCE_NOT_MAPPED",
    "NV_ENC_ERR_NEED_MORE_OUTPUT",
];

/// The header name of an `NVENCSTATUS` (fix of 2026-11-18: the host log
/// named only the number); `"unknown"` outside the enum.
pub(in crate::screen::os) fn status_name(result: NvEncStatus) -> &'static str {
    usize::try_from(result)
        .ok()
        .and_then(|index| STATUS_NAMES.get(index))
        .copied()
        .unwrap_or("unknown")
}

/// Fix of 2026-11-18 (host log of a Windows VM with an NVIDIA GPU:
/// `nvEncOpenEncodeSessionEx: NVENC status 15`): the driver's refusals of
/// API 12.1, per adapter (packed LUID) and codec. The layouts and versions
/// of the call match `nvEncodeAPI.h` 12.1 (task tests of
/// `shadowhost_nvenc_task_tests.rs`), and `create_functions` let a session
/// open only after `NvEncodeAPIGetMaxSupportedVersion` reported 12.1 or
/// newer; so `NV_ENC_ERR_INVALID_VERSION` there is a limit of the driver
/// installation [assumption A-SHADOWHOST-NVENC-VERSION], not of this
/// program. A refusal blocks only the codec on that adapter for this
/// process (the same PC reported NVENC HEVC verified while H.264 was
/// refused); it counts only while no session of that codec opened there.
#[derive(Debug, Default)]
pub(in crate::screen::os) struct VersionRefusals {
    refused: Vec<(u64, CodecKind, String)>,
    opened: Vec<(u64, CodecKind)>,
}

impl VersionRefusals {
    pub(in crate::screen::os) const fn new() -> Self {
        Self {
            refused: Vec::new(),
            opened: Vec::new(),
        }
    }

    /// The refusal that blocks `codec` on `adapter`, if any.
    pub(in crate::screen::os) fn refused(&self, adapter: u64, codec: CodecKind) -> Option<String> {
        self.refused
            .iter()
            .find(|(own, kind, _)| *own == adapter && *kind == codec)
            .map(|(_, _, reason)| reason.clone())
    }

    /// The result of opening a session of `codec` on `adapter`: a session
    /// is noted, a first version refusal blocks that pair and is returned
    /// (for the host log); everything else leaves the ledger alone.
    pub(in crate::screen::os) fn note(
        &mut self,
        adapter: u64,
        codec: CodecKind,
        result: NvEncStatus,
    ) -> Option<String> {
        if result == NV_ENC_SUCCESS {
            if !self.opened.contains(&(adapter, codec)) {
                self.opened.push((adapter, codec));
            }
            return None;
        }
        if result != NV_ENC_ERR_INVALID_VERSION
            || self.opened.contains(&(adapter, codec))
            || self.refused(adapter, codec).is_some()
        {
            return None;
        }
        let reason = format!(
            "NVENC {codec:?} not available on adapter {adapter:016x}: the driver reports API \
             {NVENCAPI_MAJOR_VERSION}.{NVENCAPI_MINOR_VERSION} or newer but refuses its sessions \
             (nvEncOpenEncodeSessionEx: NVENC status {result} NV_ENC_ERR_INVALID_VERSION); \
             not tried again in this process, other codecs stay tried"
        );
        self.refused.push((adapter, codec, reason.clone()));
        Some(reason)
    }
}

pub(in crate::screen::os) const NVENCAPI_MAJOR_VERSION: u32 = 12;
pub(in crate::screen::os) const NVENCAPI_MINOR_VERSION: u32 = 1;
pub(in crate::screen::os) const NVENCAPI_VERSION: u32 =
    NVENCAPI_MAJOR_VERSION | (NVENCAPI_MINOR_VERSION << 24);

/// `NVENCAPI_STRUCT_VERSION(ver)` (line 126).
pub(in crate::screen::os) const fn struct_version(version: u32) -> u32 {
    NVENCAPI_VERSION | (version << 16) | (0x7 << 28)
}
pub(in crate::screen::os) const NV_ENCODE_API_FUNCTION_LIST_VER: u32 = struct_version(2);
pub(in crate::screen::os) const NV_ENC_CAPS_PARAM_VER: u32 = struct_version(1);
pub(in crate::screen::os) const NV_ENC_CREATE_INPUT_BUFFER_VER: u32 = struct_version(1);
pub(in crate::screen::os) const NV_ENC_CREATE_BITSTREAM_BUFFER_VER: u32 = struct_version(1);
pub(in crate::screen::os) const NV_ENC_LOCK_INPUT_BUFFER_VER: u32 = struct_version(1);
pub(in crate::screen::os) const NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER: u32 = struct_version(1);
pub(in crate::screen::os) const NV_ENC_RC_PARAMS_VER: u32 = struct_version(1);
pub(in crate::screen::os) const NV_ENC_CONFIG_VER: u32 = struct_version(8) | (1 << 31);
pub(in crate::screen::os) const NV_ENC_INITIALIZE_PARAMS_VER: u32 = struct_version(6) | (1 << 31);
pub(in crate::screen::os) const NV_ENC_RECONFIGURE_PARAMS_VER: u32 = struct_version(1) | (1 << 31);
pub(in crate::screen::os) const NV_ENC_PRESET_CONFIG_VER: u32 = struct_version(4) | (1 << 31);
pub(in crate::screen::os) const NV_ENC_PIC_PARAMS_VER: u32 = struct_version(6) | (1 << 31);
pub(in crate::screen::os) const NV_ENC_LOCK_BITSTREAM_VER: u32 = struct_version(1) | (1 << 31);
/// Lines 2542, 2656 and 2722 (Windows' D3D11 input and completion event).
pub(in crate::screen::os) const NV_ENC_MAP_INPUT_RESOURCE_VER: u32 = struct_version(4);
pub(in crate::screen::os) const NV_ENC_REGISTER_RESOURCE_VER: u32 = struct_version(4);
pub(in crate::screen::os) const NV_ENC_EVENT_PARAMS_VER: u32 = struct_version(1);

pub(in crate::screen::os) const NVENC_INFINITE_GOPLENGTH: u32 = 0xffff_ffff;
pub(in crate::screen::os) const NV_ENC_PARAMS_RC_CBR: u32 = 0x2;
/// `NV_ENC_PARAMS_RC_VBR` (nv-codec-headers n12.1.14.0 line 265), with
/// `targetQuality` the constant-quality mode of plan 2026-11-17 P9.
pub(in crate::screen::os) const NV_ENC_PARAMS_RC_VBR: u32 = 0x1;
pub(in crate::screen::os) const NV_ENC_PIC_STRUCT_FRAME: u32 = 0x01;
pub(in crate::screen::os) const NV_ENC_PIC_TYPE_IDR: u32 = 0x03;
pub(in crate::screen::os) const NV_ENC_PIC_FLAG_FORCEIDR: u32 = 0x2;
pub(in crate::screen::os) const NV_ENC_PIC_FLAG_OUTPUT_SPSPPS: u32 = 0x4;
pub(in crate::screen::os) const NV_ENC_BUFFER_FORMAT_NV12: u32 = 0x0000_0001;
pub(in crate::screen::os) const NV_ENC_MEMORY_HEAP_SYSMEM_CACHED: u32 = 2;
pub(in crate::screen::os) const NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY: u32 = 3;
/// `NV_ENC_DEVICE_TYPE_DIRECTX` (line 796; also D3D11 devices) and
/// `NV_ENC_DEVICE_TYPE_CUDA`.
pub(in crate::screen::os) const NV_ENC_DEVICE_TYPE_DIRECTX: u32 = 0x0;
pub(in crate::screen::os) const NV_ENC_DEVICE_TYPE_CUDA: u32 = 0x1;
/// `NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX` (line 771), `NV_ENC_INPUT_IMAGE`
/// (line 784).
pub(in crate::screen::os) const NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX: u32 = 0x0;
pub(in crate::screen::os) const NV_ENC_INPUT_IMAGE: u32 = 0x0;
/// `NV_ENC_NUM_REF_FRAMES_1` (line 808).
pub(in crate::screen::os) const NV_ENC_NUM_REF_FRAMES_1: u32 = 0x1;
/// `NV_ENC_CAPS` members counted from the enum (lines 820-1192, no member
/// has an explicit value).
pub(in crate::screen::os) const NV_ENC_CAPS_WIDTH_MAX: u32 = 16;
pub(in crate::screen::os) const NV_ENC_CAPS_HEIGHT_MAX: u32 = 17;
pub(in crate::screen::os) const NV_ENC_CAPS_SUPPORT_REF_PIC_INVALIDATION: u32 = 28;
pub(in crate::screen::os) const NV_ENC_CAPS_ASYNC_ENCODE_SUPPORT: u32 = 30;
pub(in crate::screen::os) const NV_ENC_CAPS_SUPPORT_MULTIPLE_REF_FRAMES: u32 = 47;
/// `NV_ENC_LOCK_BITSTREAM` bit `doNotWait` (bit 0, line 2465).
pub(in crate::screen::os) const LOCK_DO_NOT_WAIT: u32 = 1;
/// `NV_ENC_RC_PARAMS` bit `zeroReorderDelay` (bit 9) and `enableLookahead`
/// (bit 5), lines 1496 and 1500.
pub(in crate::screen::os) const RC_ZERO_REORDER_DELAY: u32 = 1 << 9;
pub(in crate::screen::os) const RC_ENABLE_LOOKAHEAD: u32 = 1 << 5;
/// `NV_ENC_CONFIG_H264` bit `repeatSPSPPS` (bit 12, line 1685).
pub(in crate::screen::os) const H264_REPEAT_SPSPPS: u32 = 1 << 12;
/// `NV_ENC_VUI_*` values for BT.709 (H.264 Table E-3/E-4/E-5).
pub(in crate::screen::os) const VUI_BT709: u32 = 1;

pub(in crate::screen::os) const NV_ENC_CODEC_H264_GUID: Guid = Guid {
    data1: 0x6bc8_2762,
    data2: 0x4e63,
    data3: 0x4ca4,
    data4: [0xaa, 0x85, 0x1e, 0x50, 0xf3, 0x21, 0xf6, 0xbf],
};
pub(in crate::screen::os) const NV_ENC_H264_PROFILE_HIGH_GUID: Guid = Guid {
    data1: 0xe7cb_c309,
    data2: 0x4f7a,
    data3: 0x4b89,
    data4: [0xaf, 0x2a, 0xd5, 0x37, 0xc9, 0x2b, 0xe3, 0x10],
};
pub(in crate::screen::os) const NV_ENC_PRESET_P1_GUID: Guid = Guid {
    data1: 0xfc0a_8d3e,
    data2: 0x45f8,
    data3: 0x4cf8,
    data4: [0x80, 0xc7, 0x29, 0x88, 0x71, 0x59, 0x0e, 0xbf],
};

type E = Ptr;
type F<A> = Option<unsafe extern "system" fn(E, A) -> NvEncStatus>;

/// `NV_ENCODE_API_FUNCTION_LIST` (2552 bytes): 43 entries in header order
/// (lines 4375-4423), the unused ones as plain pointers.
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncFunctions {
    pub version: u32,
    pub reserved: u32,
    pub open_encode_session: Ptr,
    pub get_encode_guid_count: Ptr,
    pub get_encode_profile_guid_count: Ptr,
    pub get_encode_profile_guids: Ptr,
    pub get_encode_guids: Ptr,
    pub get_input_format_count: Ptr,
    pub get_input_formats: Ptr,
    pub get_encode_caps:
        Option<unsafe extern "system" fn(E, Guid, *mut NvEncCapsParam, *mut c_int) -> NvEncStatus>,
    pub get_encode_preset_count: Ptr,
    pub get_encode_preset_guids: Ptr,
    pub get_encode_preset_config: Ptr,
    pub initialize_encoder: F<*mut NvEncInitializeParams>,
    pub create_input_buffer: F<*mut NvEncCreateInputBuffer>,
    pub destroy_input_buffer: F<Ptr>,
    pub create_bitstream_buffer: F<*mut NvEncCreateBitstreamBuffer>,
    pub destroy_bitstream_buffer: F<Ptr>,
    pub encode_picture: F<*mut NvEncPicParams>,
    pub lock_bitstream: F<*mut NvEncLockBitstream>,
    pub unlock_bitstream: F<Ptr>,
    pub lock_input_buffer: F<*mut NvEncLockInputBuffer>,
    pub unlock_input_buffer: F<Ptr>,
    pub get_encode_stats: Ptr,
    pub get_sequence_params: Ptr,
    pub register_async_event: F<*mut NvEncEventParams>,
    pub unregister_async_event: F<*mut NvEncEventParams>,
    pub map_input_resource: F<*mut NvEncMapInputResource>,
    pub unmap_input_resource: F<Ptr>,
    pub destroy_encoder: Option<unsafe extern "system" fn(E) -> NvEncStatus>,
    /// `NvEncInvalidateRefFrames(encoder, uint64_t invalidRefFrameTimeStamp)`
    /// (line 4039).
    pub invalidate_ref_frames: F<u64>,
    pub open_encode_session_ex:
        Option<unsafe extern "system" fn(*mut NvEncOpenSessionParams, *mut E) -> NvEncStatus>,
    pub register_resource: F<*mut NvEncRegisterResource>,
    pub unregister_resource: F<Ptr>,
    pub reconfigure_encoder: F<*mut NvEncReconfigureParams>,
    pub reserved1: Ptr,
    pub create_mv_buffer: Ptr,
    pub destroy_mv_buffer: Ptr,
    pub run_motion_estimation_only: Ptr,
    pub get_last_error_string: Option<unsafe extern "system" fn(E) -> *const c_char>,
    pub set_io_cuda_streams: Ptr,
    pub get_encode_preset_config_ex: Option<
        unsafe extern "system" fn(E, Guid, Guid, u32, *mut NvEncPresetConfig) -> NvEncStatus,
    >,
    pub get_sequence_param_ex: Ptr,
    pub restore_encoder_state: Ptr,
    pub lookahead_picture: Ptr,
    pub reserved2: [Ptr; 275],
}

impl Zeroed for NvEncFunctions {}

/// `NvEncodeAPIGetMaxSupportedVersion` (line 4272).
pub(in crate::screen::os) type MaxVersionFn = unsafe extern "system" fn(*mut u32) -> NvEncStatus;
/// `NvEncodeAPICreateInstance` (line 4443).
pub(in crate::screen::os) type CreateInstanceFn =
    unsafe extern "system" fn(*mut NvEncFunctions) -> NvEncStatus;

/// The largest NVENC API version of the driver: "The 4 least significant
/// bits ... indicate the minor version and the rest ... the major version"
/// (line 4263-4266), i.e. `major << 4 | minor`.
pub(in crate::screen::os) fn supported(driver: u32) -> bool {
    driver >= ((NVENCAPI_MAJOR_VERSION << 4) | NVENCAPI_MINOR_VERSION)
}

/// The function list of a loaded driver library whose two entry points
/// are `max_version` and `create`; `Unsupported` for a driver before 12.1.
pub(in crate::screen::os) fn create_functions(
    max_version: MaxVersionFn,
    create: CreateInstanceFn,
) -> io::Result<NvEncFunctions> {
    let mut driver = 0u32;
    // SAFETY: one writable u32.
    if unsafe { max_version(&mut driver) } != NV_ENC_SUCCESS || !supported(driver) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("NVENC driver API {driver:#x} is older than 12.1"),
        ));
    }
    let mut functions = NvEncFunctions::zeroed();
    functions.version = NV_ENCODE_API_FUNCTION_LIST_VER;
    // SAFETY: a zeroed, versioned function list for the driver to fill.
    if unsafe { create(&mut functions) } != NV_ENC_SUCCESS {
        return Err(io::Error::other("NvEncodeAPICreateInstance failed"));
    }
    Ok(functions)
}

/// The loaded function list of a platform's NVENC (`NvApi` of
/// `linux_os/nvenc_ffi.rs` and `windows/nvenc_ffi.rs`).
pub(in crate::screen::os) trait NvTable {
    fn table(&self) -> &NvEncFunctions;
}

impl<T: NvTable + ?Sized> NvTable for std::sync::Arc<T> {
    fn table(&self) -> &NvEncFunctions {
        (**self).table()
    }
}

/// `Ok` for `NV_ENC_SUCCESS`, else the driver's error text with `what`.
pub(in crate::screen::os) fn status<A: NvTable + ?Sized>(
    api: &A,
    encoder: Ptr,
    result: NvEncStatus,
    what: &str,
) -> io::Result<()> {
    if result == NV_ENC_SUCCESS {
        return Ok(());
    }
    let detail = api
        .table()
        .get_last_error_string
        .filter(|_| !encoder.is_null())
        // SAFETY: a valid encoder; the driver owns the returned string.
        .map(|last| unsafe { last(encoder) })
        .filter(|text| !text.is_null())
        .map(|text| {
            // SAFETY: a NUL-terminated string of the driver.
            unsafe { std::ffi::CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_default();
    Err(io::Error::other(format!(
        "{what}: NVENC status {result} {} {detail}",
        status_name(result)
    )))
}

pub(in crate::screen::os) fn missing() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "NVENC function table incomplete",
    )
}

/// Closes a session the platform's `open_session` opened.
pub(in crate::screen::os) fn destroy_session<A: NvTable + ?Sized>(api: &A, encoder: Ptr) {
    if let Some(destroy) = api.table().destroy_encoder {
        // SAFETY: a session opened by `open_session`, used no more.
        unsafe { destroy(encoder) };
    }
}

/// One capability of the session for the codec `guid` (`NvEncGetEncodeCaps`).
pub(in crate::screen::os) fn capability<A: NvTable + ?Sized>(
    api: &A,
    encoder: Ptr,
    guid: Guid,
    which: u32,
) -> Option<u32> {
    let caps = api.table().get_encode_caps?;
    let mut param = NvEncCapsParam::zeroed();
    param.version = NV_ENC_CAPS_PARAM_VER;
    param.caps_to_query = which;
    let mut value = 0;
    // SAFETY: valid session, versioned query, writable result.
    let result = unsafe { caps(encoder, guid, &mut param, &mut value) };
    (result == NV_ENC_SUCCESS && value >= 0).then_some(value as u32)
}

/// Whether the GPU and driver support asynchronous completion events for
/// `guid` (`NV_ENC_CAPS_ASYNC_ENCODE_SUPPORT`, Sunshine `nvenc_base.cpp:219`).
pub(in crate::screen::os) fn async_encode<A: NvTable + ?Sized>(
    api: &A,
    encoder: Ptr,
    guid: Guid,
) -> bool {
    capability(api, encoder, guid, NV_ENC_CAPS_ASYNC_ENCODE_SUPPORT).is_some_and(|value| value > 0)
}

/// Reference invalidation needs both the capability and several reference
/// frames (Sunshine `nvenc_base.cpp:223`, `:292-295`), per codec `guid`.
pub(in crate::screen::os) fn invalidation<A: NvTable + ?Sized>(
    api: &A,
    encoder: Ptr,
    guid: Guid,
) -> bool {
    let has = |which| capability(api, encoder, guid, which).is_some_and(|value| value > 0);
    has(NV_ENC_CAPS_SUPPORT_REF_PIC_INVALIDATION) && has(NV_ENC_CAPS_SUPPORT_MULTIPLE_REF_FRAMES)
}

/// The largest picture of the session for the codec `guid`
/// (`NV_ENC_CAPS_WIDTH_MAX`, `NV_ENC_CAPS_HEIGHT_MAX`), within the
/// program's largest frame; `None` when the GPU lacks the codec.
pub(in crate::screen::os) fn largest_picture<A: NvTable + ?Sized>(
    api: &A,
    encoder: Ptr,
    guid: Guid,
) -> Option<crate::screen::core::pixel_convert::FrameSize> {
    use crate::screen::core::pixel_convert::FrameSize;
    let width = capability(api, encoder, guid, NV_ENC_CAPS_WIDTH_MAX).filter(|w| *w > 0)?;
    let height = capability(api, encoder, guid, NV_ENC_CAPS_HEIGHT_MAX).filter(|h| *h > 0)?;
    Some(FrameSize::new(
        width.min(FrameSize::MAX.width),
        height.min(FrameSize::MAX.height),
    ))
}

#[cfg(test)]
#[path = "shadowhost_nvenc_task_tests.rs"]
mod shadowhost_nvenc_task_tests;
