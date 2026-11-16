//! BK4b task tests of the VA-API AV1 decoder (`bildkette_task_linux_hw_av1_*`,
//! without hardware): the decode parameters of the program's own key and
//! inter frames, the reference slots, the tile groups with one and two
//! tiles, and the refusal of every tool outside the screen contract
//! (profiles, bit depth, film grain, show_existing_frame, hidden frames,
//! intra-only frames, global motion, empty reference slots).
use std::io::ErrorKind;

use super::bildkette_hw_av1_task_tests::{parsed_sequence, payload, picture, plain, SIZE};
use super::h264_bits::BitWriter;
use super::vaapi_av1_decode::{parse_tile_group, picture_params, refresh};
use super::vaapi_av1_frame::{parse_frame_header, FrameHeader};
use super::vaapi_av1_headers::{frame_header, Av1Layout};
use super::vaapi_av1_syntax::{parse_sequence_header, RefSlot, TileInfo, NUM_REF_FRAMES};
use super::vaapi_ffi::VA_INVALID_ID;

#[test]
fn bildkette_task_linux_hw_av1_tile_group_and_decode_params() {
    let tools = plain();
    let layout = Av1Layout::new(SIZE, &tools);
    let seq = parsed_sequence(&layout, &tools);
    let mut slots: [RefSlot; NUM_REF_FRAMES] = Default::default();
    let key = frame_header(&layout, &tools, &picture(true, 0));
    let h = parse_frame_header(payload(&key), &seq, &slots, 0, 0).unwrap();
    let p = picture_params(&seq, &h, &slots, 40);
    assert_eq!((p.current_frame, p.current_display_picture), (40, 40));
    assert_eq!(
        p.ref_frame_map, [VA_INVALID_ID; 8],
        "a key frame names no reference"
    );
    assert_eq!((p.frame_width_minus1, p.frame_height_minus1), (1919, 1079));
    // KEY, shown, error resilient, force_integer_mv, uniform tiles.
    assert_eq!(
        p.pic_info_fields,
        (1 << 2) | (1 << 4) | (1 << 7) | (1 << 14)
    );
    assert_eq!(
        (p.base_qindex, p.filter_level, p.superres_scale_denominator),
        (128, [8, 8], 8)
    );
    assert_eq!((p.mode_control_fields >> 7) & 3, 2);
    assert_eq!(p.ref_deltas, [1, 0, 0, 0, -1, 0, -1, -1]);
    assert_eq!(p.order_hint_bits_minus_1, 7);
    assert_eq!(
        p.seq_info_fields & (1 << 7 | 1 << 9 | 3 << 12),
        1 << 7 | 1 << 9 | 3 << 12
    );
    refresh(&mut slots, &h, 40);
    let inter = frame_header(&layout, &tools, &picture(false, 1));
    let h = parse_frame_header(payload(&inter), &seq, &slots, 0, 0).unwrap();
    let p = picture_params(&seq, &h, &slots, 41);
    assert_eq!(p.ref_frame_map, [40; 8]);
    assert_eq!(p.primary_ref_frame, 0);
    // One tile: no tile_start_and_end_present_flag, data after alignment.
    let single = parse_tile_group(&[9, 8, 7], &h).unwrap();
    assert_eq!(
        (single.start, single.end, single.data),
        (0, 0, &[9, 8, 7][..])
    );
    assert_eq!(single.tiles[0].slice_data_size, 3);
    // Two tile columns: the first tile's size in TileSizeBytes, the last runs on.
    let two = FrameHeader {
        tiles: TileInfo {
            uniform: true,
            cols_log2: 1,
            rows_log2: 0,
            widths: vec![15, 15],
            heights: vec![17],
            context_update_tile_id: 0,
            size_bytes: 2,
        },
        ..h.clone()
    };
    let data = [0x00, 0x01, 0x00, 0xa1, 0xa2, 0xb1, 0xb2, 0xb3];
    let group = parse_tile_group(&data, &two).unwrap();
    assert_eq!(group.data, &data[1..]);
    let tiles: Vec<_> = group
        .tiles
        .iter()
        .map(|t| (t.slice_data_offset, t.slice_data_size, t.tile_column))
        .collect();
    assert_eq!(tiles, [(2, 2, 0), (4, 3, 1)]);
    assert!(
        parse_tile_group(&[0x00, 0x09, 0x00, 1], &two).is_err(),
        "tile beyond its group"
    );
}

/// A sequence header with the knobs the refusal test turns (AV1 5.5).
fn crafted_sequence(profile: u32, high_bitdepth: bool, film_grain: bool) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.bits(3, profile);
    w.flag(false); // still_picture
    w.flag(false); // reduced_still_picture_header
    w.flag(false); // timing_info_present_flag
    w.flag(false); // initial_display_delay_present_flag
    w.bits(5, 0);
    w.bits(12, 0);
    w.bits(5, 8);
    w.flag(false); // seq_tier
    w.bits(4, 15);
    w.bits(4, 15);
    w.bits(16, 1919);
    w.bits(16, 1079);
    w.flag(false); // frame_id_numbers_present_flag
    for _ in 0..7 {
        w.flag(false); // 128, filter intra, edge, interintra, masked, warped, dual
    }
    w.flag(true); // enable_order_hint
    w.flag(false);
    w.flag(false);
    w.flag(false); // seq_choose_screen_content_tools
    w.flag(false); // seq_force_screen_content_tools
    w.bits(3, 7);
    w.flag(false); // enable_superres
    w.flag(true); // enable_cdef
    w.flag(false); // enable_restoration
    w.flag(high_bitdepth);
    w.flag(false); // mono_chrome
    w.flag(false); // color_description_present_flag
    w.flag(false); // color_range
    w.bits(2, 0);
    w.flag(false); // separate_uv_delta_q
    w.flag(film_grain);
    w.finish()
}

#[test]
fn bildkette_task_linux_hw_av1_refuses_unsupported_tools() {
    let seq = parse_sequence_header(&crafted_sequence(0, false, false)).expect("Main parses");
    for (bytes, what) in [
        (crafted_sequence(1, false, false), "High profile"),
        (crafted_sequence(0, true, false), "10 bit"),
        (crafted_sequence(0, false, true), "film grain"),
    ] {
        let error = parse_sequence_header(&bytes).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{what}");
    }
    let slots: [RefSlot; NUM_REF_FRAMES] = Default::default();
    for (first_bits, what) in [
        (0b1000_0000, "show_existing_frame"),
        (0b0010_0000, "hidden inter frame"),
        (0b0101_0000, "intra-only frame"),
    ] {
        let error = parse_frame_header(&[first_bits, 0, 0, 0], &seq, &slots, 0, 0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{what}");
    }
    // Global motion: set the first `is_global` of a valid inter header.
    let tools = plain();
    let layout = Av1Layout::new(SIZE, &tools);
    let own = parsed_sequence(&layout, &tools);
    let mut slots: [RefSlot; NUM_REF_FRAMES] = Default::default();
    let key = frame_header(&layout, &tools, &picture(true, 0));
    let h = parse_frame_header(payload(&key), &own, &slots, 0, 0).unwrap();
    refresh(&mut slots, &h, 1);
    let inter = frame_header(&layout, &tools, &picture(false, 1));
    let mut bytes = payload(&inter).to_vec();
    let at = (inter.size_bits - 41 - 7) as usize;
    bytes[at / 8] |= 0x80 >> (at % 8);
    let error = parse_frame_header(&bytes, &own, &slots, 0, 0).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported, "global motion");
    // An inter frame naming an empty slot is an error, not a wrong picture.
    let empty: [RefSlot; NUM_REF_FRAMES] = Default::default();
    assert!(parse_frame_header(payload(&inter), &own, &empty, 0, 0).is_err());
}
