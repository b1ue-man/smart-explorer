//! Stream structure for the run-time codec check (plan 2026-11-17
//! „Windows-Bild“ P3, `codec_verify.rs`) and the key-frame rule D8 of the
//! Media Foundation encoders: how many coded pictures one access unit
//! carries, whether it carries the parameter sets a fresh decoder needs, and
//! where they go when a key frame lacks them.
//!
//! - Pictures: H.264 slices (`nal_unit_type` 1 and 5) with
//!   `first_mb_in_slice` 0 (ITU-T H.264 7.3.3); HEVC VCL units (types 0–31)
//!   with `first_slice_segment_in_pic_flag` 1 (H.265 7.3.6.1); AV1 frame
//!   and frame-header OBUs (AV1 5.9, 5.10), at least the temporal delimiters.
//! - Parameter sets: H.264 SPS and PPS (types 7, 8), HEVC VPS, SPS and PPS,
//!   the AV1 sequence header (`video_parameter_sets`).
//! - D8: an encoder's own key-frame mark (Media Foundation
//!   `MFSampleExtension_CleanPoint`) counts only when the stream agrees
//!   (`video_keyframe`); a contradiction is an encoder failure. A key frame
//!   without parameter sets gets the last ones the encoder gave, else those
//!   of its output type (`MF_MT_MPEG_SEQUENCE_HEADER`), after a leading
//!   access unit delimiter (H.264 type 9, HEVC type 35) or temporal
//!   delimiter (AV1); without any it is an encoder failure.
//!
//! Child of `frame_size_caps`, so it reads with the same bounded NAL and OBU
//! readers (`hevc_av1_bits.rs`).
use std::io;

use super::super::negotiate::CodecKind;
use super::super::wire::invalid;
use super::bits::{annex_b_units, hevc_type, obus, Bits, OBU_FRAME, OBU_FRAME_HEADER};
use super::{video_keyframe, video_parameter_sets};

const H264_SLICE: u8 = 1;
const H264_IDR: u8 = 5;
const H264_SPS: u8 = 7;
const H264_PPS: u8 = 8;
const H264_AUD: u8 = 9;
const HEVC_AUD: u8 = 35;
const OBU_TEMPORAL_DELIMITER: u8 = 2;
/// Bytes of a slice header read for `first_mb_in_slice` (a `ue(v)` of at
/// most 33 bits).
const SLICE_HEAD: usize = 8;

/// The first `max` bytes of `unit` behind its `header` bytes, emulation
/// prevention bytes removed (H.264 7.4.1).
fn head(unit: &[u8], header: usize, max: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(max);
    let mut zeros = 0;
    for &byte in unit.iter().skip(header) {
        if out.len() >= max {
            break;
        }
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        out.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// The coded pictures that start in `data`, one access unit (H.264, HEVC)
/// or temporal unit (AV1) of `codec`; the tiles count one.
pub fn coded_pictures(codec: CodecKind, data: &[u8]) -> io::Result<usize> {
    match codec {
        CodecKind::H264 => {
            let mut count = 0;
            for unit in annex_b_units(data) {
                let kind = unit.first().map_or(0, |byte| byte & 31);
                if kind != H264_SLICE && kind != H264_IDR {
                    continue;
                }
                let bytes = head(unit, 1, SLICE_HEAD);
                if Bits::new(&bytes).ue()? == 0 {
                    count += 1;
                }
            }
            Ok(count)
        }
        CodecKind::Hevc => Ok(annex_b_units(data)
            .iter()
            .filter(|unit| hevc_type(unit).is_some_and(|kind| kind < 32))
            .filter(|unit| unit.get(2).is_some_and(|byte| byte & 0x80 != 0))
            .count()),
        CodecKind::Av1 => {
            let units = obus(data)?;
            let frames = units
                .iter()
                .filter(|obu| matches!(obu.kind, OBU_FRAME | OBU_FRAME_HEADER))
                .count();
            let delimiters = units
                .iter()
                .filter(|obu| obu.kind == OBU_TEMPORAL_DELIMITER)
                .count();
            Ok(frames.max(delimiters))
        }
        CodecKind::Tiles => Ok(1),
    }
}

/// The parameter sets of `data` a fresh decoder of `codec` needs, with
/// their start codes (H.264 SPS and PPS; HEVC and AV1 as
/// `video_parameter_sets`); `None` when one is missing.
pub fn stream_parameter_sets(codec: CodecKind, data: &[u8]) -> Option<Vec<u8>> {
    match codec {
        CodecKind::H264 => {
            let units = annex_b_units(data);
            let mut out = Vec::new();
            for kind in [H264_SPS, H264_PPS] {
                let unit = units
                    .iter()
                    .find(|unit| unit.first().is_some_and(|byte| byte & 31 == kind))?;
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(unit);
            }
            Some(out)
        }
        CodecKind::Hevc | CodecKind::Av1 => video_parameter_sets(codec, data),
        CodecKind::Tiles => None,
    }
}

/// D8: the key-frame mark of an encoder that marks its output itself
/// (`marked`) against the stream; a contradiction is an encoder failure.
pub fn marked_keyframe(codec: CodecKind, unit: &[u8], marked: bool) -> io::Result<bool> {
    if codec == CodecKind::Tiles {
        return Ok(marked);
    }
    match (marked, video_keyframe(codec, unit)) {
        (true, false) => Err(invalid(
            "the encoder marks a key frame that the stream does not start",
        )),
        (false, true) => Err(invalid(
            "the encoder gives a key frame of the stream without its mark",
        )),
        (_, stream) => Ok(stream),
    }
}

/// The index of every start code of `data` (a four-byte code from its
/// leading zero).
fn start_codes(data: &[u8]) -> Vec<usize> {
    let mut found = Vec::new();
    let mut at = 0;
    while at + 3 <= data.len() {
        if data[at..at + 3] == [0, 0, 1] {
            found.push(if at > 0 && data[at - 1] == 0 {
                at - 1
            } else {
                at
            });
            at += 3;
        } else {
            at += 1;
        }
    }
    found
}

/// Where parameter sets go in `unit`: behind a leading access unit
/// delimiter (H.264, HEVC) or AV1 temporal delimiter (OBU type 2 with a
/// size field of 0, AV1 5.3.1, 5.6), else at the start.
pub fn insertion_point(codec: CodecKind, unit: &[u8]) -> usize {
    match codec {
        CodecKind::Av1 => match unit {
            [header, 0, ..]
                if (header >> 3) & 0x0f == OBU_TEMPORAL_DELIMITER && header & 0x06 == 0x02 =>
            {
                2
            }
            _ => 0,
        },
        CodecKind::H264 | CodecKind::Hevc => {
            let units = annex_b_units(unit);
            let delimiter = units.first().is_some_and(|first| match codec {
                CodecKind::H264 => first.first().is_some_and(|byte| byte & 31 == H264_AUD),
                _ => hevc_type(first) == Some(HEVC_AUD),
            });
            if !delimiter {
                return 0;
            }
            start_codes(unit).get(1).copied().unwrap_or(unit.len())
        }
        CodecKind::Tiles => 0,
    }
}

/// What one access unit needs before it goes out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// The unit starts a picture group in the stream.
    pub keyframe: bool,
    /// Parameter sets to insert (empty: none).
    pub prefix: Vec<u8>,
    /// Where `prefix` goes in the unit.
    pub at: usize,
}

/// D8: the parameter sets a key frame of `codec` lacks, from `kept` (the
/// last ones the encoder gave) or `fallback` (the encoder's output type);
/// parameter sets in `unit` replace `kept`. A first key frame without any
/// is an error.
pub fn complete_unit(
    codec: CodecKind,
    unit: &[u8],
    kept: &mut Option<Vec<u8>>,
    fallback: Option<&[u8]>,
) -> io::Result<Completion> {
    if codec == CodecKind::Tiles {
        return Ok(Completion {
            keyframe: false,
            prefix: Vec::new(),
            at: 0,
        });
    }
    let keyframe = video_keyframe(codec, unit);
    let own = stream_parameter_sets(codec, unit);
    let prefix = if own.is_none() && keyframe {
        let known = kept
            .clone()
            .or_else(|| fallback.and_then(|bytes| stream_parameter_sets(codec, bytes)))
            .ok_or_else(|| {
                invalid("the encoder gave its first key frame without parameter sets")
            })?;
        *kept = Some(known.clone());
        known
    } else {
        Vec::new()
    };
    if own.is_some() {
        *kept = own;
    }
    let at = if prefix.is_empty() {
        0
    } else {
        insertion_point(codec, unit)
    };
    Ok(Completion {
        keyframe,
        prefix,
        at,
    })
}

/// `unit` with the completion applied (the non-admitted encoder paths).
pub fn completed(unit: &[u8], completion: &Completion) -> Vec<u8> {
    let at = completion.at.min(unit.len());
    let mut out = Vec::with_capacity(unit.len() + completion.prefix.len());
    out.extend_from_slice(&unit[..at]);
    out.extend_from_slice(&completion.prefix);
    out.extend_from_slice(&unit[at..]);
    out
}
