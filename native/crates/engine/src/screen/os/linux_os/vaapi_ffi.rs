//! libva 2.x C interface for the Linux hardware codecs (BK4, B-24, B-38),
//! loaded at run time from `libva.so.2` and `libva-drm.so.2` with
//! `libloading` like PipeWire (`pipewire_abi.rs`) and Media Foundation on
//! Windows (`mf_common.rs`). The program links no libva: without the
//! libraries, without a render node or without a driver the probe finds no
//! device and the codec register stays as before.
//!
//! Source of every constant, signature and layout below: the public headers
//! `va/va.h`, `va/va_vpp.h`, `va/drm/va_drm.h` and `va/va_drmcommon.h` of
//! libva 2.20.0 (tag `2.20.0` of github.com/intel/libva, read on 2026-11-14;
//! VA-API 1.20). The exported names were checked with `nm -D` against the
//! installed `libva.so.2.2000.0`: `vaCreateSurfaces` is versioned, and the
//! unversioned lookup resolves to its default `VA_API_0.33.0` form with the
//! attribute list (the signature below). Enum members without an explicit
//! value were counted from the header. Bit fields follow the System V
//! x86-64/AArch64 rule (first member in the least significant bit).
// Every field of these C layouts exists for its offset, read or not.
#![allow(dead_code)]
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::io;
use std::sync::{Arc, OnceLock};

use libloading::Library;

pub(super) use super::vaapi_display::{render_nodes, VaDisplay};

pub(super) const LIBVA: &str = "libva.so.2";
pub(super) const LIBVA_DRM: &str = "libva-drm.so.2";

pub(super) type VaHandle = *mut c_void;
pub(super) type VaStatus = c_int;
/// `VAGenericID` and its aliases (`VAConfigID`, `VASurfaceID`, …).
pub(super) type VaId = c_uint;

pub(super) const VA_STATUS_SUCCESS: VaStatus = 0;
pub(super) const VA_INVALID_ID: VaId = 0xffff_ffff;

// `VAProfile` (va.h).
pub(super) const VA_PROFILE_NONE: c_int = -1;
pub(super) const VA_PROFILE_H264_MAIN: c_int = 6;
pub(super) const VA_PROFILE_H264_HIGH: c_int = 7;
pub(super) const VA_PROFILE_H264_CONSTRAINED_BASELINE: c_int = 13;
/// `VAProfileHEVCMain`, `VAProfileAV1Profile0` (va.h 2.20.0; BK4b).
pub(super) const VA_PROFILE_HEVC_MAIN: c_int = 17;
pub(super) const VA_PROFILE_AV1_PROFILE0: c_int = 32;
// `VAEntrypoint` (va.h).
pub(super) const VA_ENTRYPOINT_VLD: c_int = 1;
pub(super) const VA_ENTRYPOINT_ENC_SLICE: c_int = 6;
pub(super) const VA_ENTRYPOINT_ENC_SLICE_LP: c_int = 8;
pub(super) const VA_ENTRYPOINT_VIDEO_PROC: c_int = 10;
// `VAConfigAttribType` (va.h).
pub(super) const VA_CONFIG_ATTRIB_RT_FORMAT: c_int = 0;
pub(super) const VA_CONFIG_ATTRIB_RATE_CONTROL: c_int = 5;
pub(super) const VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS: c_int = 10;
pub(super) const VA_CONFIG_ATTRIB_MAX_PICTURE_WIDTH: c_int = 18;
pub(super) const VA_CONFIG_ATTRIB_MAX_PICTURE_HEIGHT: c_int = 19;
/// `VAConfigAttribEncQualityRange` (va.h 2.20.0 line 776): the number of
/// quality levels; 1 is the best quality, the largest the fastest.
pub(super) const VA_CONFIG_ATTRIB_ENC_QUALITY_RANGE: c_int = 21;
// BK4b, the same enum: `VAConfigAttribPredictionDirection`,
// `VAConfigAttribEncHEVCFeatures`, `…EncHEVCBlockSizes`, `…EncAV1`,
// `…EncAV1Ext1`, `…EncAV1Ext2`.
pub(super) const VA_CONFIG_ATTRIB_PREDICTION_DIRECTION: c_int = 39;
pub(super) const VA_CONFIG_ATTRIB_ENC_HEVC_FEATURES: c_int = 50;
pub(super) const VA_CONFIG_ATTRIB_ENC_HEVC_BLOCK_SIZES: c_int = 51;
pub(super) const VA_CONFIG_ATTRIB_ENC_AV1: c_int = 52;
pub(super) const VA_CONFIG_ATTRIB_ENC_AV1_EXT1: c_int = 53;
pub(super) const VA_CONFIG_ATTRIB_ENC_AV1_EXT2: c_int = 54;
/// `VA_PREDICTION_DIRECTION_BI_NOT_EMPTY` (va.h): the driver wants both
/// reference lists of an inter picture filled (low-delay B, "GPB").
pub(super) const VA_PREDICTION_DIRECTION_BI_NOT_EMPTY: u32 = 0x0000_0004;
/// `VA_FEATURE_SUPPORTED`, `VA_FEATURE_REQUIRED` (va.h).
pub(super) const VA_FEATURE_SUPPORTED: u32 = 1;
pub(super) const VA_FEATURE_REQUIRED: u32 = 2;
pub(super) const VA_ATTRIB_NOT_SUPPORTED: u32 = 0x8000_0000;
pub(super) const VA_RT_FORMAT_YUV420: u32 = 0x0000_0001;
pub(super) const VA_RT_FORMAT_RGB32: u32 = 0x0002_0000;
pub(super) const VA_RC_CBR: u32 = 0x0000_0002;
/// Quality-defined VBR (va.h `VA_RC_QVBR`): `quality_factor` 1–51 under
/// `bits_per_second` as the peak; sharp mode of plan 2026-11-17 P9.
pub(super) const VA_RC_QVBR: u32 = 0x0000_0400;
pub(super) const VA_ENC_PACKED_HEADER_SEQUENCE: u32 = 0x0000_0001;
pub(super) const VA_ENC_PACKED_HEADER_PICTURE: u32 = 0x0000_0002;
pub(super) const VA_ENC_PACKED_HEADER_SLICE: u32 = 0x0000_0004;
pub(super) const VA_PROGRESSIVE: c_int = 0x1;
pub(super) const VA_FOURCC_NV12: u32 = 0x3231_564E;
pub(super) const VA_FOURCC_BGRA: u32 = 0x4152_4742;
pub(super) const VA_FOURCC_BGRX: u32 = 0x5852_4742;
// `VABufferType` (va.h).
pub(super) const VA_PICTURE_PARAMETER_BUFFER: c_int = 0;
pub(super) const VA_IQ_MATRIX_BUFFER: c_int = 1;
pub(super) const VA_SLICE_PARAMETER_BUFFER: c_int = 4;
pub(super) const VA_SLICE_DATA_BUFFER: c_int = 5;
pub(super) const VA_ENC_CODED_BUFFER: c_int = 21;
pub(super) const VA_ENC_SEQUENCE_PARAMETER_BUFFER: c_int = 22;
pub(super) const VA_ENC_PICTURE_PARAMETER_BUFFER: c_int = 23;
pub(super) const VA_ENC_SLICE_PARAMETER_BUFFER: c_int = 24;
pub(super) const VA_ENC_PACKED_HEADER_PARAMETER_BUFFER: c_int = 25;
pub(super) const VA_ENC_PACKED_HEADER_DATA_BUFFER: c_int = 26;
pub(super) const VA_ENC_MISC_PARAMETER_BUFFER: c_int = 27;
pub(super) const VA_PROC_PIPELINE_PARAMETER_BUFFER: c_int = 41;
pub(super) const VA_SUBSETS_PARAMETER_BUFFER: c_int = 57;
// `VAEncPackedHeaderType` (va.h).
pub(super) const VA_ENC_PACKED_HEADER_TYPE_SEQUENCE: u32 = 1;
pub(super) const VA_ENC_PACKED_HEADER_TYPE_PICTURE: u32 = 2;
pub(super) const VA_ENC_PACKED_HEADER_TYPE_SLICE: u32 = 3;
// `VASurfaceAttribType`, flags, `VAGenericValueType` (va.h).
pub(super) const VA_SURFACE_ATTRIB_PIXEL_FORMAT: c_int = 1;
pub(super) const VA_SURFACE_ATTRIB_MEMORY_TYPE: c_int = 6;
pub(super) const VA_SURFACE_ATTRIB_EXTERNAL_BUFFER_DESCRIPTOR: c_int = 7;
pub(super) const VA_SURFACE_ATTRIB_SETTABLE: u32 = 0x0000_0002;
pub(super) const VA_GENERIC_VALUE_TYPE_INTEGER: c_int = 1;
pub(super) const VA_GENERIC_VALUE_TYPE_POINTER: c_int = 3;
/// `VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2` (va_drmcommon.h).
pub(super) const VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2: u32 = 0x4000_0000;
// `VAProcColorStandardType` (va_vpp.h).
pub(super) const VA_PROC_COLOR_STANDARD_BT709: u32 = 2;
pub(super) const VA_PROC_COLOR_STANDARD_SRGB: u32 = 8;
/// `VA_SOURCE_RANGE_*` (va_vpp.h).
pub(super) const VA_SOURCE_RANGE_REDUCED: u8 = 1;
pub(super) const VA_SOURCE_RANGE_FULL: u8 = 2;

/// `VAConfigAttrib`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaConfigAttrib {
    pub kind: c_int,
    pub value: u32,
}

/// `VAGenericValue::value`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union VaGenericUnion {
    pub i: i32,
    pub p: *mut c_void,
}

/// `VASurfaceAttrib` with its `VAGenericValue` (24 bytes on LP64).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaSurfaceAttrib {
    pub kind: c_int,
    pub flags: u32,
    pub value_type: c_int,
    pub value: VaGenericUnion,
}

/// `VAImageFormat` (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaImageFormat {
    pub fourcc: u32,
    pub byte_order: u32,
    pub bits_per_pixel: u32,
    pub depth: u32,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub alpha_mask: u32,
    pub va_reserved: [u32; 4],
}

/// `VAImage` (120 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaImage {
    pub image_id: VaId,
    pub format: VaImageFormat,
    pub buf: VaId,
    pub width: u16,
    pub height: u16,
    pub data_size: u32,
    pub num_planes: u32,
    pub pitches: [u32; 3],
    pub offsets: [u32; 3],
    pub num_palette_entries: i32,
    pub entry_bytes: i32,
    pub component_order: [i8; 4],
    pub va_reserved: [u32; 4],
}

/// `VACodedBufferSegment` (48 bytes on LP64).
#[repr(C)]
pub(super) struct VaCodedBufferSegment {
    pub size: u32,
    pub bit_offset: u32,
    pub status: u32,
    pub reserved: u32,
    pub buf: *mut c_void,
    pub next: *mut c_void,
    pub va_reserved: [u32; 4],
}

/// `VARectangle`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaRectangle {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

/// `VAProcColorProperties` (va_vpp.h, 8 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaProcColorProperties {
    pub chroma_sample_location: u8,
    pub color_range: u8,
    pub colour_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub reserved: [u8; 3],
}

/// `VAProcPipelineParameterBuffer` (va_vpp.h; 224 bytes on LP64, where the
/// header reserves `VA_PADDING_LARGE - 16` words).
#[repr(C)]
pub(super) struct VaProcPipelineParameterBuffer {
    pub surface: VaId,
    pub surface_region: *const VaRectangle,
    pub surface_color_standard: u32,
    pub output_region: *const VaRectangle,
    pub output_background_color: u32,
    pub output_color_standard: u32,
    pub pipeline_flags: u32,
    pub filter_flags: u32,
    pub filters: *mut VaId,
    pub num_filters: u32,
    pub forward_references: *mut VaId,
    pub num_forward_references: u32,
    pub backward_references: *mut VaId,
    pub num_backward_references: u32,
    pub rotation_state: u32,
    pub blend_state: *const c_void,
    pub mirror_state: u32,
    pub additional_outputs: *mut VaId,
    pub num_additional_outputs: u32,
    pub input_surface_flag: u32,
    pub output_surface_flag: u32,
    pub input_color_properties: VaProcColorProperties,
    pub output_color_properties: VaProcColorProperties,
    pub processing_mode: u32,
    pub output_hdr_metadata: *mut c_void,
    pub va_reserved: [u32; 16],
}

/// `VADRMPRIMESurfaceDescriptor` (va_drmcommon.h, 312 bytes): the import
/// description of a DMA-BUF (BK8b, `vaapi_vpp::import_dmabuf`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaDrmPrimeObject {
    pub fd: c_int,
    pub size: u32,
    pub drm_format_modifier: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaDrmPrimeLayer {
    pub drm_format: u32,
    pub num_planes: u32,
    pub object_index: [u32; 4],
    pub offset: [u32; 4],
    pub pitch: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaDrmPrimeSurfaceDescriptor {
    pub fourcc: u32,
    pub width: u32,
    pub height: u32,
    pub num_objects: u32,
    pub objects: [VaDrmPrimeObject; 4],
    pub num_layers: u32,
    pub layers: [VaDrmPrimeLayer; 4],
}

/// `VAMessageCallback` and `vaSetErrorCallback`/`vaSetInfoCallback`.
pub(super) type MessageCallback = Option<unsafe extern "C" fn(*mut c_void, *const c_char)>;
pub(super) type SetCallback =
    unsafe extern "C" fn(VaHandle, MessageCallback, *mut c_void) -> MessageCallback;

/// The loaded functions; both libraries stay loaded while this lives.
pub(super) struct VaApi {
    pub get_display_drm: unsafe extern "C" fn(c_int) -> VaHandle,
    pub set_error_callback: SetCallback,
    pub set_info_callback: SetCallback,
    pub initialize: unsafe extern "C" fn(VaHandle, *mut c_int, *mut c_int) -> VaStatus,
    pub terminate: unsafe extern "C" fn(VaHandle) -> VaStatus,
    pub error_str: unsafe extern "C" fn(VaStatus) -> *const c_char,
    pub max_num_profiles: unsafe extern "C" fn(VaHandle) -> c_int,
    pub max_num_entrypoints: unsafe extern "C" fn(VaHandle) -> c_int,
    pub query_config_profiles: unsafe extern "C" fn(VaHandle, *mut c_int, *mut c_int) -> VaStatus,
    pub query_config_entrypoints:
        unsafe extern "C" fn(VaHandle, c_int, *mut c_int, *mut c_int) -> VaStatus,
    pub get_config_attributes:
        unsafe extern "C" fn(VaHandle, c_int, c_int, *mut VaConfigAttrib, c_int) -> VaStatus,
    pub create_config: unsafe extern "C" fn(
        VaHandle,
        c_int,
        c_int,
        *mut VaConfigAttrib,
        c_int,
        *mut VaId,
    ) -> VaStatus,
    pub destroy_config: unsafe extern "C" fn(VaHandle, VaId) -> VaStatus,
    pub create_surfaces: unsafe extern "C" fn(
        VaHandle,
        c_uint,
        c_uint,
        c_uint,
        *mut VaId,
        c_uint,
        *mut VaSurfaceAttrib,
        c_uint,
    ) -> VaStatus,
    pub destroy_surfaces: unsafe extern "C" fn(VaHandle, *mut VaId, c_int) -> VaStatus,
    pub create_context: unsafe extern "C" fn(
        VaHandle,
        VaId,
        c_int,
        c_int,
        c_int,
        *mut VaId,
        c_int,
        *mut VaId,
    ) -> VaStatus,
    pub destroy_context: unsafe extern "C" fn(VaHandle, VaId) -> VaStatus,
    pub create_buffer: unsafe extern "C" fn(
        VaHandle,
        VaId,
        c_int,
        c_uint,
        c_uint,
        *mut c_void,
        *mut VaId,
    ) -> VaStatus,
    pub map_buffer: unsafe extern "C" fn(VaHandle, VaId, *mut *mut c_void) -> VaStatus,
    pub unmap_buffer: unsafe extern "C" fn(VaHandle, VaId) -> VaStatus,
    pub destroy_buffer: unsafe extern "C" fn(VaHandle, VaId) -> VaStatus,
    pub begin_picture: unsafe extern "C" fn(VaHandle, VaId, VaId) -> VaStatus,
    pub render_picture: unsafe extern "C" fn(VaHandle, VaId, *mut VaId, c_int) -> VaStatus,
    pub end_picture: unsafe extern "C" fn(VaHandle, VaId) -> VaStatus,
    pub sync_surface: unsafe extern "C" fn(VaHandle, VaId) -> VaStatus,
    pub sync_surface2: Option<unsafe extern "C" fn(VaHandle, VaId, u64) -> VaStatus>,
    pub create_image:
        unsafe extern "C" fn(VaHandle, *mut VaImageFormat, c_int, c_int, *mut VaImage) -> VaStatus,
    pub destroy_image: unsafe extern "C" fn(VaHandle, VaId) -> VaStatus,
    pub derive_image: unsafe extern "C" fn(VaHandle, VaId, *mut VaImage) -> VaStatus,
    pub get_image:
        unsafe extern "C" fn(VaHandle, VaId, c_int, c_int, c_uint, c_uint, VaId) -> VaStatus,
    pub put_image: unsafe extern "C" fn(
        VaHandle,
        VaId,
        VaId,
        c_int,
        c_int,
        c_uint,
        c_uint,
        c_int,
        c_int,
        c_uint,
        c_uint,
    ) -> VaStatus,
    _drm: Library,
    _va: Library,
}

fn symbol<T: Copy>(library: &Library, file: &str, name: &str) -> io::Result<T> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: `T` is the C signature of the named libva function as spelled
    // out in `load_from` from va.h/va_drm.h 2.20.0.
    let found = unsafe { library.get::<T>(&bytes) };
    found
        .map(|symbol| *symbol)
        .map_err(|error| io::Error::other(format!("{file}: {name}: {error}")))
}

fn open_library(name: &str) -> io::Result<Library> {
    // SAFETY: loading libva runs only its own initializers (no global state
    // of this program is touched); libva loads its driver later, inside
    // `vaInitialize`.
    unsafe { Library::new(name) }
        .map_err(|error| io::Error::new(io::ErrorKind::NotFound, format!("{name}: {error}")))
}

/// Loads libva from the shared objects `va` and `drm`.
pub(super) fn load_from(va: &str, drm: &str) -> io::Result<VaApi> {
    let lib = open_library(va)?;
    let drm_lib = open_library(drm)?;
    macro_rules! get {
        ($name:expr) => {
            symbol(&lib, va, $name)?
        };
    }
    Ok(VaApi {
        get_display_drm: symbol(&drm_lib, drm, "vaGetDisplayDRM")?,
        set_error_callback: get!("vaSetErrorCallback"),
        set_info_callback: get!("vaSetInfoCallback"),
        initialize: get!("vaInitialize"),
        terminate: get!("vaTerminate"),
        error_str: get!("vaErrorStr"),
        max_num_profiles: get!("vaMaxNumProfiles"),
        max_num_entrypoints: get!("vaMaxNumEntrypoints"),
        query_config_profiles: get!("vaQueryConfigProfiles"),
        query_config_entrypoints: get!("vaQueryConfigEntrypoints"),
        get_config_attributes: get!("vaGetConfigAttributes"),
        create_config: get!("vaCreateConfig"),
        destroy_config: get!("vaDestroyConfig"),
        create_surfaces: get!("vaCreateSurfaces"),
        destroy_surfaces: get!("vaDestroySurfaces"),
        create_context: get!("vaCreateContext"),
        destroy_context: get!("vaDestroyContext"),
        create_buffer: get!("vaCreateBuffer"),
        map_buffer: get!("vaMapBuffer"),
        unmap_buffer: get!("vaUnmapBuffer"),
        destroy_buffer: get!("vaDestroyBuffer"),
        begin_picture: get!("vaBeginPicture"),
        render_picture: get!("vaRenderPicture"),
        end_picture: get!("vaEndPicture"),
        sync_surface: get!("vaSyncSurface"),
        sync_surface2: symbol(&lib, va, "vaSyncSurface2").ok(),
        create_image: get!("vaCreateImage"),
        destroy_image: get!("vaDestroyImage"),
        derive_image: get!("vaDeriveImage"),
        get_image: get!("vaGetImage"),
        put_image: get!("vaPutImage"),
        _drm: drm_lib,
        _va: lib,
    })
}

/// libva of this system, loaded once per process; a failure is kept, so a
/// missing library is looked for only once.
pub(super) fn load() -> Result<Arc<VaApi>, String> {
    static API: OnceLock<Result<Arc<VaApi>, String>> = OnceLock::new();
    API.get_or_init(|| {
        load_from(LIBVA, LIBVA_DRM)
            .map(Arc::new)
            .map_err(|error| error.to_string())
    })
    .clone()
}
