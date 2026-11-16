//! AMD Advanced Media Framework (AMF) C interface of the Windows AMF encoder
//! (BK12, B-22). Source of every layout, value and name: the public headers
//! of the AMF SDK, tag `v1.4.35` of GPUOpen-LibrariesAndSDKs/AMF
//! (`AMF_VERSION_MAJOR 1`, `MINOR 4`, `RELEASE 35`, `core/Version.h:52-55`),
//! read on 2026-11-14: the C (`#else // #if defined(__cplusplus)`) parts of
//! `core/Platform.h`, `core/Result.h`, `core/Variant.h`, `core/Interface.h`,
//! `core/PropertyStorage.h`, `core/Data.h`, `core/Buffer.h`, `core/Surface.h`,
//! `core/Context.h`, `core/Factory.h`, `components/Component.h`,
//! `components/ComponentCaps.h`, `components/VideoEncoderVCE.h` and
//! `components/ColorSpace.h`. Every object is a pointer to a pointer to its
//! vtable; `AMF_STD_CALL` is `__stdcall` on Windows (`Platform.h:116`), the
//! DLL entries `AMF_CDECL_CALL` (`Factory.h:105-106`); on x86_64 both are the
//! system convention. Only the entries this encoder calls are typed, the
//! others are plain pointers that keep the offsets. `amf_long` is `long`
//! (32 bit on Windows, `Platform.h:203`), `amf_bool` in C is `amf_uint8`
//! (`:198`), C enums are `int`. Sizes are asserted by
//! `bildkette_task_win_vendor_amf_layouts`.
// Every entry of these C layouts exists for its offset, called or not.
#![allow(dead_code)]
use std::ffi::{c_long, c_void};

pub(super) type This = *mut c_void;
pub(super) type Ptr = *const c_void;
/// `AMF_RESULT` (`Result.h:43`; values counted from `AMF_OK = 0`).
pub(super) type AmfResult = i32;
pub(super) const AMF_OK: AmfResult = 0;
pub(super) const AMF_EOF: AmfResult = 23;
pub(super) const AMF_REPEAT: AmfResult = 24;
pub(super) const AMF_INPUT_FULL: AmfResult = 25;
pub(super) const AMF_NEED_MORE_INPUT: AmfResult = 44;

/// `AMF_DLL_NAMEA` on `_M_AMD64` (`Factory.h:111-112`).
pub(super) const AMF_LIBRARY: &str = "amfrt64.dll";
/// `AMF_MAKE_FULL_VERSION` (`Version.h:45`) of 1.4.35.0.
pub(super) const fn full_version(major: u64, minor: u64, release: u64, build: u64) -> u64 {
    (major << 48) | (minor << 32) | (release << 16) | build
}
pub(super) const AMF_FULL_VERSION: u64 = full_version(1, 4, 35, 0);
/// The oldest runtime whose vtables this file describes (assumption
/// A-AMF-MIN: every interface used here exists unchanged since 1.4.0).
pub(super) const AMF_MIN_VERSION: u64 = full_version(1, 4, 0, 0);

/// `AMF_DX11_0` (`Data.h:75`), `AMF_SURFACE_NV12` (`Surface.h:53`).
pub(super) const AMF_DX11_0: i32 = 110;
pub(super) const AMF_SURFACE_NV12: i32 = 1;
/// `AMFVideoEncoderVCE_AVC` (`VideoEncoderVCE.h:45`).
pub(super) const ENCODER_AVC: &str = "AMFVideoEncoderVCE_AVC";

/// `AMFGuid` (`Platform.h:498-545`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AmfGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

/// `IID_AMFBuffer` (`Buffer.h:135`).
pub(super) const IID_AMF_BUFFER: AmfGuid = AmfGuid {
    data1: 0xb04b_7248,
    data2: 0xb6f0,
    data3: 0x4321,
    data4: [0xb6, 0x91, 0xba, 0xa4, 0x74, 0x0f, 0x9f, 0xcb],
};

/// `AMFSize` (`Platform.h:253`), `AMFRate` (`:371`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AmfSize {
    pub width: i32,
    pub height: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AmfRate {
    pub num: u32,
    pub den: u32,
}

/// The value union of `AMFVariantStruct` (`Variant.h:83-102`), 16 bytes
/// (its largest members are `AMFRect` and `AMFFloatVector4D`).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union AmfValue {
    pub boolean: u8,
    pub int64: i64,
    pub double: f64,
    pub pointer: *mut c_void,
    pub size: AmfSize,
    pub rate: AmfRate,
    pub raw: [u64; 2],
}

/// `AMFVariantStruct` (24 bytes, `Variant.h:80-103`), passed by value to
/// `SetProperty`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct AmfVariant {
    pub kind: i32,
    pub value: AmfValue,
}

/// `AMF_VARIANT_TYPE` (`Variant.h:52-76`).
pub(super) const VARIANT_EMPTY: i32 = 0;
pub(super) const VARIANT_BOOL: i32 = 1;
pub(super) const VARIANT_INT64: i32 = 2;
pub(super) const VARIANT_SIZE: i32 = 5;
pub(super) const VARIANT_RATE: i32 = 7;

impl AmfVariant {
    pub(super) fn empty() -> Self {
        Self {
            kind: VARIANT_EMPTY,
            value: AmfValue { raw: [0; 2] },
        }
    }
    pub(super) fn int64(value: i64) -> Self {
        let mut variant = Self::empty();
        variant.kind = VARIANT_INT64;
        variant.value.int64 = value;
        variant
    }
    pub(super) fn boolean(value: bool) -> Self {
        let mut variant = Self::empty();
        variant.kind = VARIANT_BOOL;
        variant.value.boolean = u8::from(value);
        variant
    }
    pub(super) fn size(width: u32, height: u32) -> Self {
        let mut variant = Self::empty();
        variant.kind = VARIANT_SIZE;
        variant.value.size = AmfSize {
            width: width as i32,
            height: height as i32,
        };
        variant
    }
    pub(super) fn rate(num: u32, den: u32) -> Self {
        let mut variant = Self::empty();
        variant.kind = VARIANT_RATE;
        variant.value.rate = AmfRate { num, den };
        variant
    }
    /// The 64-bit integer, when it is one.
    pub(super) fn as_int64(&self) -> Option<i64> {
        // SAFETY: the tag says the union holds the integer.
        (self.kind == VARIANT_INT64).then(|| unsafe { self.value.int64 })
    }
    /// The boolean, when it is one.
    pub(super) fn as_bool(&self) -> Option<bool> {
        // SAFETY: the tag says the union holds an `amf_bool`.
        (self.kind == VARIANT_BOOL).then(|| unsafe { self.value.boolean } != 0)
    }
    /// The size, when it is one.
    pub(super) fn as_size(&self) -> Option<AmfSize> {
        // SAFETY: the tag says the union holds an `AMFSize`.
        (self.kind == VARIANT_SIZE).then(|| unsafe { self.value.size })
    }
    /// The rate, when it is one.
    pub(super) fn as_rate(&self) -> Option<AmfRate> {
        // SAFETY: the tag says the union holds an `AMFRate`.
        (self.kind == VARIANT_RATE).then(|| unsafe { self.value.rate })
    }
}

/// `AMFInterface` (`Interface.h:76-82`).
#[repr(C)]
pub(super) struct InterfaceVtbl {
    pub acquire: unsafe extern "system" fn(This) -> c_long,
    pub release: unsafe extern "system" fn(This) -> c_long,
    pub query_interface: unsafe extern "system" fn(This, *const AmfGuid, *mut This) -> AmfResult,
}

/// `AMFPropertyStorage` entries (`PropertyStorage.h`, C part).
#[repr(C)]
pub(super) struct StorageVtbl {
    pub set_property: unsafe extern "system" fn(This, *const u16, AmfVariant) -> AmfResult,
    pub get_property: unsafe extern "system" fn(This, *const u16, *mut AmfVariant) -> AmfResult,
    pub has_property: Ptr,
    pub get_property_count: Ptr,
    pub get_property_at: Ptr,
    pub clear: Ptr,
    pub add_to: Ptr,
    pub copy_to: Ptr,
    pub add_observer: Ptr,
    pub remove_observer: Ptr,
}

/// `AMFData` (`Data.h:128-164`): the prefix of `AMFSurface` and `AMFBuffer`.
#[repr(C)]
pub(super) struct DataVtbl {
    pub base: InterfaceVtbl,
    pub storage: StorageVtbl,
    pub get_memory_type: Ptr,
    pub duplicate: Ptr,
    pub convert: Ptr,
    pub interop: Ptr,
    pub get_data_type: Ptr,
    pub is_reusable: Ptr,
    pub set_pts: unsafe extern "system" fn(This, i64),
    pub get_pts: Ptr,
    pub set_duration: unsafe extern "system" fn(This, i64),
    pub get_duration: Ptr,
}

/// `AMFBuffer` (`Buffer.h:137-183`).
#[repr(C)]
pub(super) struct BufferVtbl {
    pub data: DataVtbl,
    pub set_size: Ptr,
    pub get_size: unsafe extern "system" fn(This) -> usize,
    pub get_native: unsafe extern "system" fn(This) -> *mut c_void,
    pub add_observer_buffer: Ptr,
    pub remove_observer_buffer: Ptr,
}

/// `AMFContext` (`Context.h:185-269`).
#[repr(C)]
pub(super) struct ContextVtbl {
    pub base: InterfaceVtbl,
    pub storage: StorageVtbl,
    pub terminate: unsafe extern "system" fn(This) -> AmfResult,
    pub dx9: [Ptr; 4],
    pub init_dx11: unsafe extern "system" fn(This, *mut c_void, i32) -> AmfResult,
    pub get_dx11_device: Ptr,
    pub lock_dx11: Ptr,
    pub unlock_dx11: Ptr,
    pub opencl: [Ptr; 8],
    pub opengl: [Ptr; 5],
    pub xv: [Ptr; 4],
    pub gralloc: [Ptr; 4],
    pub alloc_buffer: Ptr,
    pub alloc_surface: Ptr,
    pub alloc_audio_buffer: Ptr,
    pub create_buffer_from_host_native: Ptr,
    pub create_surface_from_host_native: Ptr,
    pub create_surface_from_dx9_native: Ptr,
    pub create_surface_from_dx11_native:
        unsafe extern "system" fn(This, *mut c_void, *mut This, *mut c_void) -> AmfResult,
    pub create_surface_from_opengl_native: Ptr,
    pub create_surface_from_gralloc_native: Ptr,
    pub create_surface_from_opencl_native: Ptr,
    pub create_buffer_from_opencl_native: Ptr,
    pub get_compute: Ptr,
}

/// `AMFComponent` (`Component.h:144-185`).
#[repr(C)]
pub(super) struct ComponentVtbl {
    pub base: InterfaceVtbl,
    pub storage: StorageVtbl,
    /// `AMFPropertyStorageEx`: `GetPropertiesInfoCount`, `GetPropertyInfoAt`,
    /// `GetPropertyInfo`, `ValidateProperty`.
    pub storage_ex: [Ptr; 4],
    pub init: unsafe extern "system" fn(This, i32, i32, i32) -> AmfResult,
    pub reinit: Ptr,
    pub terminate: unsafe extern "system" fn(This) -> AmfResult,
    pub drain: Ptr,
    pub flush: unsafe extern "system" fn(This) -> AmfResult,
    pub submit_input: unsafe extern "system" fn(This, This) -> AmfResult,
    pub query_output: unsafe extern "system" fn(This, *mut This) -> AmfResult,
    pub get_context: Ptr,
    pub set_output_data_allocator_cb: Ptr,
    pub get_caps: unsafe extern "system" fn(This, *mut This) -> AmfResult,
    pub optimize: Ptr,
}

/// `AMFFactory` (`Factory.h:70-79`; no `AMFInterface` entries).
#[repr(C)]
pub(super) struct FactoryVtbl {
    pub create_context: unsafe extern "system" fn(This, *mut This) -> AmfResult,
    pub create_component: unsafe extern "system" fn(This, This, *const u16, *mut This) -> AmfResult,
    pub set_cache_folder: Ptr,
    pub get_cache_folder: Ptr,
    pub get_debug: Ptr,
    pub get_trace: Ptr,
    pub get_programs: Ptr,
}

/// `AMFCaps` (`ComponentCaps.h:135-159`).
#[repr(C)]
pub(super) struct CapsVtbl {
    pub base: InterfaceVtbl,
    pub storage: StorageVtbl,
    pub get_acceleration_type: Ptr,
    pub get_input_caps: unsafe extern "system" fn(This, *mut This) -> AmfResult,
    pub get_output_caps: Ptr,
}

/// `AMFIOCaps` (`ComponentCaps.h:84-108`).
#[repr(C)]
pub(super) struct IoCapsVtbl {
    pub base: InterfaceVtbl,
    pub get_width_range: unsafe extern "system" fn(This, *mut i32, *mut i32),
    pub get_height_range: unsafe extern "system" fn(This, *mut i32, *mut i32),
    pub get_vert_align: Ptr,
    pub get_num_of_formats: Ptr,
    pub get_format_at: Ptr,
    pub get_num_of_memory_types: Ptr,
    pub get_memory_type_at: Ptr,
    pub is_interlaced_supported: Ptr,
}

/// The vtable of the AMF object at `this`.
///
/// # Safety
/// `this` is a live AMF object whose interface has the layout `V`.
pub(super) unsafe fn vtable<'a, V>(this: This) -> &'a V {
    // SAFETY: an AMF object starts with its vtable pointer (`pVtbl`).
    unsafe { &**(this as *const *const V) }
}

/// A property name as a NUL-terminated `wchar_t` string (UTF-16 on Windows).
pub(super) fn wide(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Encoder properties (`VideoEncoderVCE.h`, line numbers in brackets) and
/// their values.
pub(super) mod prop {
    pub const USAGE: &str = "Usage"; // [187]
    pub const PROFILE: &str = "Profile"; // [188]
    pub const PROFILE_LEVEL: &str = "ProfileLevel"; // [189]
    pub const MAX_LTR_FRAMES: &str = "MaxOfLTRFrames"; // [190]
    pub const LTR_MODE: &str = "LTRMode"; // [191]
    pub const FULL_RANGE_COLOR: &str = "FullRangeColor"; // [197]
    pub const LOWLATENCY_MODE: &str = "LowLatencyInternal"; // [198]
    pub const RATE_CONTROL_METHOD: &str = "RateControlMethod"; // [202]
    pub const QUALITY_PRESET: &str = "QualityPreset"; // [211]
    pub const INPUT_COLOR_PROFILE: &str = "InColorProfile"; // [216]
    pub const OUTPUT_COLOR_PROFILE: &str = "OutColorProfile"; // [221]
    pub const OUTPUT_TRANSFER: &str = "OutColorTransferChar"; // [222]
    pub const OUTPUT_PRIMARIES: &str = "OutColorPrimaries"; // [223]
    pub const FRAMESIZE: &str = "FrameSize"; // [184]
    pub const FRAMERATE: &str = "FrameRate"; // [232]
    pub const ENFORCE_HRD: &str = "EnforceHRD"; // [236]
    pub const FILLER_DATA_ENABLE: &str = "FillerDataEnable"; // [237]
    pub const VBV_BUFFER_SIZE: &str = "VBVBufferSize"; // [242]
    pub const INITIAL_VBV_FULLNESS: &str = "InitialVBVBufferFullness"; // [243]
    pub const TARGET_BITRATE: &str = "TargetBitrate"; // [252]
    pub const PEAK_BITRATE: &str = "PeakBitrate"; // [253]
    pub const SKIP_FRAME_ENABLE: &str = "RateControlSkipFrameEnable"; // [254]
    pub const B_PIC_PATTERN: &str = "BPicturesPattern"; // [258]
    pub const IDR_PERIOD: &str = "IDRPeriod"; // [261]
    pub const QUERY_TIMEOUT: &str = "QueryTimeout"; // [278]
    pub const FORCE_PICTURE_TYPE: &str = "ForcePictureType"; // [286]
    pub const INSERT_SPS: &str = "InsertSPS"; // [288]
    pub const INSERT_PPS: &str = "InsertPPS"; // [289]
    pub const MARK_LTR_INDEX: &str = "MarkCurrentWithLTRIndex"; // [291]
    pub const FORCE_LTR_BITFIELD: &str = "ForceLTRReferenceBitfield"; // [292]
    pub const OUTPUT_DATA_TYPE: &str = "OutputDataType"; // [302]

    /// `AMF_VIDEO_ENCODER_USAGE_ULTRA_LOW_LATENCY` [52].
    pub const USAGE_ULTRA_LOW_LATENCY: i64 = 1;
    /// `AMF_VIDEO_ENCODER_PROFILE_HIGH` [64].
    pub const PROFILE_HIGH: i64 = 100;
    /// `AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_CBR` [102].
    pub const RATE_CONTROL_CBR: i64 = 1;
    /// `AMF_VIDEO_ENCODER_QUALITY_PRESET_SPEED` [113].
    pub const QUALITY_SPEED: i64 = 1;
    /// `AMF_VIDEO_ENCODER_PICTURE_TYPE_IDR` [129].
    pub const PICTURE_TYPE_IDR: i64 = 2;
    /// `AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_IDR` [137].
    pub const OUTPUT_IDR: i64 = 0;
    /// `AMF_VIDEO_ENCODER_LTR_MODE_KEEP_UNUSED` [166].
    pub const LTR_KEEP_UNUSED: i64 = 1;
    /// `AMF_VIDEO_CONVERTER_COLOR_PROFILE_709` (studio range,
    /// `ColorSpace.h:47`), `AMF_COLOR_TRANSFER_CHARACTERISTIC_BT709`
    /// (`:83`), `AMF_COLOR_PRIMARIES_BT709` (`:64`).
    pub const COLOR_PROFILE_709: i64 = 1;
    pub const TRANSFER_BT709: i64 = 1;
    pub const PRIMARIES_BT709: i64 = 1;
    /// `InitialVBVBufferFullness` 64 = 100 % [243].
    pub const VBV_FULL: i64 = 64;
}
