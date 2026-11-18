//! Task tests of the fix of 2026-11-18 (`nvEncOpenEncodeSessionEx: NVENC
//! status 15` on a Windows VM): the opening call matches `nvEncodeAPI.h`
//! 12.1 (nv-codec-headers `n12.1.14.0`, read again on 2026-11-18), and a
//! version refusal of the driver is named and reported once as "not
//! available".
use super::super::abi::{NvEncOpenSessionParams, Ptr};
use super::{
    status_name, NvEncFunctions, VersionRefusals, NVENCAPI_VERSION,
    NV_ENCODE_API_FUNCTION_LIST_VER, NV_ENC_ERR_INVALID_VERSION,
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER, NV_ENC_SUCCESS,
};
use crate::screen::core::negotiate::CodecKind;

#[test]
fn shadowhost_task_nvenc_open_session_layout_matches_header_12_1() {
    // NVENCAPI_VERSION = 12 | 1 << 24; NVENCAPI_STRUCT_VERSION(v) =
    // NVENCAPI_VERSION | v << 16 | 0x7 << 28.
    assert_eq!(NVENCAPI_VERSION, 0x0100_000C);
    assert_eq!(NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER, 0x7101_000C);
    assert_eq!(NV_ENCODE_API_FUNCTION_LIST_VER, 0x7102_000C);
    if std::mem::size_of::<Ptr>() == 8 {
        // version, deviceType, device, reserved, apiVersion,
        // reserved1[253], reserved2[64] on a 64-bit target.
        assert_eq!(std::mem::size_of::<NvEncOpenSessionParams>(), 1552);
        assert_eq!(
            std::mem::offset_of!(NvEncOpenSessionParams, api_version),
            24
        );
        assert_eq!(
            std::mem::offset_of!(NvEncOpenSessionParams, reserved2),
            1040
        );
        // Two u32, 43 entries and reserved2[275].
        assert_eq!(std::mem::size_of::<NvEncFunctions>(), 2552);
    }
}

#[test]
fn shadowhost_task_nvenc_version_refusal_blocks_only_its_codec_and_adapter() {
    assert_eq!(status_name(NV_ENC_SUCCESS), "NV_ENC_SUCCESS");
    assert_eq!(status_name(15), "NV_ENC_ERR_INVALID_VERSION");
    assert_eq!(status_name(2), "NV_ENC_ERR_UNSUPPORTED_DEVICE");
    assert_eq!(status_name(26), "NV_ENC_ERR_NEED_MORE_OUTPUT");
    assert_eq!(status_name(27), "unknown");
    assert_eq!(status_name(-1), "unknown");
    let (gpu, other) = (0x0000_0001_0000_7a3c_u64, 0x0000_0000_0000_d1f2_u64);
    let mut ledger = VersionRefusals::new();
    // The logged case: H.264 refused with NV_ENC_ERR_INVALID_VERSION.
    let reason = ledger
        .note(gpu, CodecKind::H264, NV_ENC_ERR_INVALID_VERSION)
        .unwrap();
    assert!(reason.contains("API 12.1"), "{reason}");
    assert!(reason.contains("NV_ENC_ERR_INVALID_VERSION"), "{reason}");
    assert_eq!(ledger.refused(gpu, CodecKind::H264), Some(reason));
    // HEVC and AV1 on the same adapter and H.264 on another stay tried.
    assert_eq!(ledger.refused(gpu, CodecKind::Hevc), None);
    assert_eq!(ledger.refused(gpu, CodecKind::Av1), None);
    assert_eq!(ledger.refused(other, CodecKind::H264), None);
    assert_eq!(ledger.note(gpu, CodecKind::Hevc, NV_ENC_SUCCESS), None);
    // A second refusal of the same pair is not logged again.
    assert_eq!(
        ledger.note(gpu, CodecKind::H264, NV_ENC_ERR_INVALID_VERSION),
        None
    );
    // After a session of a codec opened there, a refusal is an ordinary
    // error; other errors never block.
    assert_eq!(
        ledger.note(gpu, CodecKind::Hevc, NV_ENC_ERR_INVALID_VERSION),
        None
    );
    assert_eq!(ledger.refused(gpu, CodecKind::Hevc), None);
    assert_eq!(ledger.note(other, CodecKind::Av1, 2), None);
    assert_eq!(ledger.refused(other, CodecKind::Av1), None);
}
