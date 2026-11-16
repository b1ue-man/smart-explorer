//! BK4 task tests of the VA-API decoder's bit stream side
//! (`bildkette_task_linux_hw_*`): Exp-Golomb codes and emulation
//! prevention, SPS/PPS/slice header parsing, picture order count, the
//! reference list of P slices and the reference marking, and the decode
//! parameter words, all without hardware.
use super::h264_bits::{annex_b_nal, nal_units, unescape, BitReader, BitWriter, NAL_SLICE};
use super::vaapi_decode_params::{picture_fields, sequence_fields, slice_params};
use super::vaapi_h264_dpb::Dpb;
use super::vaapi_h264_headers::{pps, sps, Layout};
use super::vaapi_h264_slice::{parse_slice, ListModification, Mmco, SliceHeader};
use super::vaapi_h264_syntax::{effective_lists, parse_pps, parse_sps, Pps, ScalingLists, Sps};
use crate::screen::core::negotiate::H264Profile;
use crate::screen::core::pixel_convert::FrameSize;

#[test]
fn bildkette_task_linux_hw_bits_roundtrip() {
    let values = [0u32, 1, 2, 3, 255, 65_535, u32::MAX - 1];
    let signed = [0i32, 1, -1, 26, -26, i32::MAX, -i32::MAX];
    let mut writer = BitWriter::default();
    writer.bits(5, 0b10110);
    values.iter().for_each(|v| writer.ue(*v));
    signed.iter().for_each(|v| writer.se(*v));
    let rbsp = writer.finish();
    let mut reader = BitReader::new(&rbsp);
    assert_eq!(reader.bits(5).unwrap(), 0b10110);
    for value in values {
        assert_eq!(reader.ue().unwrap(), value);
    }
    for value in signed {
        assert_eq!(reader.se().unwrap(), value);
    }
    assert!(!reader.more_rbsp_data());
    // Emulation prevention in, and out again.
    let payload = [0u8, 0, 1, 0, 0, 0, 0, 0, 3, 7];
    let nal = annex_b_nal(3, NAL_SLICE, &payload);
    assert_eq!(&nal[5..], &[0, 0, 3, 1, 0, 0, 3, 0, 0, 3, 0, 3, 7]);
    assert_eq!(unescape(&nal[5..]), payload);
    let stream = [
        &[0u8, 0, 1, 0x67, 1][..],
        &[0, 0, 0, 1, 0x68, 2, 0, 0][..],
        &[0, 0, 1, 0x65, 3][..],
    ]
    .concat();
    let units = nal_units(&stream);
    assert_eq!(units, [&[0x67u8, 1][..], &[0x68, 2][..], &[0x65, 3][..]]);
}

#[test]
fn bildkette_task_linux_hw_parse_own_parameter_sets() {
    let layout = Layout::new(FrameSize::new(1366, 768));
    for profile in [H264Profile::High, H264Profile::ConstrainedBaseline] {
        let sps_unit = sps(&layout, profile, 32);
        let parsed = parse_sps(nal_units(&sps_unit)[0]).unwrap();
        assert_eq!((parsed.width_mbs, parsed.height_mbs), (86, 48));
        let order = (parsed.log2_max_frame_num, parsed.poc_type);
        assert_eq!(order, (16, 2));
        assert_eq!(parsed.max_num_ref_frames, 1);
        assert!(parsed.scaling.is_none());
        let pps_unit = pps(profile);
        let pps = parse_pps(nal_units(&pps_unit)[0], |_| Some(parsed.clone())).unwrap();
        let high = profile == H264Profile::High;
        let tools = (pps.entropy_coding_mode, pps.transform_8x8_mode);
        assert_eq!(tools, (high, high));
        assert!(pps.deblocking_filter_control_present);
        assert_eq!(effective_lists(&parsed, &pps), ScalingLists::flat());
    }
}

/// An SPS with frame_num and POC LSB of four bits each, a scaling matrix
/// whose lists are all absent, and two reference frames.
fn small_sps() -> Sps {
    let mut w = BitWriter::default();
    w.bits(8, 100);
    w.bits(16, 30);
    w.ue(0);
    w.ue(1);
    w.ue(0);
    w.ue(0);
    w.flag(false);
    w.flag(true); // seq_scaling_matrix_present_flag: every list absent
    (0..8).for_each(|_| w.flag(false));
    w.ue(0); // log2_max_frame_num_minus4: 16 frame numbers
    w.ue(0); // pic_order_cnt_type 0
    w.ue(0); // log2_max_pic_order_cnt_lsb_minus4: 16 LSBs
    w.ue(2);
    w.flag(false);
    w.ue(3);
    w.ue(3);
    w.flag(true);
    w.flag(true);
    w.flag(false);
    w.flag(false);
    let unit = [&[0x67u8][..], &w.finish()].concat();
    parse_sps(&unit).unwrap()
}

/// A CAVLC PPS with two active references by default.
fn small_pps(sps: &Sps) -> Pps {
    let mut w = BitWriter::default();
    w.ue(0);
    w.ue(0);
    w.flag(false);
    w.flag(false);
    w.ue(0);
    w.ue(1);
    w.ue(0);
    w.flag(false);
    w.bits(2, 0);
    w.se(0);
    w.se(0);
    w.se(0);
    w.flag(true);
    w.flag(false);
    w.flag(false);
    let unit = [&[0x68u8][..], &w.finish()].concat();
    parse_pps(&unit, |_| Some(sps.clone())).unwrap()
}

/// What a test slice carries besides its kind.
#[derive(Default)]
struct Slice<'a> {
    frame_num: u32,
    lsb: u32,
    modify: &'a [(u32, u32)],
    mmco: &'a [(u32, u32)],
}

/// A slice NAL unit of `small_sps`: IDR or P with optional list
/// modifications and memory operations.
fn slice_unit(idr: bool, slice: Slice<'_>) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.ue(0);
    w.ue(if idr { 7 } else { 5 });
    w.ue(0);
    w.bits(4, slice.frame_num);
    if idr {
        w.ue(0);
    }
    w.bits(4, slice.lsb);
    if !idr {
        w.flag(false);
        w.flag(!slice.modify.is_empty());
        if !slice.modify.is_empty() {
            for (idc, value) in slice.modify {
                w.ue(*idc);
                w.ue(*value);
            }
            w.ue(3);
        }
    }
    if idr {
        w.flag(false);
        w.flag(false);
    } else {
        w.flag(!slice.mmco.is_empty());
        if !slice.mmco.is_empty() {
            for (op, value) in slice.mmco {
                w.ue(*op);
                if *op != 5 {
                    w.ue(*value);
                }
            }
            w.ue(0);
        }
    }
    w.se(-2);
    w.ue(0);
    w.se(1);
    w.se(-1);
    let header = if idr { 0x65 } else { 0x61 };
    [&[header][..], &w.finish()].concat()
}

/// A P slice with `frame_num` and POC LSB and nothing else.
fn p(frame_num: u32, lsb: u32) -> Vec<u8> {
    let slice = Slice {
        frame_num,
        lsb,
        ..Slice::default()
    };
    slice_unit(false, slice)
}

fn parsed(unit: &[u8], sps: &Sps) -> SliceHeader {
    parse_slice(unit, sps, &small_pps(sps)).unwrap()
}

#[test]
fn bildkette_task_linux_hw_slice_header_and_scaling_fallback() {
    let sps = small_sps();
    let lists = sps.scaling.clone().expect("matrix present");
    assert_eq!(lists.list4x4[0][..4], [6, 13, 13, 20], "rule A: intra");
    assert_eq!(lists.list4x4[3][..4], [10, 14, 14, 20], "rule A: inter");
    let slice = Slice {
        frame_num: 3,
        lsb: 6,
        modify: &[(0, 1)],
        mmco: &[(1, 0)],
    };
    let header = parsed(&slice_unit(false, slice), &sps);
    let fields = (header.slice_type, header.frame_num, header.poc_lsb);
    assert_eq!(fields, (0, 3, 6));
    assert_eq!(header.num_ref_idx_l0_minus1, 1);
    assert_eq!(header.modifications, [ListModification::Subtract(2)]);
    assert_eq!(header.mmco, Some(vec![Mmco::UnmarkShort(1)]));
    assert_eq!(header.slice_qp_delta, -2);
    let deblocking = (header.alpha_offset_div2, header.beta_offset_div2);
    assert_eq!(deblocking, (1, -1));
    assert!(header.header_bits > 8);
    let parameters = slice_params(&header, &[], 100).unwrap();
    let offset = parameters.slice_data_bit_offset as usize;
    assert_eq!(offset, header.header_bits);
    let data = (parameters.slice_data_size, parameters.slice_type);
    assert_eq!(data, (100, 0));
}

#[test]
fn bildkette_task_linux_hw_reference_pictures() {
    let sps = small_sps();
    let decode = |dpb: &mut Dpb, unit: Vec<u8>, surface: u32| {
        let header = parsed(&unit, &sps);
        dpb.check_gap(&header, &sps).unwrap();
        let poc = dpb.poc(&header, &sps).unwrap();
        let list = if header.slice_type == 0 {
            dpb.list0(&header, &sps).unwrap()
        } else {
            Vec::new()
        };
        dpb.mark(&header, &sps, surface, poc).unwrap();
        (poc.0, list.iter().map(|r| r.surface).collect::<Vec<_>>())
    };
    let idr = slice_unit(true, Slice::default());
    let mut dpb = Dpb::default();
    assert_eq!(decode(&mut dpb, idr.clone(), 10).0, 0);
    // Two references are active by default; only one exists yet.
    assert_eq!(decode(&mut dpb, p(1, 2), 11), (2, vec![10]));
    // The newest first; the sliding window drops the oldest.
    assert_eq!(decode(&mut dpb, p(2, 4), 12), (4, vec![11, 10]));
    assert_eq!(decode(&mut dpb, p(3, 6), 13), (6, vec![12, 11]));
    // Modification: PicNum 4 − 2 = 2 (surface 12) moves first.
    let modified = Slice {
        frame_num: 4,
        lsb: 8,
        modify: &[(0, 1)],
        ..Slice::default()
    };
    let list = decode(&mut dpb, slice_unit(false, modified), 14).1;
    assert_eq!(list, vec![12, 13]);
    // POC LSBs wrap at 16: 12 → 2 continues at 18.
    let mut wrap = Dpb::default();
    decode(&mut wrap, idr, 1);
    decode(&mut wrap, p(1, 6), 2);
    decode(&mut wrap, p(2, 12), 3);
    assert_eq!(decode(&mut wrap, p(3, 2), 4).0, 18);
    // frame_num gaps are refused.
    let gap = parsed(&p(7, 2), &sps);
    assert!(wrap.check_gap(&gap, &sps).is_err());
    // Memory operation 5 empties the buffer; the picture's POC becomes 0.
    let reset = Slice {
        frame_num: 4,
        lsb: 4,
        mmco: &[(5, 0)],
        ..Slice::default()
    };
    decode(&mut wrap, slice_unit(false, reset), 5);
    assert_eq!(wrap.refs.len(), 1);
    assert_eq!(wrap.refs[0].top_poc, 0);
}

#[test]
fn bildkette_task_linux_hw_decode_parameter_words() {
    let sps = small_sps();
    let pps = small_pps(&sps);
    let fields = sequence_fields(&sps);
    assert_eq!(fields & 3, 1);
    assert_eq!((fields >> 4) & 1, 1, "frame_mbs_only_flag");
    assert_eq!((fields >> 12) & 3, 0, "POC type 0");
    assert_eq!((fields >> 14) & 0xf, 0, "log2_max_pic_order_cnt_lsb_minus4");
    let words = picture_fields(&pps, true);
    assert_eq!(words & 1, 0, "CAVLC");
    assert_eq!((words >> 8) & 1, 1, "deblocking control");
    assert_eq!((words >> 10) & 1, 1, "reference picture");
}
