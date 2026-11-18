//! Finding 1 of the measurement of 2026-11-18 (0.5.200, AV1 from an AMD
//! host): a key frame is told by its bitstream, not by the encoder's flag
//! alone, so the first-picture watch never counts a real AV1 key frame as
//! silence.
use super::*;

/// One OBU with its size field (AV1 5.3.1): `kind`, then leb128 size.
fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(kind << 3) | 0x02, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

/// A temporal unit: temporal delimiter, optionally a sequence header, one
/// frame OBU whose header starts with `show_existing_frame` 0 and the
/// two bits of `frame_type`.
fn temporal_unit(sequence: bool, frame_type: u8) -> Vec<u8> {
    let mut unit = obu(2, &[]);
    if sequence {
        unit.extend(obu(1, &[0x00, 0x00, 0x00, 0x6a, 0xef, 0xbf, 0xe1, 0xbc]));
    }
    // show_existing_frame (1 bit) 0, frame_type (2 bits), show_frame 1.
    let header = ((frame_type & 0x03) << 5) | 0x10;
    unit.extend(obu(6, &[header, 0x00, 0x00, 0x00]));
    unit
}

#[test]
fn rollout_task_av1_keyframe_from_obu_bytes() {
    let key = temporal_unit(true, 0);
    let inter = temporal_unit(false, 1);
    // A key frame without its flag (an encoder that does not mark it).
    assert!(starts_group(CodecKind::Av1, false, [&key]));
    // A key frame whose sequence header was put in front by the hull is
    // still one; so is the frame header alone (OBU_FRAME_HEADER, type 3).
    assert!(starts_group(
        CodecKind::Av1,
        false,
        [&temporal_unit(false, 0)]
    ));
    let mut header_only = obu(2, &[]);
    header_only.extend(obu(3, &[0x10, 0x00]));
    assert!(starts_group(CodecKind::Av1, false, [&header_only]));
    // An inter frame (INTER_FRAME 1) and an intra-only frame (2) are none.
    assert!(!starts_group(CodecKind::Av1, false, [&inter]));
    assert!(!starts_group(
        CodecKind::Av1,
        false,
        [&temporal_unit(true, 2)]
    ));
    // A frame shown again (`show_existing_frame` 1) is none.
    let mut existing = obu(2, &[]);
    existing.extend(obu(3, &[0x80]));
    assert!(!starts_group(CodecKind::Av1, false, [&existing]));
    // The flag stands; broken bytes and several records keep the flag.
    assert!(starts_group(CodecKind::Av1, true, [&inter]));
    assert!(!starts_group(CodecKind::Av1, false, [&[0x80u8, 0x00][..]]));
    assert!(!starts_group(CodecKind::Av1, false, [&key, &key]));
    assert!(!starts_group(CodecKind::Tiles, false, [&key]));
    // H.264: an IDR unit (type 5) without the flag.
    let idr = [0u8, 0, 0, 1, 0x65, 0x88];
    assert!(starts_group(CodecKind::H264, false, [&idr[..]]));
    assert!(!starts_group(
        CodecKind::H264,
        false,
        [&[0u8, 0, 0, 1, 0x41, 0x9a][..]]
    ));
    // No records: no group.
    assert!(!starts_group(CodecKind::Av1, false, Vec::<Vec<u8>>::new()));
}
