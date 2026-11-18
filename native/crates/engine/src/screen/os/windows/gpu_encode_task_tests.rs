//! Plan 2026-11-17 P9 (GPU encode route): the low-latency settings of the
//! vendor encoders without look-ahead or pre-analysis, and sharp mode's
//! constant quality under the session rate (NVENC VBR with `targetQuality`,
//! AMF quality VBR with its level), checked without a GPU.
use super::amf_abi::{prop, AmfVariant};
use super::amf_codecs::{AV1, AVC, HEVC};
use super::amf_settings::static_settings;
use super::nvenc_encode::apply_settings;
use super::nvenc_ffi::{self as nv, Zeroed};
use super::vendor_encode::RateParams;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::model::SHARP_TARGET_QUALITY;

fn value(list: &[(&str, AmfVariant)], name: &str) -> AmfVariant {
    list.iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
        .unwrap_or_else(|| panic!("{name} missing"))
}

#[test]
fn gpu_encode_task_win_nvenc_sharp_mode_is_constant_quality_under_the_rate() {
    let cbr = RateParams::for_target(30_000_000, 120);
    let sharp = cbr.with_quality(Some(SHARP_TARGET_QUALITY));
    assert!(cbr.differs(&sharp), "a new quality is a new rate");
    let mut config = nv::NvEncConfig::zeroed();
    apply_settings(&mut config, &sharp);
    let rc = &config.rc_params;
    assert_eq!(rc.rate_control_mode, nv::NV_ENC_PARAMS_RC_VBR);
    assert_eq!(rc.target_quality, 18);
    assert_eq!((rc.average_bit_rate, rc.max_bit_rate), (0, 30_000_000));
    assert_eq!(rc.vbv_buffer_size, 250_000, "one frame of the ceiling");
    assert_eq!(rc.flags & nv::RC_ENABLE_LOOKAHEAD, 0, "no look-ahead");
    assert_eq!(config.frame_interval_p, 1, "no B pictures");
    // Back to CBR clears the quality target.
    apply_settings(&mut config, &cbr);
    let rc = &config.rc_params;
    assert_eq!(rc.rate_control_mode, nv::NV_ENC_PARAMS_RC_CBR);
    assert_eq!((rc.average_bit_rate, rc.target_quality), (30_000_000, 0));
}

#[test]
fn gpu_encode_task_win_amf_sharp_mode_and_no_pre_analysis() {
    let size = FrameSize::new(2880, 1800);
    let cbr = RateParams::for_target(40_000_000, 120);
    let sharp = cbr.with_quality(Some(SHARP_TARGET_QUALITY));
    for names in [&AVC, &HEVC, &AV1] {
        let settings = static_settings(names, size, &cbr);
        assert_eq!(
            value(&settings, names.rate_control).as_int64(),
            Some(names.cbr)
        );
        assert_eq!(value(&settings, names.pre_analysis).as_bool(), Some(false));
        assert!(settings.iter().all(|(key, _)| *key != names.qvbr_level));
        let settings = static_settings(names, size, &sharp);
        assert_eq!(settings[0].0, names.usage, "Usage first");
        assert_eq!(value(&settings, names.rate_control).as_int64(), Some(4));
        assert_eq!(value(&settings, names.qvbr_level).as_int64(), Some(18));
        assert_eq!(value(&settings, names.peak).as_int64(), Some(40_000_000));
    }
    // The VCE pre-encode switch is an enum, the HEVC and AV1 ones booleans.
    let avc = static_settings(&AVC, size, &cbr);
    assert_eq!(value(&avc, AVC.pre_encode).as_int64(), Some(0));
    assert_eq!(value(&avc, prop::B_PIC_PATTERN).as_int64(), Some(0));
    let hevc = static_settings(&HEVC, size, &cbr);
    assert_eq!(value(&hevc, HEVC.pre_encode).as_bool(), Some(false));
    assert_eq!(
        (AVC.qvbr_level, HEVC.qvbr_level, AV1.qvbr_level),
        (
            "QvbrQualityLevel",
            "HevcQvbrQualityLevel",
            "Av1QvbrQualityLevel"
        )
    );
}
