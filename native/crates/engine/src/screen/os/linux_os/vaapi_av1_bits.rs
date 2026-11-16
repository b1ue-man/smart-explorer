//! AV1 OBUs and bit syntax for the VA-API AV1 coders (plan 2026-11-14
//! „Bildkette“ BK4b): splitting a temporal unit of the low-overhead
//! bitstream format into OBUs (AV1 Bitstream & Decoding Process
//! Specification v1.0.0 with Errata 1, 5.3 `obu_header`, 5.3.3 extension,
//! 4.10.5 `leb128`), writing OBUs with `obu_has_size_field`, the reading
//! descriptors `su`, `ns`, `le`, `uvlc` (4.10.3, 4.10.6, 4.10.7, 4.10.8)
//! on top of the MSB-first `BitReader` of `h264_bits.rs`, and the order of
//! a temporal unit the viewers expect (7.5: a temporal delimiter first,
//! then the sequence header of a key frame, then the frame).
//!
//! Format on the wire (BK6, `wire_binary.rs:14-17`): one temporal unit per
//! record in the low-overhead format with sizes on every OBU; the core's
//! `hevc_av1_bits::obus` reads exactly that, `video_keyframe` looks at the
//! first frame (header) OBU, `video_parameter_sets` takes the sequence
//! header OBU. dav1d, MediaCodec (`video/av01`) and the Media Foundation
//! AV1 decoder take a temporal unit in this format (assumptions A-DAV1D-
//! ORDER and A-MF-AV1-INPUT of BK6).
use std::io;

use super::h264_bits::BitReader;
use crate::screen::core::wire::invalid;

/// `obu_type` (AV1 6.2.2).
pub(super) const OBU_SEQUENCE_HEADER: u8 = 1;
pub(super) const OBU_TEMPORAL_DELIMITER: u8 = 2;
pub(super) const OBU_FRAME_HEADER: u8 = 3;
pub(super) const OBU_TILE_GROUP: u8 = 4;
pub(super) const OBU_FRAME: u8 = 6;
pub(super) const OBU_TILE_LIST: u8 = 8;
/// A temporal delimiter OBU with an empty payload (header 0x12, size 0).
pub(super) const TEMPORAL_DELIMITER: [u8; 2] = [OBU_TEMPORAL_DELIMITER << 3 | 0x02, 0];

/// One OBU of a temporal unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Obu<'a> {
    pub kind: u8,
    pub temporal_id: u8,
    pub spatial_id: u8,
    pub payload: &'a [u8],
    /// Header, size and payload.
    pub whole: &'a [u8],
}

/// `leb128()` at `at`: the value and its length (at most 8 bytes).
pub(super) fn read_leb128(data: &[u8], at: usize) -> io::Result<(u64, usize)> {
    let mut value = 0u64;
    for index in 0..8 {
        let byte = *data
            .get(at + index)
            .ok_or_else(|| invalid("truncated AV1 leb128"))?;
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err(invalid("AV1 leb128 longer than 8 bytes"))
}

/// The OBUs of `data`; an OBU without `obu_has_size_field` runs to the end.
pub(super) fn split_obus(data: &[u8]) -> io::Result<Vec<Obu<'_>>> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let header = data[at];
        if header & 0x80 != 0 {
            return Err(invalid("AV1 OBU forbidden bit set"));
        }
        let kind = (header >> 3) & 0x0f;
        let extension = header & 0x04 != 0;
        let has_size = header & 0x02 != 0;
        let (mut temporal_id, mut spatial_id) = (0, 0);
        let mut body = at + 1;
        if extension {
            let byte = *data
                .get(body)
                .ok_or_else(|| invalid("truncated AV1 OBU extension"))?;
            temporal_id = byte >> 5;
            spatial_id = (byte >> 3) & 3;
            body += 1;
        }
        let size = if has_size {
            let (size, used) = read_leb128(data, body)?;
            body += used;
            usize::try_from(size).map_err(|_| invalid("AV1 OBU size overflows"))?
        } else {
            data.len()
                .checked_sub(body)
                .ok_or_else(|| invalid("truncated AV1 OBU header"))?
        };
        let end = body
            .checked_add(size)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| invalid("AV1 OBU longer than its temporal unit"))?;
        found.push(Obu {
            kind,
            temporal_id,
            spatial_id,
            payload: &data[body..end],
            whole: &data[at..end],
        });
        at = end;
    }
    Ok(found)
}

/// `leb128` of `value` in the fewest bytes.
pub(super) fn leb128(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// `leb128` of `value` padded to four bytes (the `obu_size` field a driver
/// rewrites, `va_enc_av1.h` `size_in_bits_frame_hdr_obu`).
pub(super) fn leb128_fixed4(value: u32) -> [u8; 4] {
    [
        (value & 0x7f) as u8 | 0x80,
        ((value >> 7) & 0x7f) as u8 | 0x80,
        ((value >> 14) & 0x7f) as u8 | 0x80,
        ((value >> 21) & 0x7f) as u8,
    ]
}

/// An OBU of `kind` without extension, with its size field.
pub(super) fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(kind << 3) | 0x02];
    out.extend_from_slice(&leb128(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

/// The temporal unit of an encoder output in the order of AV1 7.5: the
/// temporal delimiter (inserted when the driver wrote none), for a key
/// frame the sequence header (the driver's, else `sequence`), then every
/// other OBU in the driver's order. Only OBUs with a size field are taken
/// (the core's `obus` and every viewer need them).
pub(super) fn ordered_temporal_unit(
    data: &[u8],
    key: bool,
    sequence: &[u8],
) -> io::Result<Vec<u8>> {
    let units = split_obus(data)?;
    if units.iter().any(|unit| unit.whole[0] & 0x02 == 0) {
        return Err(invalid("AV1 encoder wrote an OBU without its size"));
    }
    let mut out = Vec::with_capacity(data.len() + sequence.len() + 2);
    out.extend_from_slice(&TEMPORAL_DELIMITER);
    if key {
        match units.iter().find(|unit| unit.kind == OBU_SEQUENCE_HEADER) {
            Some(unit) => out.extend_from_slice(unit.whole),
            None => out.extend_from_slice(sequence),
        }
    }
    for unit in &units {
        if !matches!(unit.kind, OBU_TEMPORAL_DELIMITER | OBU_SEQUENCE_HEADER) {
            out.extend_from_slice(unit.whole);
        }
    }
    Ok(out)
}

/// `su(n)` (4.10.6).
pub(super) fn su(r: &mut BitReader<'_>, n: u32) -> io::Result<i32> {
    if n == 0 || n > 32 {
        return Err(invalid("AV1 su width out of range"));
    }
    let value = r.bits(n)? as i64;
    let sign = 1i64 << (n - 1);
    Ok(if value & sign != 0 {
        (value - 2 * sign) as i32
    } else {
        value as i32
    })
}

/// `ns(n)` (4.10.7).
pub(super) fn ns(r: &mut BitReader<'_>, n: u32) -> io::Result<u32> {
    if n <= 1 {
        return Ok(0);
    }
    let w = 32 - n.leading_zeros();
    let m = (1u64 << w) - u64::from(n);
    let v = u64::from(r.bits(w - 1)?);
    if v < m {
        return Ok(v as u32);
    }
    let extra = u64::from(r.bits(1)?);
    Ok(((v << 1) - m + extra) as u32)
}

/// `uvlc()` (4.10.3); values beyond `u32` are refused.
pub(super) fn uvlc(r: &mut BitReader<'_>) -> io::Result<u32> {
    let mut zeros = 0u32;
    while !r.flag()? {
        zeros += 1;
        if zeros >= 32 {
            return Err(invalid("AV1 uvlc out of range"));
        }
    }
    let rest = r.bits(zeros)?;
    u32::try_from((1u64 << zeros) - 1 + u64::from(rest)).map_err(|_| invalid("AV1 uvlc"))
}

/// `le(n)` (4.10.8) of the `n` bytes at `at`.
pub(super) fn le(data: &[u8], at: usize, n: usize) -> io::Result<u64> {
    if n > 8 {
        return Err(invalid("AV1 le width out of range"));
    }
    let end = at
        .checked_add(n)
        .ok_or_else(|| invalid("AV1 le offset overflow"))?;
    let bytes = data
        .get(at..end)
        .ok_or_else(|| invalid("truncated AV1 tile size"))?;
    Ok(bytes.iter().enumerate().fold(0u64, |sum, (index, byte)| {
        sum | (u64::from(*byte) << (8 * index))
    }))
}

/// `delta_q` of `read_delta_q()` (5.9.13): a flag and `su(1+6)`.
pub(super) fn delta_q(r: &mut BitReader<'_>) -> io::Result<i32> {
    if r.flag()? {
        su(r, 7)
    } else {
        Ok(0)
    }
}

/// `tile_log2(blkSize, target)` (5.9.15).
pub(super) fn tile_log2(block: u32, target: u32) -> u32 {
    if block == 0 {
        return 0;
    }
    let mut k = 0;
    while (u64::from(block) << k) < u64::from(target) {
        k += 1;
    }
    k
}
