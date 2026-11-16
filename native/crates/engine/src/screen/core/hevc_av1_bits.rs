//! Bit-level reading of HEVC NAL units and AV1 OBUs (plan 2026-11-14
//! „Bildkette“ BK6). Only what the screen contract needs: the units of an
//! Annex B byte stream (ITU-T H.265 (08/2021) Annex B.2, start code
//! `00 00 01`), the removal of emulation prevention bytes (7.4.2), the
//! fixed-length and Exp-Golomb reads (7.2, 9.2), and the OBUs of the AV1
//! low-overhead bitstream format (AV1 Bitstream & Decoding Process
//! Specification v1.0.0 with Errata 1, 5.2 and 5.3: `obu_header`,
//! `leb128` sizes, 4.10.5). Everything is bounded: a malformed unit is an
//! error, never a panic or an unbounded allocation.
use std::io;

use super::super::wire::invalid;

/// HEVC `nal_unit_type` of the parameter sets (H.265 Table 7-1).
pub(super) const HEVC_VPS: u8 = 32;
pub(super) const HEVC_SPS: u8 = 33;
pub(super) const HEVC_PPS: u8 = 34;
/// HEVC `IDR_W_RADL` and `IDR_N_LP` (Table 7-1): a picture group starts here.
pub(super) const HEVC_IDR_W_RADL: u8 = 19;
pub(super) const HEVC_IDR_N_LP: u8 = 20;
/// AV1 `obu_type` values (AV1 6.2.2, Table "obu_type").
pub(super) const OBU_SEQUENCE_HEADER: u8 = 1;
pub(super) const OBU_FRAME_HEADER: u8 = 3;
pub(super) const OBU_FRAME: u8 = 6;
/// Largest parameter set the screen accepts (as `check_avc_dimensions`).
pub(super) const MAX_PARAMETER_BYTES: usize = 65_536;

/// A big-endian bit reader over an RBSP or an OBU payload.
pub(super) struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Bits<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    /// `u(n)`/`f(n)` with `n` ≤ 32.
    pub(super) fn read(&mut self, count: usize) -> io::Result<u32> {
        if count > 32
            || self
                .at
                .checked_add(count)
                .is_none_or(|end| end > self.data.len() * 8)
        {
            return Err(invalid("truncated video parameter set"));
        }
        let mut value = 0u32;
        for _ in 0..count {
            let bit = (self.data[self.at / 8] >> (7 - self.at % 8)) & 1;
            value = (value << 1) | u32::from(bit);
            self.at += 1;
        }
        Ok(value)
    }

    pub(super) fn skip(&mut self, count: usize) -> io::Result<()> {
        let mut left = count;
        while left > 0 {
            let step = left.min(32);
            self.read(step)?;
            left -= step;
        }
        Ok(())
    }

    /// `ue(v)` (H.265 9.2), at most 32 leading zeros.
    pub(super) fn ue(&mut self) -> io::Result<u32> {
        let mut zeros = 0usize;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(invalid("Exp-Golomb code overflows"));
            }
        }
        let rest = self.read(zeros)?;
        (1u64 << zeros)
            .checked_sub(1)
            .map(|base| base + u64::from(rest))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| invalid("Exp-Golomb code overflows"))
    }

    /// AV1 `uvlc()` (4.10.3).
    pub(super) fn uvlc(&mut self) -> io::Result<u32> {
        // Every leading zero and the terminating one are read (4.10.3);
        // from 32 zeros on the value bits are absent and the value is
        // 2^32 - 1. The reader ends at the end of the data.
        let mut zeros = 0usize;
        while self.read(1)? == 0 {
            zeros += 1;
        }
        if zeros >= 32 {
            return Ok(u32::MAX);
        }
        let rest = self.read(zeros)?;
        Ok(((1u64 << zeros) - 1 + u64::from(rest)).min(u64::from(u32::MAX)) as u32)
    }
}

/// The NAL units of an Annex B byte stream, each without its start code.
pub(super) fn annex_b_units(data: &[u8]) -> Vec<&[u8]> {
    let start = |at: usize| data.get(at..at + 3) == Some(&[0, 0, 1]);
    let mut units = Vec::new();
    let mut at = 0;
    while at + 3 <= data.len() {
        if !start(at) {
            at += 1;
            continue;
        }
        let from = at + 3;
        let mut end = from;
        while end + 3 <= data.len() && !start(end) {
            end += 1;
        }
        let end = if end + 3 <= data.len() {
            end
        } else {
            data.len()
        };
        // A four-byte start code leaves one zero byte at the end of the unit.
        let mut last = end;
        while last > from && data[last - 1] == 0 && end != data.len() {
            last -= 1;
        }
        if last > from {
            units.push(&data[from..last]);
        }
        at = end;
    }
    units
}

/// `nal_unit_type` of an HEVC NAL unit (H.265 7.3.1.2).
pub(super) fn hevc_type(unit: &[u8]) -> Option<u8> {
    unit.first().map(|byte| (byte >> 1) & 0x3f)
}

/// The RBSP of a NAL unit after its `header` bytes: emulation prevention
/// bytes (`00 00 03`) removed (H.265 7.4.2).
pub(super) fn rbsp(unit: &[u8], header: usize) -> io::Result<Vec<u8>> {
    if unit.len() > MAX_PARAMETER_BYTES {
        return Err(invalid("video parameter set exceeds its budget"));
    }
    let body = unit
        .get(header..)
        .ok_or_else(|| invalid("NAL unit shorter than its header"))?;
    let mut out = Vec::with_capacity(body.len());
    let mut zeros = 0;
    for (index, &byte) in body.iter().enumerate() {
        if zeros >= 2 && byte == 3 {
            if body.get(index + 1).is_some_and(|next| *next > 3) {
                return Err(invalid("invalid emulation prevention"));
            }
            zeros = 0;
            continue;
        }
        out.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    Ok(out)
}

/// One AV1 OBU: its type, its payload and its whole bytes (header included).
#[derive(Clone, Copy, Debug)]
pub(super) struct Obu<'a> {
    pub kind: u8,
    pub payload: &'a [u8],
    pub whole: &'a [u8],
}

/// `leb128()` (AV1 4.10.5) at `at`; the value and the bytes it took.
fn leb128(data: &[u8], at: usize) -> io::Result<(u64, usize)> {
    let mut value = 0u64;
    for index in 0..8 {
        let byte = *data
            .get(at + index)
            .ok_or_else(|| invalid("truncated AV1 OBU size"))?;
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err(invalid("AV1 OBU size longer than 8 bytes"))
}

/// The OBUs of one temporal unit in the low-overhead format (AV1 5.2); an
/// OBU without `obu_has_size_field` runs to the end of the data.
pub(super) fn obus(data: &[u8]) -> io::Result<Vec<Obu<'_>>> {
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
        let mut body = at + 1 + usize::from(extension);
        if body > data.len() {
            return Err(invalid("truncated AV1 OBU header"));
        }
        let size = if has_size {
            let (size, used) = leb128(data, body)?;
            body += used;
            usize::try_from(size).map_err(|_| invalid("AV1 OBU size overflows"))?
        } else {
            data.len() - body
        };
        let end = body
            .checked_add(size)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| invalid("AV1 OBU longer than its temporal unit"))?;
        found.push(Obu {
            kind,
            payload: &data[body..end],
            whole: &data[at..end],
        });
        at = end;
    }
    Ok(found)
}
