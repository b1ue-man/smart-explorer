//! BK4b task tests (`bildkette_task_linux_hw_av1_*`): the VA-API AV1
//! encoder without hardware. The program's sequence and frame header OBUs
//! through its own parser and the core's check, the bit offsets the
//! driver rewrites, the order of a temporal unit, the parameter mapping of
//! the encoder, the OBU bit syntax and the C structure sizes counted from
//! libva 2.20.0. The decoder's tests are in
//! `bildkette_hw_av1_decode_task_tests`.
use std::mem::{offset_of, size_of};

use super::h264_bits::{BitReader, BitWriter};
use super::hw_encode_common::RateParams;
use super::vaapi_av1_abi as abi;
use super::vaapi_av1_bits::{
    leb128, leb128_fixed4, ns, obu, ordered_temporal_unit, read_leb128, split_obus, su, uvlc,
    OBU_FRAME_HEADER, OBU_SEQUENCE_HEADER, OBU_TEMPORAL_DELIMITER, OBU_TILE_GROUP,
    TEMPORAL_DELIMITER,
};
use super::vaapi_av1_decode::refresh;
use super::vaapi_av1_encode::av1_level_idx;
use super::vaapi_av1_frame::{parse_frame_header, FrameHeader, INTER_FRAME, KEY_FRAME};
use super::vaapi_av1_headers::{
    frame_header, sequence_header, sequence_params, tile_group, Av1Layout, Av1Picture, Av1Tools,
    FrameHeaderObu, INITIAL_QINDEX,
};
use super::vaapi_av1_syntax::{
    parse_sequence_header, RefSlot, SequenceHeader, SuperBlocks, NUM_REF_FRAMES,
};
use super::vaapi_ffi::{VA_ATTRIB_NOT_SUPPORTED, VA_INVALID_ID};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::frame_size_caps::{
    check_video_dimensions, video_keyframe, video_parameter_sets,
};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;

pub(super) const SIZE: FrameSize = FrameSize::new(1920, 1080);

/// The tools of a driver that reports none of the AV1 attributes.
pub(super) fn plain() -> Av1Tools {
    Av1Tools::new(
        VA_ATTRIB_NOT_SUPPORTED,
        VA_ATTRIB_NOT_SUPPORTED,
        VA_ATTRIB_NOT_SUPPORTED,
        VA_ATTRIB_NOT_SUPPORTED,
    )
}

/// Palette, switchable filters and both reference lists.
fn rich() -> Av1Tools {
    Av1Tools::new(1 << 12, 0x10, 3 | (4 << 4), 0x5)
}

pub(super) fn picture(key: bool, order_hint: u8) -> Av1Picture {
    Av1Picture {
        key,
        order_hint,
        recon: 21,
        reference: (!key).then_some(20),
        coded: 99,
    }
}

/// `count` bits of `data` from bit `offset`.
fn bits_at(data: &[u8], offset: u32, count: u32) -> u32 {
    let mut r = BitReader::new(data);
    for _ in 0..offset {
        r.bits(1).unwrap();
    }
    r.bits(count).unwrap()
}

/// The program's sequence header of `layout`, parsed by its own parser.
pub(super) fn parsed_sequence(layout: &Av1Layout, tools: &Av1Tools) -> SequenceHeader {
    let bytes = sequence_header(layout, tools, 8);
    let units = split_obus(&bytes).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].kind, OBU_SEQUENCE_HEADER);
    parse_sequence_header(units[0].payload).unwrap()
}

/// The payload of a frame header OBU of `header`.
pub(super) fn payload(header: &FrameHeaderObu) -> &[u8] {
    &header.bytes[5..]
}

#[test]
fn bildkette_task_linux_hw_av1_struct_sizes() {
    assert_eq!(size_of::<abi::VaEncSequenceAv1>(), 88);
    assert_eq!(offset_of!(abi::VaEncSequenceAv1, seq_fields), 16);
    assert_eq!(
        offset_of!(abi::VaEncSequenceAv1, order_hint_bits_minus_1),
        20
    );
    assert_eq!(size_of::<abi::VaEncSegParamAv1>(), 156);
    assert_eq!(offset_of!(abi::VaEncSegParamAv1, feature_mask), 130);
    assert_eq!(size_of::<abi::VaWarpedMotionAv1>(), 56);
    assert_eq!(offset_of!(abi::VaWarpedMotionAv1, invalid), 36);
    assert_eq!(size_of::<abi::VaEncPictureAv1>(), 1032);
    assert_eq!(offset_of!(abi::VaEncPictureAv1, ref_frame_ctrl_l0), 56);
    assert_eq!(offset_of!(abi::VaEncPictureAv1, picture_flags), 64);
    assert_eq!(offset_of!(abi::VaEncPictureAv1, base_qindex), 88);
    assert_eq!(offset_of!(abi::VaEncPictureAv1, mode_control_flags), 100);
    assert_eq!(offset_of!(abi::VaEncPictureAv1, segments), 104);
    assert_eq!(offset_of!(abi::VaEncPictureAv1, width_in_sbs_minus_1), 264);
    assert_eq!(
        offset_of!(abi::VaEncPictureAv1, context_update_tile_id),
        516
    );
    assert_eq!(offset_of!(abi::VaEncPictureAv1, wm), 540);
    assert_eq!(offset_of!(abi::VaEncPictureAv1, bit_offset_qindex), 932);
    assert_eq!(
        offset_of!(abi::VaEncPictureAv1, tile_group_obu_hdr_info),
        960
    );
    assert_eq!(
        offset_of!(abi::VaEncPictureAv1, skip_frames_reduced_size),
        964
    );
    assert_eq!(size_of::<abi::VaEncTileGroupAv1>(), 20);
    assert_eq!(size_of::<abi::VaSegmentationAv1>(), 156);
    assert_eq!(size_of::<abi::VaFilmGrainAv1>(), 176);
    assert_eq!(offset_of!(abi::VaFilmGrainAv1, cb_offset), 154);
    assert_eq!(size_of::<abi::VaDecPictureAv1>(), 1160);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, anchor_frames_list), 24);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, ref_frame_map), 40);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, seg_info), 84);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, film_grain_info), 240);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, width_in_sbs_minus_1), 418);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, pic_info_fields), 676);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, base_qindex), 697);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, qmatrix_fields), 704);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, mode_control_fields), 708);
    assert_eq!(offset_of!(abi::VaDecPictureAv1, wm), 732);
    assert_eq!(size_of::<abi::VaSliceParameterAv1>(), 40);
    assert_eq!(
        offset_of!(abi::VaSliceParameterAv1, tile_idx_in_tile_list),
        22
    );
}

#[test]
fn bildkette_task_linux_hw_av1_obu_bits() {
    for value in [0u64, 1, 127, 128, 16_383, 16_384, 1 << 40] {
        let bytes = leb128(value);
        assert_eq!(read_leb128(&bytes, 0).unwrap(), (value, bytes.len()));
    }
    let padded = leb128_fixed4(300);
    assert_eq!(read_leb128(&padded, 0).unwrap(), (300, 4));
    let mut w = BitWriter::default();
    w.bits(7, 0x7f); // su(7) −1
    w.bits(2, 0b01); // ns(5): w 3, m 3, v (2 bits) 1 < 3 → 1
    w.bits(3, 0b001); // uvlc: two zeros, one, then "01" → 4
    w.bits(2, 0b01);
    let data = w.finish();
    let mut r = BitReader::new(&data);
    assert_eq!(su(&mut r, 7).unwrap(), -1);
    assert_eq!(ns(&mut r, 5).unwrap(), 1);
    assert_eq!(uvlc(&mut r).unwrap(), 4);
    // An OBU with an extension, one without a size field at the end.
    let mut stream = vec![(OBU_TILE_GROUP << 3) | 0x06, 0b0100_0000, 2, 0xaa, 0xbb];
    stream.extend_from_slice(&[OBU_FRAME_HEADER << 3, 1, 2, 3]);
    let units = split_obus(&stream).unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(
        (units[0].temporal_id, units[0].payload),
        (2, &[0xaa, 0xbb][..])
    );
    assert_eq!(units[1].payload, &[1, 2, 3][..]);
    assert!(split_obus(&[0x80]).is_err(), "forbidden bit");
    assert!(split_obus(&[0x0a, 5, 1]).is_err(), "longer than the unit");
    assert_eq!(TEMPORAL_DELIMITER, [0x12, 0]);
    // Superblocks and Annex A tile limits.
    let sb = SuperBlocks::new(1920, 1080, false);
    assert_eq!((sb.cols, sb.rows, sb.size_log2), (30, 17, 6));
    assert_eq!(sb.limits(), (0, 5, 5, 0));
    let big = SuperBlocks::new(4096, 2304, false);
    assert_eq!(big.limits().0, 0, "one tile column up to 4096");
}

#[test]
fn bildkette_task_linux_hw_av1_sequence_header_roundtrip() {
    for tools in [plain(), rich()] {
        let layout = Av1Layout::new(SIZE, &tools);
        let seq = parsed_sequence(&layout, &tools);
        assert_eq!((seq.max_width, seq.max_height), (1920, 1080));
        assert_eq!(seq.order_hint_bits, 8);
        assert!(seq.enable_order_hint && seq.enable_cdef && !seq.enable_restoration);
        assert_eq!(
            seq.seq_force_screen_content_tools,
            u8::from(tools.screen_content)
        );
        assert_eq!(seq.matrix_coefficients, 1);
        assert!(!seq.color_range && !seq.enable_superres);
        let level = av1_level_idx(SIZE, 60);
        let bytes = sequence_header(&layout, &tools, level);
        check_video_dimensions(CodecKind::Av1, &bytes, SIZE, FrameLimits::RESOURCE).unwrap();
        assert_eq!(
            video_parameter_sets(CodecKind::Av1, &bytes),
            Some(bytes.clone())
        );
    }
    // Level 4.0 (8) shows 70 778 880 samples/s, 1080p60 needs 124 416 000.
    assert_eq!(av1_level_idx(SIZE, 60), 9, "level 4.1 carries 1080p60");
}

#[test]
fn bildkette_task_linux_hw_av1_frame_header_roundtrip() {
    for tools in [plain(), rich()] {
        let layout = Av1Layout::new(SIZE, &tools);
        let seq = parsed_sequence(&layout, &tools);
        let mut slots: [RefSlot; NUM_REF_FRAMES] = Default::default();
        let mut previous: Option<FrameHeader> = None;
        for (index, key) in [true, false, false].into_iter().enumerate() {
            let written = frame_header(&layout, &tools, &picture(key, index as u8));
            let units = split_obus(&written.bytes).unwrap();
            assert_eq!(units[0].kind, OBU_FRAME_HEADER);
            assert_eq!(units[0].payload, payload(&written));
            let h = parse_frame_header(payload(&written), &seq, &slots, 0, 0).unwrap();
            assert_eq!(h.frame_type, if key { KEY_FRAME } else { INTER_FRAME });
            assert_eq!(h.order_hint, index as u32);
            assert_eq!(h.refresh_frame_flags, 0xff);
            assert_eq!(h.primary_ref_frame, if key { 7 } else { 0 });
            assert_eq!(h.ref_frame_idx, [0; 7]);
            assert_eq!((h.frame_width, h.frame_height), (1920, 1080));
            assert_eq!(h.base_q_idx, INITIAL_QINDEX);
            assert_eq!(h.loop_filter_level, [8; 4]);
            assert_eq!((h.cdef_damping_minus_3, h.cdef_bits), (2, 0));
            assert_eq!(h.tx_mode, 2);
            assert_eq!(h.reference_select, !key && tools.compound);
            assert!(
                !h.skip_mode_present,
                "all references are the previous frame"
            );
            assert_eq!(
                h.interpolation_filter,
                if key || !tools.switchable_filter {
                    0
                } else {
                    4
                }
            );
            assert_eq!(h.tiles.count(), 1);
            // The offsets the driver's rate control rewrites.
            assert_eq!(h.header_bits as u32 + 41, written.size_bits);
            assert_eq!(bits_at(&written.bytes, written.qindex, 8), 128);
            assert_eq!(bits_at(&written.bytes, written.loop_filter, 6), 8);
            assert_eq!(bits_at(&written.bytes, written.cdef, 2), 2);
            assert_eq!(written.cdef_bits, 2 + 2 + 6 + 6);
            assert_eq!(
                read_leb128(&written.bytes, 1).unwrap().0 as usize,
                payload(&written).len()
            );
            refresh(&mut slots, &h, 30 + index as u32);
            if let Some(previous) = &previous {
                assert_eq!(previous.order_hint + 1, h.order_hint);
            }
            previous = Some(h);
        }
        assert!(slots.iter().all(|slot| slot.valid && slot.surface == 32));
    }
}

#[test]
fn bildkette_task_linux_hw_av1_temporal_unit_order() {
    let tools = plain();
    let layout = Av1Layout::new(SIZE, &tools);
    let sequence = sequence_header(&layout, &tools, 8);
    let header = frame_header(&layout, &tools, &picture(true, 0)).bytes;
    let tiles = obu(OBU_TILE_GROUP, &[1, 2, 3]);
    // Driver wrote no TD and no sequence header: both are added.
    let bare = [header.clone(), tiles.clone()].concat();
    let unit = ordered_temporal_unit(&bare, true, &sequence).unwrap();
    let kinds: Vec<_> = split_obus(&unit).unwrap().iter().map(|o| o.kind).collect();
    assert_eq!(
        kinds,
        [
            OBU_TEMPORAL_DELIMITER,
            OBU_SEQUENCE_HEADER,
            OBU_FRAME_HEADER,
            OBU_TILE_GROUP
        ]
    );
    assert!(video_keyframe(CodecKind::Av1, &unit));
    assert_eq!(
        video_parameter_sets(CodecKind::Av1, &unit),
        Some(sequence.clone())
    );
    // Driver wrote sequence header before its TD: the order is mended.
    let swapped = [sequence.clone(), TEMPORAL_DELIMITER.to_vec(), bare.clone()].concat();
    assert_eq!(ordered_temporal_unit(&swapped, true, &[]).unwrap(), unit);
    // Inter frames carry no sequence header.
    let inter = frame_header(&layout, &tools, &picture(false, 1)).bytes;
    let inter_unit = ordered_temporal_unit(&[inter.clone(), tiles].concat(), false, &sequence);
    let kinds: Vec<_> = split_obus(&inter_unit.unwrap())
        .unwrap()
        .iter()
        .map(|o| o.kind)
        .collect();
    assert_eq!(
        kinds,
        [OBU_TEMPORAL_DELIMITER, OBU_FRAME_HEADER, OBU_TILE_GROUP]
    );
    // An OBU without a size field is refused (every viewer needs sizes).
    let unsized_obu = [inter, vec![OBU_TILE_GROUP << 3, 9]].concat();
    assert!(ordered_temporal_unit(&unsized_obu, false, &sequence).is_err());
}

#[test]
fn bildkette_task_linux_hw_av1_parameter_mapping() {
    let tools = rich();
    assert!(tools.screen_content && tools.switchable_filter && tools.tx_select);
    assert!(tools.compound && !tools.use_128 && !tools.filter_intra);
    assert_eq!(tools.tile_size_bytes, 4);
    let plain = plain();
    assert!(!plain.screen_content && !plain.switchable_filter && plain.tx_select);
    // Only TX_MODE_LARGEST reported: no TX_MODE_SELECT.
    assert!(!Av1Tools::new(VA_ATTRIB_NOT_SUPPORTED, 0, 2 << 4, 0).tx_select);
    let layout = Av1Layout::new(SIZE, &tools);
    assert_eq!(layout.surface(), FrameSize::new(1920, 1088));
    assert_eq!(
        (layout.tiles.widths.clone(), layout.tiles.heights.clone()),
        (vec![30], vec![17])
    );
    let rate = RateParams::for_target(10_000_000, 60);
    let seq = sequence_params(&tools, 8, &rate);
    assert_eq!(
        (seq.seq_profile, seq.seq_level_idx, seq.ip_period),
        (0, 8, 1)
    );
    assert_eq!(seq.order_hint_bits_minus_1, 7);
    // order hint, CDEF, 4:2:0.
    assert_eq!(seq.seq_fields, (1 << 8) | (1 << 12) | (1 << 17) | (1 << 18));
    for key in [true, false] {
        let p = picture(key, 3);
        let header = frame_header(&layout, &tools, &p);
        let params = super::vaapi_av1_headers::picture_params(&layout, &tools, &p, &header);
        assert_eq!(
            (params.frame_width_minus_1, params.frame_height_minus_1),
            (1919, 1079)
        );
        assert_eq!(params.reconstructed_frame, 21);
        let reference = if key { VA_INVALID_ID } else { 20 };
        assert_eq!(params.reference_frames, [reference; 8]);
        assert_eq!(params.primary_ref_frame, if key { 7 } else { 0 });
        assert_eq!(params.refresh_frame_flags, 0xff);
        assert_eq!(params.ref_frame_ctrl_l0, u32::from(!key));
        assert_eq!(params.ref_frame_ctrl_l1, if key { 0 } else { 5 });
        assert_eq!(params.picture_flags & 3, u32::from(!key));
        assert_eq!((params.min_base_qindex, params.max_base_qindex), (1, 255));
        assert_eq!((params.tile_cols, params.tile_rows), (1, 1));
        assert_eq!(
            (
                params.width_in_sbs_minus_1[0],
                params.height_in_sbs_minus_1[0]
            ),
            (29, 16)
        );
        assert_eq!(params.bit_offset_qindex, header.qindex);
        assert_eq!(params.size_in_bits_frame_hdr_obu, header.size_bits);
        assert_eq!(params.byte_offset_frame_hdr_obu_size, 1);
        assert_eq!(params.tile_group_obu_hdr_info, 0x02);
        let reference_mode = (params.mode_control_flags >> 9) & 3;
        assert_eq!(reference_mode, if key { 0 } else { 2 });
        assert_eq!((params.mode_control_flags >> 7) & 3, 2, "TX_MODE_SELECT");
    }
    let group = tile_group(&layout);
    assert_eq!((group.tg_start, group.tg_end), (0, 0));
}
