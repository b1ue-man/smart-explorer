//! BK4b task tests of the VA-API HEVC decoder (`bildkette_task_linux_hw_hevc_*`,
//! without hardware): POC, reference picture sets and lists of the
//! program's own stream, the decode parameter words, the emulation
//! prevention count, and the refusal of every tool outside the screen
//! contract (profiles, bit depth, scaling lists, weighted prediction,
//! long-term references, leading pictures, references to later pictures).
use std::collections::BTreeMap;
use std::io::ErrorKind;

use super::bildkette_hw_hevc_task_tests::{maps, picture, tools, SIZE};
use super::h264_bits::{nal_units, BitWriter};
use super::vaapi_ffi::VA_ATTRIB_NOT_SUPPORTED;
use super::vaapi_hevc_abi as abi;
use super::vaapi_hevc_decode_params::{emulation_bytes_before, picture_params, slice_params};
use super::vaapi_hevc_dpb::{check_nal_type, ref_lists, Dpb};
use super::vaapi_hevc_headers::{parameter_sets, pps, slice_header, HevcLayout};
use super::vaapi_hevc_slice::{parse_slice, SliceHeader, SLICE_I};
use super::vaapi_hevc_syntax::{nal_header, parse_pps, parse_sps, ShortTermRps};

#[test]
fn bildkette_task_linux_hw_hevc_decode_rps_and_lists() {
    let layout = HevcLayout::new(SIZE, VA_ATTRIB_NOT_SUPPORTED);
    let tools = tools();
    let (sps_map, pps_map) = maps(&nal_units(&parameter_sets(&layout, &tools, 123)));
    let sps = &sps_map[&0];
    let header = |idr: bool, poc: u32| {
        let written = slice_header(&tools, &picture(idr, poc));
        let unit = nal_units(&written)[0].to_vec();
        let nal_type = nal_header(&unit).unwrap().0;
        parse_slice(&unit, nal_type, &sps_map, &pps_map, None).unwrap()
    };
    let mut dpb = Dpb::default();
    let first = header(true, 0);
    let poc = dpb.poc(&first, sps, 0).unwrap();
    let refs = dpb.apply_rps(&first, poc).unwrap();
    assert_eq!((poc, refs.frames.len()), (0, 0));
    dpb.insert(100, poc, sps).unwrap();
    for index in 1..5u32 {
        let h = header(false, index);
        let poc = dpb.poc(&h, sps, 0).unwrap();
        assert_eq!(poc, index as i32);
        let refs = dpb.apply_rps(&h, poc).unwrap();
        assert_eq!(refs.frames.len(), 1, "only the previous picture stays");
        assert_eq!(refs.frames[0].pic_order_cnt, poc - 1);
        assert_eq!(
            refs.frames[0].flags,
            abi::VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE
        );
        let lists = ref_lists(&h, &refs).unwrap();
        assert_eq!(lists[0][..2], [0, abi::HEVC_REF_INVALID]);
        assert_eq!(lists[1][0], abi::HEVC_REF_INVALID);
        dpb.insert(100 + index, poc, sps).unwrap();
    }
    // POC LSB wrap (8.3.1): 30000, 60000, then LSB 5 counts on to 65541.
    let mut wrap = Dpb::default();
    wrap.poc(&first, sps, 0).unwrap();
    for (lsb, expected) in [(30_000, 30_000), (60_000, 60_000), (5, 65_541)] {
        let h = SliceHeader {
            poc_lsb: lsb,
            ..header(false, 1)
        };
        assert_eq!(wrap.poc(&h, sps, 0).unwrap(), expected);
    }
    // A reference to a later picture reorders: refused.
    let mut reorder = header(false, 2);
    reorder.rps = ShortTermRps {
        negative: Vec::new(),
        positive: vec![(1, true)],
    };
    let mut dpb = Dpb::default();
    dpb.poc(&first, sps, 0).unwrap();
    dpb.insert(1, 3, sps).unwrap();
    let refused = dpb.apply_rps(&reorder, 2).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::Unsupported);
    // A missing current reference is an error, not a wrong picture.
    let mut empty = Dpb::default();
    assert!(empty.apply_rps(&header(false, 1), 1).is_err());
    // Leading and broken-link pictures are outside the contract.
    for nal_type in [6, 8, 16] {
        assert_eq!(
            check_nal_type(nal_type).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
    }
    for nal_type in [0, 1, 19, 20, 21] {
        check_nal_type(nal_type).unwrap();
    }
}

#[test]
fn bildkette_task_linux_hw_hevc_decode_parameter_words() {
    let layout = HevcLayout::new(SIZE, VA_ATTRIB_NOT_SUPPORTED);
    let tools = tools();
    let (sps_map, pps_map) = maps(&nal_units(&parameter_sets(&layout, &tools, 123)));
    let (sps, pps) = (&sps_map[&0], &pps_map[&0]);
    let written = slice_header(&tools, &picture(true, 0));
    let unit = nal_units(&written)[0];
    let h = parse_slice(unit, 19, &sps_map, &pps_map, None).unwrap();
    let mut dpb = Dpb::default();
    let refs = dpb.apply_rps(&h, 0).unwrap();
    let p = picture_params(sps, pps, &h, &refs, 7).unwrap();
    assert_eq!((p.curr_pic.picture_id, p.curr_pic.pic_order_cnt), (7, 0));
    assert_eq!(
        (p.pic_width_in_luma_samples, p.pic_height_in_luma_samples),
        (1920, 1088)
    );
    // chroma, AMP, cu_qp_delta, NoPicReorderingFlag.
    assert_eq!(p.pic_fields, 1 | (1 << 6) | (1 << 10) | (1 << 19));
    // temporal MVP, SAO, RapPic, IdrPic, IntraPic.
    assert_eq!(
        p.slice_parsing_fields,
        (1 << 2) | (1 << 7) | (1 << 11) | (1 << 12) | (1 << 13)
    );
    assert_eq!(p.sps_max_dec_pic_buffering_minus1, 1);
    assert_eq!(p.log2_max_pic_order_cnt_lsb_minus4, 12);
    let s = slice_params(&h, &refs, unit, true).unwrap();
    assert_eq!(s.slice_data_size as usize, unit.len());
    assert_eq!(s.slice_data_byte_offset as usize, h.header_bytes);
    assert_eq!(s.long_slice_flags & 0xf, 1 | (SLICE_I as u32) << 2);
    assert_eq!(s.collocated_ref_idx, 0xff);
    // Emulation prevention bytes inside a header are counted.
    let escaped = [0x26, 0x01, 0x00, 0x00, 0x03, 0x01, 0x80];
    assert_eq!(emulation_bytes_before(&escaped, 3), 1);
    assert_eq!(emulation_bytes_before(&escaped, 2), 0);
}

/// An SPS with the knobs the refusal test turns (H.265 7.3.2.2).
fn crafted_sps(profile: u32, bit_depth_minus8: u32, scaling_lists: bool) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.bits(4, 0);
    w.bits(3, 0);
    w.flag(true);
    w.bits(2, 0);
    w.flag(false);
    w.bits(5, profile);
    w.bits(32, 1 << (31 - profile));
    w.bits(4, 0b1001);
    w.bits(32, 0);
    w.bits(12, 0);
    w.bits(8, 123);
    w.ue(0);
    w.ue(1);
    w.ue(1920);
    w.ue(1080);
    w.flag(false);
    w.ue(bit_depth_minus8);
    w.ue(bit_depth_minus8);
    w.ue(4);
    w.flag(true);
    w.ue(1);
    w.ue(0);
    w.ue(0);
    for value in [0, 2, 0, 3, 2, 2] {
        w.ue(value);
    }
    w.flag(scaling_lists);
    w.flag(false);
    w.flag(false);
    w.flag(false);
    w.ue(0);
    w.flag(false);
    w.flag(false);
    w.flag(false);
    w.flag(false);
    w.flag(false);
    super::h264_bits::annex_b_unit(&[33 << 1, 1], &w.finish())[4..].to_vec()
}

#[test]
fn bildkette_task_linux_hw_hevc_refuses_unsupported_tools() {
    parse_sps(&crafted_sps(1, 0, false)).expect("plain Main parses");
    for (sps, what) in [
        (crafted_sps(4, 0, false), "range extensions profile"),
        (crafted_sps(1, 2, false), "10 bit"),
        (crafted_sps(1, 0, true), "scaling lists"),
    ] {
        let error = parse_sps(&sps).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{what}");
    }
    // A PPS with weighted prediction.
    let mut w = BitWriter::default();
    w.ue(0);
    w.ue(0);
    w.flag(false);
    w.flag(false);
    w.bits(3, 0);
    w.flag(false);
    w.flag(false);
    w.ue(0);
    w.ue(0);
    w.se(0);
    w.flag(false);
    w.flag(false);
    w.flag(false);
    w.se(0);
    w.se(0);
    w.flag(false);
    w.flag(true); // weighted_pred_flag
    let unit = super::h264_bits::annex_b_unit(&[34 << 1, 1], &w.finish());
    assert_eq!(
        parse_pps(&unit[4..]).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    // A slice that uses a long-term picture.
    let lt_sps = {
        let mut sps = parse_sps(&crafted_sps(1, 0, false)).unwrap();
        sps.long_term_present = true;
        sps
    };
    let pps = parse_pps(&pps(&tools())[4..]).unwrap();
    let mut w = BitWriter::default();
    w.flag(true); // first_slice_segment_in_pic_flag
    w.ue(0); // slice_pic_parameter_set_id
    w.ue(1); // P
    w.bits(8, 1); // slice_pic_order_cnt_lsb (log2_max_poc_lsb 8)
    w.flag(false); // short_term_ref_pic_set_sps_flag
    w.ue(1);
    w.ue(0);
    w.ue(0);
    w.flag(true);
    w.ue(1); // num_long_term_pics
    let unit = super::h264_bits::annex_b_unit(&[1 << 1, 1], &w.finish());
    let sps_map = BTreeMap::from([(0, lt_sps)]);
    let pps_map = BTreeMap::from([(0, pps)]);
    let error = parse_slice(&unit[4..], 1, &sps_map, &pps_map, None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}
