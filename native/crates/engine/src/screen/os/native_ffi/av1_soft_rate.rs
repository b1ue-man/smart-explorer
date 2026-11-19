//! A new bit rate for a running SVT-AV1 encoder (review 2026-11-19: a rate
//! change opened the whole encoder anew, the slide-show pattern), as
//! `Source/App/app_process_cmd.c` (`test_update_rate_info`,
//! `FTR_RATE_ON_FLY_SAMPLE`) does: an `EbPrivDataNode` of type
//! `RATE_CHANGE_EVENT` with an `SvtAv1RateInfo` hangs at the picture's
//! `p_app_private`.
//!
//! Layouts (`Source/API/EbSvtAv1.h`, tags v2.3.0, v4.0.0, v4.3.0):
//! `PrivDataType` counts `PRIVATE_DATA`, `REF_FRAME_SCALING_EVENT`,
//! `ROI_MAP_EVENT`, `RES_CHANGE_EVENT`, `RATE_CHANGE_EVENT` = 4 in every one
//! (newer types follow it); `EbPrivDataNode` is `{ node_type, data, size,
//! next }`; `SvtAv1RateInfo` is `{ uint32_t seq_qp; uint32_t
//! target_bit_rate; }` (bit/s, `seq_qp` 0 keeps the QP).
//!
//! Which pictures take it (`Source/Lib/Codec/resource_coordination_process.c`,
//! `update_rate_info`): 2.3.0 to 3.1.x apply it only on a key picture
//! (`if (input_ptr->pic_type == EB_AV1_KEY_PICTURE)`, read at v2.3.0, v3.0.0
//! and v3.1.0); 4.0.0 and later on any picture (no `pic_type` check, read at
//! v4.0.0 and master). So 4.x changes its rate without a key frame; the
//! older ones change it with the key frame they need, in the running
//! encoder instead of a new one.
//!
//! Memory (`Source/Lib/Globals/enc_handle.c`, `copy_private_data_list`,
//! v2.3.0 and v4.3.0, called by `svt_av1_enc_send_picture` through
//! `copy_input_buffer`): the library allocates its own nodes and copies
//! `size` bytes of every payload but `PRIVATE_DATA` and `ROI_MAP_EVENT`
//! before the call returns, and frees its copies itself. The node and the
//! rate here live on the caller's stack until `send` returns.
use std::os::raw::c_void;

/// `RATE_CHANGE_EVENT` of `PrivDataType`.
const RATE_CHANGE_EVENT: u32 = 4;

#[repr(C)]
pub(super) struct SvtAv1RateInfo {
    seq_qp: u32,
    target_bit_rate: u32,
}

#[repr(C)]
pub(super) struct EbPrivDataNode {
    node_type: u32,
    data: *mut c_void,
    size: u32,
    next: *mut EbPrivDataNode,
}

impl SvtAv1RateInfo {
    /// `bits_per_second` within the range of the opening setting (`tbr`).
    pub(super) fn new(bits_per_second: u64) -> Self {
        Self {
            seq_qp: 0,
            target_bit_rate: bits_per_second.clamp(1000, 100_000_000) as u32,
        }
    }
}

impl EbPrivDataNode {
    /// The node carrying `info`; both must outlive the `send` call.
    pub(super) fn rate(info: &mut SvtAv1RateInfo) -> Self {
        Self {
            node_type: RATE_CHANGE_EVENT,
            data: (info as *mut SvtAv1RateInfo).cast(),
            size: std::mem::size_of::<SvtAv1RateInfo>() as u32,
            next: std::ptr::null_mut(),
        }
    }
}

/// Whether SVT-AV1 `version` ("v4.0.0") takes a new rate on any picture
/// (4.0.0 and later); the older ones take it on a key picture only.
pub(super) fn rate_on_any_picture(version: &str) -> bool {
    version
        .trim()
        .trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= 4)
}
