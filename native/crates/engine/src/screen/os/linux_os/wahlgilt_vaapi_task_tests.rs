//! Rule „Wahl gilt“ (user 2026-11-18), finding 4: „Scharf (GPU)“ reaches
//! the Linux GPU encoders. NVENC opens VBR with `targetQuality` (as
//! Windows); VA-API opens QVBR with the quality factor where the driver lists
//! it and refuses the session with a named reason where it does not, instead
//! of encoding CBR silently.
use std::io::ErrorKind;

use super::super::hw_encode_common::RateParams;
use super::super::nvenc_encode::{admitted_quality, apply_codec_settings, SessionRate};
use super::super::nvenc_ffi::{NvEncConfig, Zeroed, NV_ENC_PARAMS_RC_CBR, NV_ENC_PARAMS_RC_VBR};
use super::super::vaapi_ffi::{VA_ATTRIB_NOT_SUPPORTED, VA_RC_CBR, VA_RC_QVBR};
use super::super::vaapi_h264_headers::misc_params;
use super::{rate_control, with_quality, SHARP_UNSUPPORTED};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::sharp_mode::SHARP_QUALITY;

#[test]
fn wahlgilt_task_linux_sharp_quality_reaches_the_gpu_encoders() {
    // VA-API: CBR without sharp mode, whatever the driver lists.
    assert_eq!(rate_control(None, 0).unwrap(), VA_RC_CBR);
    assert_eq!(
        rate_control(None, VA_ATTRIB_NOT_SUPPORTED).unwrap(),
        VA_RC_CBR
    );
    // Sharp mode: QVBR where listed, else a named refusal.
    assert_eq!(
        rate_control(Some(SHARP_QUALITY), VA_RC_CBR | VA_RC_QVBR).unwrap(),
        VA_RC_QVBR
    );
    for listed in [VA_RC_CBR, 0, VA_ATTRIB_NOT_SUPPORTED] {
        let error = rate_control(Some(SHARP_QUALITY), listed).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{listed:#x}");
        assert_eq!(error.to_string(), SHARP_UNSUPPORTED);
        assert!(SHARP_UNSUPPORTED.starts_with("sharp_quality_unsupported:"));
    }
    // The quality factor rides with the rate control; the rate is its peak.
    let rate = RateParams::for_target(20_000_000, 60);
    let (control, _, _) = misc_params(&rate, false);
    assert_eq!(with_quality(control, None), control);
    let sharp = with_quality(control, Some(SHARP_QUALITY));
    assert_eq!(sharp.quality_factor, u32::from(SHARP_QUALITY));
    assert_eq!(sharp.bits_per_second, 20_000_000);
    assert_eq!(with_quality(control, Some(0)).quality_factor, 1);
    assert_eq!(with_quality(control, Some(60)).quality_factor, 51);
    // NVENC: VBR with the target quality under the rate, CBR without.
    for codec in [CodecKind::H264, CodecKind::Hevc] {
        let mut config = NvEncConfig::zeroed();
        apply_codec_settings(&mut config, &SessionRate(&rate, Some(SHARP_QUALITY)), codec);
        assert_eq!(config.rc_params.rate_control_mode, NV_ENC_PARAMS_RC_VBR);
        assert_eq!(config.rc_params.target_quality, SHARP_QUALITY);
        assert_eq!(config.rc_params.max_bit_rate, 20_000_000);
        let mut plain = NvEncConfig::zeroed();
        apply_codec_settings(&mut plain, &SessionRate(&rate, None), codec);
        assert_eq!(plain.rc_params.rate_control_mode, NV_ENC_PARAMS_RC_CBR);
        assert_eq!(plain.rc_params.average_bit_rate, 20_000_000);
    }
    // Only an admitted session opens with it (as Windows' `vendor_codecs.rs`).
    assert_eq!(admitted_quality(None), None);
    // Every Linux GPU factory hands the quality to its session.
    let nvenc = include_str!("nvenc_encode.rs");
    assert!(nvenc.contains("NvencSession::open(size, fps, admitted_quality(grant.as_ref()))"));
    assert!(nvenc.contains("SessionRate(rate, self.quality)"));
    let codecs = include_str!("nvenc_codecs.rs");
    assert!(codecs.contains("NvencSession::open_codec(size, fps, self.codec, quality)"));
    for (name, text, call) in [
        (
            "vaapi_encode.rs",
            include_str!("vaapi_encode.rs"),
            "VaH264Session::open(display, caps, self.device.video_proc, size, quality)",
        ),
        (
            "vaapi_video_encode.rs",
            include_str!("vaapi_video_encode.rs"),
            "VaHevcSession::open(display, &self.caps, video_proc, size, quality)",
        ),
    ] {
        assert!(text.contains(call), "{name}");
        assert!(text.contains("session_quality()"), "{name}");
    }
}
