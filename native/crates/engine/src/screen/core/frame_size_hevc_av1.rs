//! Levels, stream marks and parameter checks of HEVC and AV1 (plan
//! 2026-11-14 „Bildkette“ BK6, B-06, B-30).
//!
//! Levels: HEVC from ITU-T H.265 (08/2021) Table A.8 (general tier and
//! level limits: MaxLumaPs) and Table A.9 (MaxLumaSr, MaxBR of Main and
//! High tier in units of 1000 bit/s for CpbVclFactor 1000, i.e. Main; Main
//! 4:4:4 of the range extensions has CpbVclFactor 2000, Table A.3); AV1 from
//! the AV1 Bitstream & Decoding Process Specification v1.0.0 with Errata 1,
//! Annex A.3 (MaxPicSize, MaxHSize, MaxVSize, MaxDisplayRate, MainMbps,
//! HighMbps; MaxBitrate = Mbps × BitrateProfileFactor 1.0 for Main, 2.0 for
//! High). The table values are taken from the standards as known to the
//! author; they were not re-read in this session (assumption A-BK6-LEVELS
//! in the BK6 report).
//!
//! Stream marks: an HEVC picture group starts with an IDR (`IDR_W_RADL`,
//! `IDR_N_LP`) and carries VPS, SPS and PPS; an AV1 group starts with a
//! temporal unit whose first frame header has `frame_type` KEY_FRAME
//! (AV1 5.9.2) and that carries the sequence header OBU.
//!
//! Parameter checks: before a native decoder allocates, every in-band
//! HEVC SPS (H.265 7.3.2.2) and AV1 sequence header (AV1 5.5.1) must name
//! a picture within the frame contract and within its level, as
//! `check_avc_dimensions` does for H.264. The screen contract is 8-bit,
//! progressive, 4:2:0 or 4:4:4.
use std::io;

use super::super::negotiate::CodecKind;
use super::super::wire::invalid;
use super::bits::{
    annex_b_units, hevc_type, obus, rbsp, Bits, HEVC_IDR_N_LP, HEVC_IDR_W_RADL, HEVC_PPS, HEVC_SPS,
    HEVC_VPS, OBU_FRAME, OBU_FRAME_HEADER, OBU_SEQUENCE_HEADER,
};
use super::FrameSize;

/// One HEVC level: `general_level_idc` (30 × level), MaxLumaPs, MaxLumaSr,
/// MaxBR of Main and High tier (1000 bit/s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HevcLevel {
    pub level_idc: u8,
    pub max_luma_ps: u64,
    pub max_luma_sr: u64,
    pub max_br_main: u64,
    pub max_br_high: Option<u64>,
}

const fn hevc(idc: u8, ps: u64, sr: u64, main: u64, high: Option<u64>) -> HevcLevel {
    HevcLevel {
        level_idc: idc,
        max_luma_ps: ps,
        max_luma_sr: sr,
        max_br_main: main,
        max_br_high: high,
    }
}

/// H.265 Tables A.8 and A.9.
pub const HEVC_LEVELS: [HevcLevel; 13] = [
    hevc(30, 36_864, 552_960, 128, None),
    hevc(60, 122_880, 3_686_400, 1_500, None),
    hevc(63, 245_760, 7_372_800, 3_000, None),
    hevc(90, 552_960, 16_588_800, 6_000, None),
    hevc(93, 983_040, 33_177_600, 10_000, None),
    hevc(120, 2_228_224, 66_846_720, 12_000, Some(30_000)),
    hevc(123, 2_228_224, 133_693_440, 20_000, Some(50_000)),
    hevc(150, 8_912_896, 267_386_880, 25_000, Some(100_000)),
    hevc(153, 8_912_896, 534_773_760, 40_000, Some(160_000)),
    hevc(156, 8_912_896, 1_069_547_520, 60_000, Some(240_000)),
    hevc(180, 35_651_584, 1_069_547_520, 60_000, Some(240_000)),
    hevc(183, 35_651_584, 2_139_095_040, 120_000, Some(480_000)),
    hevc(186, 35_651_584, 4_278_190_080, 240_000, Some(800_000)),
];

impl HevcLevel {
    /// Whether `size` at `fps` (0: any rate) fits: luma samples, the side
    /// limit √(8 × MaxLumaPs) (A.4.1) and the sample rate.
    pub fn supports(&self, size: FrameSize, fps: u32) -> bool {
        let side = ((8 * self.max_luma_ps) as f64).sqrt() as u64;
        size.pixels() <= self.max_luma_ps
            && u64::from(size.width) <= side
            && u64::from(size.height) <= side
            && (fps == 0 || size.pixels() * u64::from(fps) <= self.max_luma_sr)
    }
}

/// The lowest HEVC level that carries `size` at `fps`.
pub fn hevc_level_for(size: FrameSize, fps: u32) -> Option<HevcLevel> {
    HEVC_LEVELS
        .iter()
        .copied()
        .find(|level| level.supports(size, fps))
}

/// One AV1 level of Annex A.3: `seq_level_idx` and its limits (Mbps in
/// units of 1000 bit/s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Av1Level {
    pub seq_level_idx: u8,
    pub max_pic_size: u64,
    pub max_h_size: u32,
    pub max_v_size: u32,
    pub max_display_rate: u64,
    pub main_kbps: u64,
    pub high_kbps: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
const fn av1(
    idx: u8,
    pic: u64,
    h: u32,
    v: u32,
    rate: u64,
    main: u64,
    high: Option<u64>,
) -> Av1Level {
    Av1Level {
        seq_level_idx: idx,
        max_pic_size: pic,
        max_h_size: h,
        max_v_size: v,
        max_display_rate: rate,
        main_kbps: main,
        high_kbps: high,
    }
}

/// AV1 Annex A.3, the defined levels 2.0 to 6.3.
pub const AV1_LEVELS: [Av1Level; 14] = [
    av1(0, 147_456, 2048, 1152, 4_423_680, 1_500, None),
    av1(1, 278_784, 2816, 1584, 8_363_520, 3_000, None),
    av1(4, 665_856, 4352, 2448, 19_975_680, 6_000, None),
    av1(5, 1_065_024, 5504, 3096, 31_950_720, 10_000, None),
    av1(8, 2_359_296, 6144, 3456, 70_778_880, 12_000, Some(30_000)),
    av1(9, 2_359_296, 6144, 3456, 141_557_760, 20_000, Some(50_000)),
    av1(
        12,
        8_912_896,
        8192,
        4352,
        267_386_880,
        30_000,
        Some(100_000),
    ),
    av1(
        13,
        8_912_896,
        8192,
        4352,
        534_773_760,
        40_000,
        Some(160_000),
    ),
    av1(
        14,
        8_912_896,
        8192,
        4352,
        1_069_547_520,
        60_000,
        Some(240_000),
    ),
    av1(
        15,
        8_912_896,
        8192,
        4352,
        1_069_547_520,
        60_000,
        Some(240_000),
    ),
    av1(
        16,
        35_651_584,
        16384,
        8704,
        1_069_547_520,
        60_000,
        Some(240_000),
    ),
    av1(
        17,
        35_651_584,
        16384,
        8704,
        2_139_095_040,
        100_000,
        Some(480_000),
    ),
    av1(
        18,
        35_651_584,
        16384,
        8704,
        4_278_190_080,
        160_000,
        Some(800_000),
    ),
    av1(
        19,
        35_651_584,
        16384,
        8704,
        4_278_190_080,
        160_000,
        Some(800_000),
    ),
];

impl Av1Level {
    pub fn supports(&self, size: FrameSize, fps: u32) -> bool {
        size.pixels() <= self.max_pic_size
            && size.width <= self.max_h_size
            && size.height <= self.max_v_size
            && (fps == 0 || size.pixels() * u64::from(fps) <= self.max_display_rate)
    }
}

/// The lowest AV1 level that carries `size` at `fps`.
pub fn av1_level_for(size: FrameSize, fps: u32) -> Option<Av1Level> {
    AV1_LEVELS
        .iter()
        .copied()
        .find(|level| level.supports(size, fps))
}

/// The bit-rate ceiling of an HEVC or AV1 pairing (B-06): MaxBR of the
/// highest level within the pairing's sample rate (`macroblocks_per_second`
/// × 256 luma samples), the highest level when none is reported; Main tier
/// and Main profile, doubled for the 4:4:4 profile (HEVC CpbVclFactor
/// 2000/1000, AV1 BitrateProfileFactor 2.0).
pub fn video_max_bitrate_bps(
    codec: CodecKind,
    macroblocks_per_second: Option<u64>,
    chroma_444: bool,
) -> u64 {
    let samples = macroblocks_per_second.map(|rate| rate.saturating_mul(256));
    let kbps = match codec {
        CodecKind::Hevc => {
            HEVC_LEVELS
                .iter()
                .rev()
                .find(|level| samples.is_none_or(|rate| level.max_luma_sr <= rate))
                .unwrap_or(&HEVC_LEVELS[0])
                .max_br_main
        }
        CodecKind::Av1 => {
            AV1_LEVELS
                .iter()
                .rev()
                .find(|level| samples.is_none_or(|rate| level.max_display_rate <= rate))
                .unwrap_or(&AV1_LEVELS[0])
                .main_kbps
        }
        CodecKind::H264 | CodecKind::Tiles => return u64::MAX,
    };
    kbps.saturating_mul(1000)
        .saturating_mul(if chroma_444 { 2 } else { 1 })
}

/// Whether the encoded picture `data` of `codec` starts a picture group.
pub fn video_keyframe(codec: CodecKind, data: &[u8]) -> bool {
    match codec {
        CodecKind::H264 => annex_b_units(data)
            .iter()
            .any(|unit| unit.first().is_some_and(|byte| byte & 31 == 5)),
        CodecKind::Hevc => annex_b_units(data)
            .iter()
            .any(|unit| matches!(hevc_type(unit), Some(HEVC_IDR_W_RADL | HEVC_IDR_N_LP))),
        CodecKind::Av1 => obus(data).is_ok_and(|units| {
            units
                .iter()
                .find(|obu| matches!(obu.kind, OBU_FRAME | OBU_FRAME_HEADER))
                .is_some_and(|obu| av1_key_frame(obu.payload))
        }),
        CodecKind::Tiles => false,
    }
}

/// `show_existing_frame` 0 and `frame_type` KEY_FRAME (AV1 5.9.2), for a
/// sequence without `reduced_still_picture_header` (no se encoder sets it).
fn av1_key_frame(payload: &[u8]) -> bool {
    let mut bits = Bits::new(payload);
    matches!((bits.read(1), bits.read(2)), (Ok(0), Ok(0)))
}

/// The parameter sets of `data`: HEVC VPS, SPS and PPS with start codes,
/// the AV1 sequence header OBU; `None` when `data` lacks one of them.
pub fn video_parameter_sets(codec: CodecKind, data: &[u8]) -> Option<Vec<u8>> {
    match codec {
        CodecKind::Hevc => {
            let units = annex_b_units(data);
            let mut out = Vec::new();
            for kind in [HEVC_VPS, HEVC_SPS, HEVC_PPS] {
                let unit = units.iter().find(|unit| hevc_type(unit) == Some(kind))?;
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(unit);
            }
            Some(out)
        }
        CodecKind::Av1 => obus(data)
            .ok()?
            .into_iter()
            .find(|obu| obu.kind == OBU_SEQUENCE_HEADER)
            .map(|obu| obu.whole.to_vec()),
        CodecKind::H264 | CodecKind::Tiles => None,
    }
}

/// Checks every in-band parameter set of `data` against `size` and
/// `limits` before a native decoder sees it.
pub fn check_video_dimensions(
    codec: CodecKind,
    data: &[u8],
    size: FrameSize,
    limits: super::super::frame_budget::FrameLimits,
) -> io::Result<()> {
    match codec {
        CodecKind::H264 => super::check_avc_dimensions(data, size, limits),
        CodecKind::Tiles => limits.check(size).map(|_| ()),
        CodecKind::Hevc => {
            limits.check(size)?;
            for unit in annex_b_units(data) {
                if hevc_type(unit) == Some(HEVC_SPS) {
                    check_hevc_sps(&rbsp(unit, 2)?, size, limits)?;
                }
            }
            Ok(())
        }
        CodecKind::Av1 => {
            limits.check(size)?;
            for obu in obus(data)? {
                if obu.kind == OBU_SEQUENCE_HEADER {
                    check_av1_sequence(obu.payload, size, limits)?;
                }
            }
            Ok(())
        }
    }
}

/// `profile_tier_level(1, max_sub_layers_minus1)` (H.265 7.3.3); the level.
fn hevc_profile_tier_level(bits: &mut Bits<'_>, sub_layers: usize) -> io::Result<u32> {
    bits.skip(2 + 1 + 5 + 32 + 4 + 43 + 1)?;
    let level = bits.read(8)?;
    let mut present = [(false, false); 8];
    for flags in present.iter_mut().take(sub_layers) {
        *flags = (bits.read(1)? == 1, bits.read(1)? == 1);
    }
    if sub_layers > 0 {
        bits.skip(2 * (8 - sub_layers))?;
    }
    for (profile, level) in present.iter().take(sub_layers) {
        if *profile {
            bits.skip(88)?;
        }
        if *level {
            bits.skip(8)?;
        }
    }
    Ok(level)
}

fn check_hevc_sps(
    sps: &[u8],
    size: FrameSize,
    limits: super::super::frame_budget::FrameLimits,
) -> io::Result<()> {
    let mut bits = Bits::new(sps);
    bits.read(4)?;
    let sub_layers = bits.read(3)? as usize;
    bits.read(1)?;
    let level_idc = hevc_profile_tier_level(&mut bits, sub_layers)?;
    if bits.ue()? > 15 {
        return Err(invalid("HEVC SPS id exceeds 15"));
    }
    let chroma = bits.ue()?;
    let (sub_width, sub_height) = match chroma {
        1 => (2, 2),
        3 => {
            if bits.read(1)? != 0 {
                return Err(invalid(
                    "separate colour planes are outside the screen contract",
                ));
            }
            (1, 1)
        }
        _ => return Err(invalid("screen HEVC requires 4:2:0 or 4:4:4")),
    };
    let width = bits.ue()?;
    let height = bits.ue()?;
    let mut window = [0u32; 4];
    if bits.read(1)? == 1 {
        for value in &mut window {
            *value = bits.ue()?;
        }
    }
    if bits.ue()? != 0 || bits.ue()? != 0 {
        return Err(invalid("screen HEVC requires 8-bit samples"));
    }
    let crop = |a: u32, b: u32, factor: u32| {
        a.checked_add(b)
            .and_then(|sum| sum.checked_mul(factor))
            .ok_or_else(|| invalid("HEVC conformance window overflows"))
    };
    let visible_w = width.checked_sub(crop(window[0], window[1], sub_width)?);
    let visible_h = height.checked_sub(crop(window[2], window[3], sub_height)?);
    let coded = FrameSize::new(width, height);
    let level = HEVC_LEVELS
        .iter()
        .find(|level| u32::from(level.level_idc) == level_idc)
        .ok_or_else(|| invalid("unknown HEVC level"))?;
    if visible_w != Some(size.width) || visible_h != Some(size.height) || !level.supports(coded, 0)
    {
        return Err(invalid(
            "HEVC SPS dimensions or level contradict the frame contract",
        ));
    }
    limits.check(coded).map(|_| ())
}

/// The leading fields of `sequence_header_obu` (AV1 5.5.1) up to
/// `max_frame_height_minus_1`.
fn check_av1_sequence(
    payload: &[u8],
    size: FrameSize,
    limits: super::super::frame_budget::FrameLimits,
) -> io::Result<()> {
    let mut bits = Bits::new(payload);
    if bits.read(3)? > 1 {
        return Err(invalid(
            "AV1 Professional profile is outside the screen contract",
        ));
    }
    bits.read(1)?;
    let reduced = bits.read(1)? == 1;
    let level = if reduced {
        bits.read(5)?
    } else {
        let timing = bits.read(1)? == 1;
        let mut decoder_model = false;
        let mut delay_bits = 0;
        if timing {
            bits.skip(64)?;
            if bits.read(1)? == 1 {
                bits.uvlc()?;
            }
            decoder_model = bits.read(1)? == 1;
            if decoder_model {
                delay_bits = bits.read(5)? as usize + 1;
                bits.skip(32 + 5 + 5)?;
            }
        }
        let display_delay = bits.read(1)? == 1;
        let points = bits.read(5)? + 1;
        let mut first = None;
        for _ in 0..points {
            bits.read(12)?;
            let level = bits.read(5)?;
            if level > 7 {
                bits.read(1)?;
            }
            if decoder_model && bits.read(1)? == 1 {
                bits.skip(2 * delay_bits + 1)?;
            }
            if display_delay && bits.read(1)? == 1 {
                bits.read(4)?;
            }
            first.get_or_insert(level);
        }
        first.unwrap_or(31)
    };
    let width_bits = bits.read(4)? as usize + 1;
    let height_bits = bits.read(4)? as usize + 1;
    let width = bits.read(width_bits)?.saturating_add(1);
    let height = bits.read(height_bits)?.saturating_add(1);
    let coded = FrameSize::new(width, height);
    // seq_level_idx 31 names no level ("maximum parameters", A.3).
    let fits = level == 31
        || AV1_LEVELS
            .iter()
            .find(|entry| u32::from(entry.seq_level_idx) == level)
            .is_some_and(|entry| entry.supports(coded, 0));
    if width < size.width || height < size.height || !fits {
        return Err(invalid(
            "AV1 sequence header dimensions or level contradict the frame contract",
        ));
    }
    limits.check(coded).map(|_| ())
}

#[cfg(test)]
#[path = "bildkette_codec_bits_task_tests.rs"]
mod bildkette_codec_bits_task_tests;
