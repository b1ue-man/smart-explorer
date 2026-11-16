//! Acceptance of plan 2026-11-14 „Bildkette“ BK6: the stream marks and the
//! parameter checks of HEVC and AV1 (`frame_size_hevc_av1.rs`) on streams
//! written bit by bit after ITU-T H.265 7.3.2.2 and AV1 5.5.1/5.9.2, and
//! the bit-rate ceiling of their levels (B-06).
use super::super::super::negotiate::CodecKind;
use super::super::FrameSize;
use super::{
    check_video_dimensions, video_keyframe, video_max_bitrate_bps, video_parameter_sets,
    HEVC_LEVELS,
};
use crate::screen::core::frame_budget::FrameLimits;

/// Big-endian bits with Exp-Golomb codes.
#[derive(Default)]
struct Writer(Vec<bool>);

impl Writer {
    fn bits(&mut self, value: u64, count: usize) -> &mut Self {
        for index in (0..count).rev() {
            self.0.push(value >> index & 1 == 1);
        }
        self
    }

    fn ue(&mut self, value: u64) -> &mut Self {
        let coded = value + 1;
        let length = 64 - coded.leading_zeros() as usize;
        self.bits(0, length - 1).bits(coded, length)
    }

    /// The bytes, the last one padded with a stop bit and zeros.
    fn bytes(&mut self, stop: bool) -> Vec<u8> {
        if stop {
            self.0.push(true);
        }
        while self.0.len() % 8 != 0 {
            self.0.push(false);
        }
        self.0
            .chunks(8)
            .map(|byte| byte.iter().fold(0u8, |acc, bit| acc << 1 | u8::from(*bit)))
            .collect()
    }
}

/// Inserts emulation prevention bytes (H.265 7.4.2).
fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut zeros = 0;
    for &byte in rbsp {
        if zeros >= 2 && byte <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// An Annex B NAL unit of `kind` with `rbsp`.
fn nal(kind: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 1, kind << 1, 1];
    out.extend(escape(rbsp));
    out
}

/// An HEVC SPS of Main at level 4 for 1920 × 1080 (coded 1920 × 1088).
fn hevc_sps(level_idc: u64, chroma: u64) -> Vec<u8> {
    let mut w = Writer::default();
    w.bits(0, 4).bits(0, 3).bits(1, 1);
    // profile_tier_level: space, tier, profile 1, compatibility flag 1,
    // progressive, interlaced, non-packed, frame-only, 43 + 1 bits.
    w.bits(0, 2).bits(0, 1).bits(1, 5).bits(0x4000_0000, 32);
    w.bits(0b1001, 4).bits(0, 43).bits(0, 1).bits(level_idc, 8);
    w.ue(0).ue(chroma);
    if chroma == 3 {
        w.bits(0, 1);
    }
    w.ue(1920).ue(1088).bits(1, 1);
    // Conformance window in chroma units of 4:2:0: 4 × 2 = 8 rows.
    w.ue(0).ue(0).ue(0).ue(if chroma == 3 { 8 } else { 4 });
    w.ue(0).ue(0);
    w.bytes(true)
}

/// An AV1 OBU of `kind` with its size field.
fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![kind << 3 | 0b010];
    let mut size = payload.len();
    loop {
        let byte = (size & 0x7f) as u8;
        size >>= 7;
        out.push(if size > 0 { byte | 0x80 } else { byte });
        if size == 0 {
            break;
        }
    }
    out.extend_from_slice(payload);
    out
}

/// An AV1 sequence header of Main at `seq_level_idx` for `width` × `height`.
fn av1_sequence(level: u64, width: u64, height: u64) -> Vec<u8> {
    let mut w = Writer::default();
    w.bits(0, 3).bits(0, 1).bits(0, 1); // profile, still, reduced
    w.bits(0, 1).bits(0, 1).bits(0, 5); // timing, display delay, 1 point
    w.bits(0, 12).bits(level, 5);
    if level > 7 {
        w.bits(0, 1);
    }
    w.bits(15, 4)
        .bits(15, 4)
        .bits(width - 1, 16)
        .bits(height - 1, 16);
    w.bytes(true)
}

/// A frame OBU whose header begins with `show_existing_frame` 0 and
/// `frame_type` (0 KEY_FRAME, 1 INTER_FRAME).
fn av1_frame(frame_type: u64) -> Vec<u8> {
    let mut w = Writer::default();
    w.bits(0, 1).bits(frame_type, 2).bits(0, 13);
    obu(6, &w.bytes(false))
}

#[test]
fn bildkette_task_codec_hevc_marks_and_sps_check() {
    let size = FrameSize::new(1920, 1080);
    let sps = hevc_sps(120, 1);
    let mut idr = nal(32, &[0x0c, 0x01]);
    idr.extend(nal(33, &sps));
    idr.extend(nal(34, &[0xc1, 0x72]));
    idr.extend(nal(19, &[0xaf, 0x00, 0x10]));
    assert!(video_keyframe(CodecKind::Hevc, &idr));
    assert!(!video_keyframe(CodecKind::Hevc, &nal(1, &[0x02, 0x80])));
    let sets = video_parameter_sets(CodecKind::Hevc, &idr).unwrap();
    assert_eq!(sets.windows(4).filter(|w| *w == [0, 0, 0, 1]).count(), 3);
    assert!(video_parameter_sets(CodecKind::Hevc, &nal(19, &[0xaf])).is_none());
    check_video_dimensions(CodecKind::Hevc, &idr, size, FrameLimits::RESOURCE).unwrap();
    // 4:4:4 (Main 4:4:4) passes, a wrong size or an unknown level does not.
    let full = nal(33, &hevc_sps(120, 3));
    check_video_dimensions(CodecKind::Hevc, &full, size, FrameLimits::RESOURCE).unwrap();
    let other = FrameSize::new(1280, 720);
    assert!(check_video_dimensions(CodecKind::Hevc, &idr, other, FrameLimits::RESOURCE).is_err());
    let unknown = nal(33, &hevc_sps(121, 1));
    assert!(
        check_video_dimensions(CodecKind::Hevc, &unknown, size, FrameLimits::RESOURCE).is_err()
    );
    assert_eq!(HEVC_LEVELS[5].level_idc, 120);
}

#[test]
fn bildkette_task_codec_av1_marks_and_sequence_check() {
    let size = FrameSize::new(1920, 1080);
    let mut key = obu(2, &[]);
    key.extend(obu(1, &av1_sequence(8, 1920, 1080)));
    key.extend(av1_frame(0));
    assert!(video_keyframe(CodecKind::Av1, &key));
    let mut inter = obu(2, &[]);
    inter.extend(av1_frame(1));
    assert!(!video_keyframe(CodecKind::Av1, &inter));
    assert!(video_parameter_sets(CodecKind::Av1, &inter).is_none());
    let header = video_parameter_sets(CodecKind::Av1, &key).unwrap();
    assert_eq!(header[0] >> 3, 1, "the sequence header OBU");
    check_video_dimensions(CodecKind::Av1, &key, size, FrameLimits::RESOURCE).unwrap();
    // Level 2.0 (seq_level_idx 0) cannot carry 1080p; a smaller maximum
    // than the stream's size is refused; a truncated OBU is an error.
    let mut low = obu(1, &av1_sequence(0, 1920, 1080));
    low.extend(av1_frame(0));
    assert!(check_video_dimensions(CodecKind::Av1, &low, size, FrameLimits::RESOURCE).is_err());
    let small = obu(1, &av1_sequence(8, 1280, 720));
    assert!(check_video_dimensions(CodecKind::Av1, &small, size, FrameLimits::RESOURCE).is_err());
    assert!(check_video_dimensions(
        CodecKind::Av1,
        &key[..key.len() - 1],
        size,
        FrameLimits::RESOURCE
    )
    .is_err());
}

/// B-06 for HEVC and AV1: MaxBR of the highest level within the pairing's
/// sample rate, doubled for 4:4:4; the highest level without a report.
#[test]
fn bildkette_task_codec_hevc_av1_rate_ceiling() {
    // 1080p60 = 124 416 000 luma samples/s = 486 000 macroblocks/s: HEVC
    // level 4.1 (133 693 440 > rate, so 4.0 with 12 Mbit/s); AV1 4.0.
    let rate = Some(486_000);
    assert_eq!(
        video_max_bitrate_bps(CodecKind::Hevc, rate, false),
        12_000_000
    );
    assert_eq!(
        video_max_bitrate_bps(CodecKind::Hevc, rate, true),
        24_000_000
    );
    assert_eq!(
        video_max_bitrate_bps(CodecKind::Av1, rate, false),
        12_000_000
    );
    assert_eq!(
        video_max_bitrate_bps(CodecKind::Hevc, None, false),
        240_000_000
    );
    assert_eq!(
        video_max_bitrate_bps(CodecKind::Av1, None, false),
        160_000_000
    );
    assert_eq!(
        video_max_bitrate_bps(CodecKind::H264, None, false),
        u64::MAX
    );
}
