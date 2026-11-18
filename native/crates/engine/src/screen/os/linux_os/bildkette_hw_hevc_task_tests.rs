//! BK4b task tests (`bildkette_task_linux_hw_hevc_*`): the VA-API HEVC
//! encoder and the registration without hardware. The program's VPS/SPS/PPS
//! and slice headers through its own parser and the core's check, the
//! parameter mapping of the encoder, the C structure sizes counted from
//! libva 2.20.0, the probe and the order and report of the VA-API HEVC/AV1
//! entries. The decoder's tests are in `bildkette_hw_hevc_decode_task_tests`.
use std::collections::BTreeMap;
use std::mem::{offset_of, size_of};
use std::path::PathBuf;
use std::sync::Arc;

use super::h264_bits::{nal_units, unescape};
use super::hw_encode_common::RateParams;
use super::vaapi_ffi::{self as va, VA_ATTRIB_NOT_SUPPORTED};
use super::vaapi_hevc_abi as abi;
use super::vaapi_hevc_encode::hevc_level_idc;
use super::vaapi_hevc_headers::{
    parameter_sets, slice_header, vps, HevcLayout, HevcPicture, HevcTools,
};
use super::vaapi_hevc_slice::{parse_slice, SLICE_B, SLICE_I, SLICE_P};
use super::vaapi_hevc_syntax::{nal_header, parse_pps, parse_sps, Pps, Sps};
use super::vaapi_probe::{VaDevice, VaVideoDecodeCaps, VaVideoEncodeCaps};
use super::vaapi_video_probe::{choose_video_encode, decode_from, VALUES};
use crate::screen::core::codec_registry::{CodecRegistry, OpenH264Report, OpenH264State};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::frame_size_caps::{
    check_video_dimensions, video_keyframe, video_parameter_sets,
};
use crate::screen::core::negotiate::{negotiate, CodecKind, CodecOffer, CodecPreference};
use crate::screen::core::pixel_convert::FrameSize;

pub(super) const SIZE: FrameSize = FrameSize::new(1920, 1080);

pub(super) fn tools() -> HevcTools {
    HevcTools::new(VA_ATTRIB_NOT_SUPPORTED, VA_ATTRIB_NOT_SUPPORTED)
}

/// The SPS and PPS of `units` by id.
pub(super) fn maps(units: &[&[u8]]) -> (BTreeMap<u32, Sps>, BTreeMap<u32, Pps>) {
    let (mut sps_map, mut pps_map) = (BTreeMap::new(), BTreeMap::new());
    for unit in units {
        match nal_header(unit).map(|(kind, _)| kind) {
            Some(33) => {
                let sps = parse_sps(unit).unwrap();
                sps_map.insert(sps.id, sps);
            }
            Some(34) => {
                let pps = parse_pps(unit).unwrap();
                pps_map.insert(pps.id, pps);
            }
            _ => {}
        }
    }
    (sps_map, pps_map)
}

pub(super) fn picture(idr: bool, poc: u32) -> HevcPicture {
    HevcPicture {
        idr,
        poc,
        recon: 11,
        reference: (!idr).then_some(10),
        coded: 99,
    }
}

/// A VA device with HEVC and AV1 in both directions (no H.264).
pub(super) fn video_device() -> Arc<VaDevice> {
    let encode = |codec, va_profile| VaVideoEncodeCaps {
        codec,
        va_profile,
        entrypoint: va::VA_ENTRYPOINT_ENC_SLICE_LP,
        packed: 0x0f,
        max: FrameSize::new(4096, 2304),
        features: VA_ATTRIB_NOT_SUPPORTED,
        blocks: VA_ATTRIB_NOT_SUPPORTED,
        ext2: VA_ATTRIB_NOT_SUPPORTED,
        prediction: VA_ATTRIB_NOT_SUPPORTED,
    };
    let decode = |codec, va_profile| VaVideoDecodeCaps {
        codec,
        va_profile,
        max: FrameSize::new(4096, 2304),
    };
    Arc::new(VaDevice {
        node: PathBuf::from("/nonexistent-bk4b/renderD128"),
        encode: Vec::new(),
        decode: None,
        video_proc: true,
        video_encode: vec![
            encode(CodecKind::Hevc, va::VA_PROFILE_HEVC_MAIN),
            encode(CodecKind::Av1, va::VA_PROFILE_AV1_PROFILE0),
        ],
        video_decode: vec![
            decode(CodecKind::Hevc, va::VA_PROFILE_HEVC_MAIN),
            decode(CodecKind::Av1, va::VA_PROFILE_AV1_PROFILE0),
        ],
    })
}

#[test]
fn bildkette_task_linux_hw_hevc_struct_sizes() {
    assert_eq!(size_of::<abi::VaPictureHevc>(), 28);
    assert_eq!(size_of::<abi::VaEncSequenceHevc>(), 116);
    assert_eq!(offset_of!(abi::VaEncSequenceHevc, seq_fields), 24);
    assert_eq!(
        offset_of!(abi::VaEncSequenceHevc, pcm_sample_bit_depth_luma_minus1),
        36
    );
    assert_eq!(offset_of!(abi::VaEncSequenceHevc, vui_fields), 56);
    assert_eq!(
        offset_of!(abi::VaEncSequenceHevc, min_spatial_segmentation_idc),
        80
    );
    assert_eq!(offset_of!(abi::VaEncSequenceHevc, scc_fields), 84);
    assert_eq!(size_of::<abi::VaEncPictureHevc>(), 576);
    assert_eq!(offset_of!(abi::VaEncPictureHevc, coded_buf), 448);
    assert_eq!(offset_of!(abi::VaEncPictureHevc, row_height_minus1), 479);
    assert_eq!(offset_of!(abi::VaEncPictureHevc, nal_unit_type), 505);
    assert_eq!(offset_of!(abi::VaEncPictureHevc, pic_fields), 508);
    assert_eq!(offset_of!(abi::VaEncPictureHevc, scc_fields), 514);
    assert_eq!(size_of::<abi::VaEncSliceHevc>(), 1076);
    assert_eq!(offset_of!(abi::VaEncSliceHevc, ref_pic_list1), 432);
    assert_eq!(offset_of!(abi::VaEncSliceHevc, max_num_merge_cand), 1034);
    assert_eq!(offset_of!(abi::VaEncSliceHevc, slice_fields), 1040);
    assert_eq!(size_of::<abi::VaPictureParameterHevc>(), 604);
    assert_eq!(offset_of!(abi::VaPictureParameterHevc, pic_fields), 452);
    assert_eq!(
        offset_of!(abi::VaPictureParameterHevc, column_width_minus1),
        476
    );
    assert_eq!(
        offset_of!(abi::VaPictureParameterHevc, slice_parsing_fields),
        556
    );
    assert_eq!(offset_of!(abi::VaPictureParameterHevc, st_rps_bits), 568);
    assert_eq!(size_of::<abi::VaSliceParameterHevc>(), 264);
    assert_eq!(offset_of!(abi::VaSliceParameterHevc, long_slice_flags), 52);
    assert_eq!(
        offset_of!(abi::VaSliceParameterHevc, collocated_ref_idx),
        56
    );
    assert_eq!(
        offset_of!(abi::VaSliceParameterHevc, five_minus_max_num_merge_cand),
        246
    );
    assert_eq!(
        offset_of!(abi::VaSliceParameterHevc, num_entry_point_offsets),
        248
    );
    assert_eq!(
        offset_of!(abi::VaSliceParameterHevc, slice_data_num_emu_prevn_bytes),
        252
    );
}

#[test]
fn bildkette_task_linux_hw_hevc_parameter_sets_roundtrip() {
    let layout = HevcLayout::new(SIZE, VA_ATTRIB_NOT_SUPPORTED);
    assert_eq!(layout.coded(), FrameSize::new(1920, 1088));
    assert_eq!(layout.surface(), FrameSize::new(1920, 1088));
    let tools = tools();
    let level = hevc_level_idc(layout.coded(), 60);
    let sets = parameter_sets(&layout, &tools, level);
    let units = nal_units(&sets);
    let kinds: Vec<_> = units.iter().map(|u| nal_header(u).unwrap().0).collect();
    assert_eq!(kinds, [32, 33, 34]);
    assert_eq!(units[0], &vps(level)[4..]);
    let parsed = parse_sps(units[1]).unwrap();
    assert_eq!((parsed.width, parsed.height), (1920, 1088));
    assert_eq!((parsed.log2_min_cb, parsed.ctb_log2()), (4, 5));
    assert_eq!((parsed.log2_min_tb, parsed.log2_diff_max_min_tb), (2, 3));
    assert_eq!(
        (parsed.max_dec_pic_buffering, parsed.num_reorder_pics),
        (2, 0)
    );
    assert_eq!(parsed.log2_max_poc_lsb, 16);
    assert!(parsed.amp && parsed.sao && parsed.temporal_mvp);
    assert!(parsed.st_rps.is_empty() && !parsed.long_term_present);
    let parsed_pps = parse_pps(units[2]).unwrap();
    assert_eq!(
        (
            parsed_pps.num_ref_idx_l0_default,
            parsed_pps.num_ref_idx_l1_default
        ),
        (1, 1)
    );
    assert_eq!(parsed_pps.cu_qp_delta, Some(0));
    assert!(parsed_pps.tiles.is_none() && !parsed_pps.entropy_coding_sync);
    // An IDR access unit: the core's mark, parameter sets and SPS check.
    let mut access_unit = sets.clone();
    access_unit.extend_from_slice(&slice_header(&tools, &picture(true, 0)));
    assert!(video_keyframe(CodecKind::Hevc, &access_unit));
    assert_eq!(
        video_parameter_sets(CodecKind::Hevc, &access_unit),
        Some(sets)
    );
    check_video_dimensions(CodecKind::Hevc, &access_unit, SIZE, FrameLimits::RESOURCE).unwrap();
    let other = check_video_dimensions(
        CodecKind::Hevc,
        &access_unit,
        FrameSize::new(1280, 720),
        FrameLimits::RESOURCE,
    );
    assert!(other.is_err(), "the conformance window names 1920x1080");
    let p_unit = slice_header(&tools, &picture(false, 1));
    assert!(!video_keyframe(CodecKind::Hevc, &p_unit));
    // Odd sizes: whole minimum coding blocks, cropped back.
    let small = HevcLayout::new(FrameSize::new(1366, 768), VA_ATTRIB_NOT_SUPPORTED);
    assert_eq!(small.coded(), FrameSize::new(1376, 768));
    assert_eq!(small.crop(), (5, 0));
}

#[test]
fn bildkette_task_linux_hw_hevc_slice_header_roundtrip() {
    let layout = HevcLayout::new(SIZE, VA_ATTRIB_NOT_SUPPORTED);
    let gpb = HevcTools::new(
        VA_ATTRIB_NOT_SUPPORTED,
        va::VA_PREDICTION_DIRECTION_BI_NOT_EMPTY,
    );
    for tools in [tools(), gpb] {
        let sets = parameter_sets(&layout, &tools, 123);
        let (sps_map, pps_map) = maps(&nal_units(&sets));
        for (idr, poc, kind) in [(true, 0, SLICE_I), (false, 70_000, SLICE_P)] {
            let kind = if !idr && tools.gpb { SLICE_B } else { kind };
            let written = slice_header(&tools, &picture(idr, poc));
            let unit = nal_units(&written)[0];
            let nal_type = nal_header(unit).unwrap().0;
            assert_eq!(nal_type, if idr { 19 } else { 1 });
            let h = parse_slice(unit, nal_type, &sps_map, &pps_map, None).unwrap();
            assert_eq!(h.slice_type, kind);
            assert!(h.first && !h.dependent);
            assert_eq!(
                h.header_bytes,
                unescape(&unit[2..]).len() + 2,
                "ends aligned"
            );
            if idr {
                assert!(h.idr() && h.rps.count() == 0);
            } else {
                assert_eq!(h.poc_lsb, 70_000 & 0xffff);
                assert_eq!(h.rps.negative, [(-1, true)]);
                assert!(h.temporal_mvp && h.collocated_from_l0);
                let lists = if tools.gpb { [1, 1] } else { [1, 0] };
                assert_eq!(h.num_ref_idx, lists);
                assert_eq!(h.st_rps_bits, 6, "ue(1) 3 bits, ue(0) twice, one flag");
            }
            assert!(h.sao_luma && h.sao_chroma);
        }
    }
}

#[test]
fn bildkette_task_linux_hw_hevc_parameter_mapping() {
    let layout = HevcLayout::new(SIZE, VA_ATTRIB_NOT_SUPPORTED);
    let tools = tools();
    let rate = RateParams::for_target(8_000_000, 60);
    let seq = super::vaapi_hevc_params::sequence_params(&layout, &tools, 123, &rate);
    assert_eq!((seq.general_profile_idc, seq.general_level_idc), (1, 123));
    assert_eq!(
        (
            seq.pic_width_in_luma_samples,
            seq.pic_height_in_luma_samples
        ),
        (1920, 1088)
    );
    assert_eq!(seq.ip_period, 1);
    assert_eq!(seq.bits_per_second, 8_000_000);
    // chroma 4:2:0, AMP, SAO, temporal MVP, low delay.
    assert_eq!(
        seq.seq_fields,
        1 | (1 << 11) | (1 << 12) | (1 << 15) | (1 << 16)
    );
    let p = picture(false, 5);
    let pic = super::vaapi_hevc_params::picture_params(&tools, &p);
    assert_eq!(pic.decoded_curr_pic.pic_order_cnt, 5);
    assert_eq!(pic.reference_frames[0].picture_id, 10);
    assert_eq!(pic.reference_frames[0].pic_order_cnt, 4);
    assert_eq!(pic.reference_frames[1].flags, abi::VA_PICTURE_HEVC_INVALID);
    assert_eq!((pic.collocated_ref_pic_index, pic.nal_unit_type), (0, 1));
    assert_eq!(pic.pic_fields & 0xf, 2 << 1, "not IDR, coding type P");
    let idr = super::vaapi_hevc_params::picture_params(&tools, &picture(true, 0));
    assert_eq!(
        (idr.collocated_ref_pic_index, idr.nal_unit_type),
        (0xff, 19)
    );
    assert_eq!(idr.pic_fields & 0xf, 1 | (1 << 1));
    let slice = super::vaapi_hevc_params::slice_params(&layout, &tools, &p);
    assert_eq!(slice.num_ctu_in_slice, 60 * 34);
    assert_eq!((slice.slice_type, slice.max_num_merge_cand), (SLICE_P, 5));
    assert_eq!(slice.ref_pic_list1[0].flags, abi::VA_PICTURE_HEVC_INVALID);
    let gpb = HevcTools::new(VA_ATTRIB_NOT_SUPPORTED, 0x5);
    let slice = super::vaapi_hevc_params::slice_params(&layout, &gpb, &p);
    assert_eq!(slice.slice_type, SLICE_B);
    assert_eq!(
        slice.ref_pic_list1[0].picture_id, 10,
        "low-delay B: list 1 = list 0"
    );
    // Driver block sizes: CTB 64, min CB 8, TB 4…32, depth 2/2.
    let blocks = 3 | (2 << 6) | (2 << 10) | (2 << 14);
    let lp = HevcLayout::new(SIZE, blocks);
    assert_eq!((lp.ctb_log2, lp.min_cb_log2, lp.max_tb_log2), (6, 3, 4));
    assert_eq!(lp.surface(), FrameSize::new(1920, 1088));
    assert_eq!(lp.ctus(), 30 * 17);
    // Features: AMP unsupported, SAO supported, sign hiding required.
    let features = (1 << 6) | (2 << 16) | (1 << 22);
    let picked = HevcTools::new(features, VA_ATTRIB_NOT_SUPPORTED);
    assert!(!picked.amp && picked.sao && picked.sign_data_hiding && picked.cu_qp_delta);
    assert!(!picked.temporal_mvp && !picked.transform_skip && !picked.gpb);
    assert!(
        hevc_level_idc(FrameSize::new(3840, 2160), 60) >= 153,
        "level 5.1"
    );
}

#[test]
fn bildkette_task_linux_hw_hevc_probe_and_rank() {
    let full = |packed: u32| {
        let mut values = [VA_ATTRIB_NOT_SUPPORTED; VALUES];
        values[0] = va::VA_RT_FORMAT_YUV420;
        values[1] = va::VA_RC_CBR;
        values[2] = packed;
        values
    };
    let candidates = [
        (va::VA_ENTRYPOINT_ENC_SLICE, full(0x0d)),
        (va::VA_ENTRYPOINT_ENC_SLICE_LP, full(0x05)),
    ];
    let caps = choose_video_encode(CodecKind::Hevc, va::VA_PROFILE_HEVC_MAIN, &candidates).unwrap();
    assert_eq!(
        caps.entrypoint,
        va::VA_ENTRYPOINT_ENC_SLICE_LP,
        "low power first"
    );
    assert_eq!((caps.packed, caps.max), (0x05, FrameSize::MAX));
    let mut required_lists = full(0x05);
    required_lists[5] = 2 << 2; // scaling_lists: VA_FEATURE_REQUIRED
    let refused = [(va::VA_ENTRYPOINT_ENC_SLICE_LP, required_lists)];
    assert!(choose_video_encode(CodecKind::Hevc, 17, &refused).is_none());
    assert!(choose_video_encode(CodecKind::Av1, 32, &refused).is_some());
    let mut no_cbr = full(0);
    no_cbr[1] = 0x10;
    assert!(choose_video_encode(CodecKind::Hevc, 17, &[(8, no_cbr)]).is_none());
    assert!(
        decode_from(CodecKind::Hevc, 17, [0x4, 4096, 4096]).is_none(),
        "no 4:2:0"
    );
    let decode = decode_from(CodecKind::Av1, 32, [1, VA_ATTRIB_NOT_SUPPORTED, 2160]).unwrap();
    assert_eq!(decode.max, FrameSize::new(4096, 2160));
    // Order and report: VA-API HEVC/AV1 per node, hardware, profile main.
    let (encoders, decoders) = super::video_ordered(&[video_device()]);
    let names: Vec<_> = encoders.iter().map(|f| f.name()).collect();
    assert_eq!(names, ["vaapi-hevc-renderD128", "vaapi-av1-renderD128"]);
    let names: Vec<_> = decoders.iter().map(|f| f.name()).collect();
    assert_eq!(names, ["vaapi-hevc-renderD128", "vaapi-av1-renderD128"]);
    let registry = CodecRegistry::new(encoders, decoders);
    let report = registry.report(OpenH264Report::new(OpenH264State::Absent, None));
    let json = serde_json::to_value(&report).unwrap();
    for list in ["encoders", "decoders"] {
        let rows = json[list].as_array().unwrap();
        assert_eq!(rows.len(), 3, "{rows:?}");
        for (row, codec) in rows.iter().zip(["hevc", "av1"]) {
            assert_eq!(row["codec"], codec);
            assert_eq!(row["hardware"], true);
            assert_eq!(row["profile_names"], serde_json::json!(["main"]));
        }
    }
    // Hardware on both sides: HEVC before AV1 (P9 2026-11-17: AV1 never first).
    let viewer = [
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::video(CodecKind::Av1, false, true),
    ];
    let choices = negotiate(&registry.encoder_offers(), &viewer, CodecPreference::Auto);
    assert_eq!(choices[0].codec, CodecKind::Hevc);
    assert_eq!(choices[1].codec, CodecKind::Av1);
    // Without a device nothing changes; opening checks before the device.
    let (encoders, decoders) = super::video_ordered(&[]);
    assert!(encoders.is_empty() && decoders.is_empty());
    let registry = CodecRegistry::new(super::video_ordered(&[video_device()]).0, Vec::new());
    let error = registry
        .open_encoder(&choices[0], FrameSize::new(1281, 720), 60)
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("vaapi-hevc-renderD128"),
        "{error}"
    );
}
