//! NVENC structure layouts of both NVENC encoders (Linux BK4, Windows BK12;
//! one module since merge W2, plan 2026-11-14 „Bildkette“), from
//! `nvEncodeAPI.h` of NVENC API 12.1 as published in FFmpeg's
//! nv-codec-headers tag `n12.1.14.0` (read on 2026-11-14). The header uses
//! only fixed-width integers, enums and pointers (no `long`), so the sizes of
//! LP64 (Linux) and LLP64 (Windows x64) are the same; they are asserted by
//! `bildkette_task_linux_hw_nvenc_struct_sizes` and
//! `bildkette_task_win_vendor_nvenc_struct_sizes`. Every structure is only
//! ever written in its leading fields and otherwise zero, as the SDK asks
//! ("Reserved and must be set to 0"). The D3D11 input structures
//! (`NV_ENC_REGISTER_RESOURCE`, `NV_ENC_MAP_INPUT_RESOURCE`, header lines
//! 2529-2542 and 2613-2656) are used by Windows only.
// Every field of these C layouts exists for its offset, read or not.
#![allow(dead_code)]
use std::ffi::c_void;

/// `GUID`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::screen::os) struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub(in crate::screen::os) type Ptr = *mut c_void;

/// `NV_ENC_RC_PARAMS` (128 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncRcParams {
    pub version: u32,
    pub rate_control_mode: u32,
    pub const_qp: [u32; 3],
    pub average_bit_rate: u32,
    pub max_bit_rate: u32,
    pub vbv_buffer_size: u32,
    pub vbv_initial_delay: u32,
    /// `enableMinQP` 0 … `zeroReorderDelay` 9 … `enableExtLookahead` 16.
    pub flags: u32,
    pub min_qp: [u32; 3],
    pub max_qp: [u32; 3],
    pub initial_rc_qp: [u32; 3],
    pub temporal_layer_idx_mask: u32,
    pub temporal_layer_qp: [u8; 8],
    pub target_quality: u8,
    pub target_quality_lsb: u8,
    pub lookahead_depth: u16,
    pub low_delay_key_frame_scale: u8,
    pub y_dc_qp_index_offset: i8,
    pub u_dc_qp_index_offset: i8,
    pub v_dc_qp_index_offset: i8,
    pub qp_map_mode: u32,
    pub multi_pass: u32,
    pub alpha_layer_bitrate_ratio: u32,
    pub cb_qp_index_offset: i8,
    pub cr_qp_index_offset: i8,
    pub reserved2: u16,
    pub reserved: [u32; 4],
}

/// `NV_ENC_CONFIG_H264_VUI_PARAMETERS` (112 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncVui {
    pub overscan_info_present_flag: u32,
    pub overscan_info: u32,
    pub video_signal_type_present_flag: u32,
    pub video_format: u32,
    pub video_full_range_flag: u32,
    pub colour_description_present_flag: u32,
    pub colour_primaries: u32,
    pub transfer_characteristics: u32,
    pub colour_matrix: u32,
    pub chroma_sample_location_flag: u32,
    pub chroma_sample_location_top: u32,
    pub chroma_sample_location_bot: u32,
    pub bitstream_restriction_flag: u32,
    pub timing_info_present_flag: u32,
    pub num_unit_in_ticks: u32,
    pub time_scale: u32,
    pub reserved: [u32; 12],
}

/// `NV_ENC_CONFIG_H264` (1792 bytes, the largest member of
/// `NV_ENC_CODEC_CONFIG`; HEVC 1560 and AV1 1552 counted from the header).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncConfigH264 {
    /// `enableTemporalSVC` 0 … `repeatSPSPPS` 12 … `enableTimeCode` 21.
    pub flags: u32,
    pub level: u32,
    pub idr_period: u32,
    pub separate_colour_plane_flag: u32,
    pub disable_deblocking_filter_idc: u32,
    pub num_temporal_layers: u32,
    pub sps_id: u32,
    pub pps_id: u32,
    pub adaptive_transform_mode: u32,
    pub fmo_mode: u32,
    pub bdirect_mode: u32,
    pub entropy_coding_mode: u32,
    pub stereo_mode: u32,
    pub intra_refresh_period: u32,
    pub intra_refresh_cnt: u32,
    pub max_num_ref_frames: u32,
    pub slice_mode: u32,
    pub slice_mode_data: u32,
    pub vui: NvEncVui,
    pub ltr_num_frames: u32,
    pub ltr_trust_mode: u32,
    pub chroma_format_idc: u32,
    pub max_temporal_layers: u32,
    pub use_bframes_as_ref: u32,
    pub num_ref_l0: u32,
    pub num_ref_l1: u32,
    pub reserved1: [u32; 267],
    pub reserved2: [Ptr; 64],
}

/// `NV_ENC_CONFIG` (3584 bytes); the codec union holds the H.264 member.
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncConfig {
    pub version: u32,
    pub profile_guid: Guid,
    pub gop_length: u32,
    pub frame_interval_p: i32,
    pub mono_chrome_encoding: u32,
    pub frame_field_mode: u32,
    pub mv_precision: u32,
    pub rc_params: NvEncRcParams,
    pub h264: NvEncConfigH264,
    pub reserved: [u32; 278],
    pub reserved2: [Ptr; 64],
}

/// `NV_ENC_PRESET_CONFIG` (5128 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncPresetConfig {
    pub version: u32,
    pub preset_cfg: NvEncConfig,
    pub reserved1: [u32; 255],
    pub reserved2: [Ptr; 64],
}

/// `NV_ENC_INITIALIZE_PARAMS` (1808 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncInitializeParams {
    pub version: u32,
    pub encode_guid: Guid,
    pub preset_guid: Guid,
    pub encode_width: u32,
    pub encode_height: u32,
    pub dar_width: u32,
    pub dar_height: u32,
    pub frame_rate_num: u32,
    pub frame_rate_den: u32,
    pub enable_encode_async: u32,
    pub enable_ptd: u32,
    pub flags: u32,
    pub priv_data_size: u32,
    pub priv_data: Ptr,
    pub encode_config: *mut NvEncConfig,
    pub max_encode_width: u32,
    pub max_encode_height: u32,
    pub max_me_hint_counts_per_block: [[u32; 4]; 2],
    pub tuning_info: u32,
    pub buffer_format: u32,
    pub num_state_buffers: u32,
    pub output_stats_level: u32,
    pub reserved: [u32; 285],
    pub reserved2: [Ptr; 64],
}

/// `NV_ENC_RECONFIGURE_PARAMS` (1824 bytes); `flags`: `resetEncoder` 0,
/// `forceIDR` 1.
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncReconfigureParams {
    pub version: u32,
    pub re_init_encode_params: NvEncInitializeParams,
    pub flags: u32,
}

/// `NV_ENC_PIC_PARAMS` (3360 bytes); the codec union (1552 bytes, its AV1
/// member being the largest) stays zero for H.264 P/IDR pictures.
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncPicParams {
    pub version: u32,
    pub input_width: u32,
    pub input_height: u32,
    pub input_pitch: u32,
    pub encode_pic_flags: u32,
    pub frame_idx: u32,
    pub input_time_stamp: u64,
    pub input_duration: u64,
    pub input_buffer: Ptr,
    pub output_bitstream: Ptr,
    pub completion_event: Ptr,
    pub buffer_fmt: u32,
    pub picture_struct: u32,
    pub picture_type: u32,
    pub codec_pic_params: [u64; 194],
    pub me_hint_counts_per_block: [[u32; 4]; 2],
    pub me_external_hints: Ptr,
    pub reserved1: [u32; 6],
    pub reserved2: [Ptr; 2],
    pub qp_delta_map: Ptr,
    pub qp_delta_map_size: u32,
    pub reserved_bit_fields: u32,
    pub me_hint_ref_pic_dist: [u16; 2],
    pub alpha_buffer: Ptr,
    pub me_external_sb_hints: Ptr,
    pub me_sb_hints_count: u32,
    pub state_buffer_idx: u32,
    pub output_recon_buffer: Ptr,
    pub reserved3: [u32; 284],
    pub reserved4: [Ptr; 57],
}

/// `NV_ENC_LOCK_BITSTREAM` (1552 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncLockBitstream {
    pub version: u32,
    pub flags: u32,
    pub output_bitstream: Ptr,
    pub slice_offsets: *mut u32,
    pub frame_idx: u32,
    pub hw_encode_status: u32,
    pub num_slices: u32,
    pub bitstream_size_in_bytes: u32,
    pub output_time_stamp: u64,
    pub output_duration: u64,
    pub bitstream_buffer_ptr: Ptr,
    pub picture_type: u32,
    pub picture_struct: u32,
    pub frame_avg_qp: u32,
    pub frame_satd: u32,
    pub ltr_frame_idx: u32,
    pub ltr_frame_bitmap: u32,
    pub temporal_id: u32,
    pub intra_mb_count: u32,
    pub inter_mb_count: u32,
    pub average_mvx: i32,
    pub average_mvy: i32,
    pub alpha_layer_size_in_bytes: u32,
    pub output_stats_ptr_size: u32,
    pub output_stats_ptr: Ptr,
    pub frame_idx_display: u32,
    pub reserved1: [u32; 220],
    pub reserved2: [Ptr; 63],
    pub reserved_internal: [u32; 8],
}

/// `NV_ENC_CREATE_INPUT_BUFFER` (776 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncCreateInputBuffer {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub memory_heap: u32,
    pub buffer_fmt: u32,
    pub reserved: u32,
    pub input_buffer: Ptr,
    pub sys_mem_buffer: Ptr,
    pub reserved1: [u32; 57],
    pub reserved2: [Ptr; 63],
}

/// `NV_ENC_CREATE_BITSTREAM_BUFFER` (776 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncCreateBitstreamBuffer {
    pub version: u32,
    pub size: u32,
    pub memory_heap: u32,
    pub reserved: u32,
    pub bitstream_buffer: Ptr,
    pub bitstream_buffer_ptr: Ptr,
    pub reserved1: [u32; 58],
    pub reserved2: [Ptr; 64],
}

/// `NV_ENC_LOCK_INPUT_BUFFER` (1544 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncLockInputBuffer {
    pub version: u32,
    pub flags: u32,
    pub input_buffer: Ptr,
    pub buffer_data_ptr: Ptr,
    pub pitch: u32,
    pub reserved1: [u32; 251],
    pub reserved2: [Ptr; 64],
}

/// `NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS` (1552 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncOpenSessionParams {
    pub version: u32,
    pub device_type: u32,
    pub device: Ptr,
    pub reserved: Ptr,
    pub api_version: u32,
    pub reserved1: [u32; 253],
    pub reserved2: [Ptr; 64],
}

/// `NV_ENC_CAPS_PARAM` (256 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncCapsParam {
    pub version: u32,
    pub caps_to_query: u32,
    pub reserved: [u32; 62],
}

/// All-zero is a valid value of every NVENC structure (integers, raw
/// pointers, `Option` of function pointers), the start the SDK requires.
pub(in crate::screen::os) trait Zeroed: Copy + Sized {
    fn zeroed() -> Self {
        // SAFETY: implemented only for plain `repr(C)` NVENC structs.
        unsafe { std::mem::zeroed() }
    }
}
impl Zeroed for NvEncConfig {}
impl Zeroed for NvEncPresetConfig {}
impl Zeroed for NvEncInitializeParams {}
impl Zeroed for NvEncReconfigureParams {}
impl Zeroed for NvEncPicParams {}
impl Zeroed for NvEncLockBitstream {}
impl Zeroed for NvEncCreateInputBuffer {}
impl Zeroed for NvEncCreateBitstreamBuffer {}
impl Zeroed for NvEncLockInputBuffer {}
impl Zeroed for NvEncOpenSessionParams {}
impl Zeroed for NvEncCapsParam {}

/// `NV_ENC_REGISTER_RESOURCE` (1536 bytes, lines 2613-2653).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncRegisterResource {
    pub version: u32,
    pub resource_type: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub sub_resource_index: u32,
    pub resource_to_register: Ptr,
    pub registered_resource: Ptr,
    pub buffer_format: u32,
    pub buffer_usage: u32,
    pub input_fence_point: Ptr,
    pub chroma_offset: [u32; 2],
    pub reserved1: [u32; 245],
    pub reserved2: [Ptr; 61],
}

/// `NV_ENC_MAP_INPUT_RESOURCE` (1544 bytes, lines 2529-2539).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncMapInputResource {
    pub version: u32,
    pub sub_resource_index: u32,
    pub input_resource: Ptr,
    pub registered_resource: Ptr,
    pub mapped_resource: Ptr,
    pub mapped_buffer_fmt: u32,
    pub reserved1: [u32; 251],
    pub reserved2: [Ptr; 63],
}

/// `NV_ENC_EVENT_PARAMS` (1544 bytes, lines 2712-2719).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncEventParams {
    pub version: u32,
    pub reserved: u32,
    pub completion_event: Ptr,
    pub reserved1: [u32; 253],
    pub reserved2: [Ptr; 64],
}

impl Zeroed for NvEncRegisterResource {}
impl Zeroed for NvEncMapInputResource {}
impl Zeroed for NvEncEventParams {}
