//! H.264 bit syntax for the Linux hardware codecs (BK4): Annex B NAL
//! splitting, emulation prevention (ITU-T H.264 (08/2021) 7.3.1 and 7.4.1),
//! fixed-width and Exp-Golomb codes (9.1) for reading and writing. Plain
//! portable code; the VA-API encoder writes its SPS/PPS with `BitWriter`,
//! the VA-API decoder parses SPS, PPS and slice headers with `BitReader`.
use std::io;

use crate::screen::core::wire::invalid;

/// NAL unit types of H.264 Table 7-1 that the hardware path looks at.
pub(super) const NAL_SLICE: u8 = 1;
pub(super) const NAL_IDR: u8 = 5;
pub(super) const NAL_SPS: u8 = 7;
pub(super) const NAL_PPS: u8 = 8;

/// The NAL units of an Annex B access unit, each without its start code
/// (the first byte is the NAL header). Three- and four-byte start codes;
/// trailing zero bytes before the next start code belong to no NAL unit.
pub(super) fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut at = 0;
    while at + 3 <= data.len() {
        if data[at] == 0 && data[at + 1] == 0 && data[at + 2] == 1 {
            starts.push(at + 3);
            at += 3;
        } else {
            at += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (index, &start) in starts.iter().enumerate() {
        let mut end = starts
            .get(index + 1)
            .map_or(data.len(), |next| next.saturating_sub(3));
        while end > start && data[end - 1] == 0 {
            end -= 1;
        }
        if end > start {
            units.push(&data[start..end]);
        }
    }
    units
}

/// `nal_unit_type` of a NAL unit (its header's low five bits).
pub(super) fn nal_type(unit: &[u8]) -> u8 {
    unit.first().map_or(0, |header| header & 0x1f)
}

/// Whether the Annex B bytes contain a NAL unit of `kind`.
pub(super) fn contains_nal(data: &[u8], kind: u8) -> bool {
    nal_units(data).iter().any(|unit| nal_type(unit) == kind)
}

/// The SPS and PPS units of an access unit as Annex B bytes (four-byte
/// start codes), `None` without both.
pub(super) fn parameter_sets(data: &[u8]) -> Option<Vec<u8>> {
    let units = nal_units(data);
    let wanted = |kind: u8| units.iter().any(|unit| nal_type(unit) == kind);
    if !wanted(NAL_SPS) || !wanted(NAL_PPS) {
        return None;
    }
    let mut out = Vec::new();
    for unit in units
        .iter()
        .filter(|unit| matches!(nal_type(unit), NAL_SPS | NAL_PPS))
    {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(unit);
    }
    Some(out)
}

/// The RBSP of a NAL unit payload (without its header byte): emulation
/// prevention bytes `00 00 03` lose the `03` (7.4.1).
pub(super) fn unescape(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    let mut zeros = 0;
    for &byte in payload {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

/// Reads an RBSP bit by bit, most significant bit first.
pub(super) struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    /// Bits consumed so far.
    pub(super) fn position(&self) -> usize {
        self.bit
    }

    pub(super) fn flag(&mut self) -> io::Result<bool> {
        Ok(self.bits(1)? == 1)
    }

    /// `u(n)` for `n` ≤ 32.
    pub(super) fn bits(&mut self, count: u32) -> io::Result<u32> {
        if count > 32 {
            return Err(invalid("H.264 field wider than 32 bits"));
        }
        let mut value = 0u32;
        for _ in 0..count {
            let byte = *self
                .data
                .get(self.bit / 8)
                .ok_or_else(|| invalid("H.264 header ends early"))?;
            let bit = (byte >> (7 - (self.bit % 8))) & 1;
            value = (value << 1) | u32::from(bit);
            self.bit += 1;
        }
        Ok(value)
    }

    /// `ue(v)` (9.1), at most 32 leading zero bits.
    pub(super) fn ue(&mut self) -> io::Result<u32> {
        let mut zeros = 0u32;
        while !self.flag()? {
            zeros += 1;
            if zeros > 31 {
                return Err(invalid("H.264 Exp-Golomb code too long"));
            }
        }
        let rest = self.bits(zeros)?;
        Ok(((1u64 << zeros) - 1 + u64::from(rest)) as u32)
    }

    /// `se(v)` (9.1.1).
    pub(super) fn se(&mut self) -> io::Result<i32> {
        let code = i64::from(self.ue()?);
        let value = if code & 1 == 1 {
            (code + 1) / 2
        } else {
            -(code / 2)
        };
        i32::try_from(value).map_err(|_| invalid("H.264 signed code out of range"))
    }

    /// `more_rbsp_data()` (7.2): another syntax element before the stop bit.
    pub(super) fn more_rbsp_data(&self) -> bool {
        let Some(last) = self.data.iter().rposition(|byte| *byte != 0) else {
            return false;
        };
        let stop = last * 8 + 7 - self.data[last].trailing_zeros() as usize;
        self.bit < stop
    }
}

/// Writes an RBSP, most significant bit first.
#[derive(Default)]
pub(super) struct BitWriter {
    bytes: Vec<u8>,
    bits: u32,
}

impl BitWriter {
    /// Bits written so far (BK4b: the AV1 header offsets).
    pub(super) fn position(&self) -> u32 {
        self.bits
    }

    pub(super) fn bits(&mut self, count: u32, value: u32) {
        for index in (0..count).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = ((value >> index) & 1) as u8;
            if let Some(last) = self.bytes.last_mut() {
                *last |= bit << (7 - (self.bits % 8));
            }
            self.bits += 1;
        }
    }

    pub(super) fn flag(&mut self, on: bool) {
        self.bits(1, u32::from(on));
    }

    pub(super) fn ue(&mut self, value: u32) {
        let code = u64::from(value) + 1;
        let length = 64 - code.leading_zeros();
        self.bits(length - 1, 0);
        // `code` has `length` ≤ 33 bits: the top one goes first, alone.
        if length == 33 {
            self.bits(1, 1);
            self.bits(32, code as u32);
        } else {
            self.bits(length, code as u32);
        }
    }

    pub(super) fn se(&mut self, value: i32) {
        let code = if value > 0 {
            (i64::from(value) * 2 - 1) as u32
        } else {
            (-i64::from(value) * 2) as u32
        };
        self.ue(code);
    }

    /// `rbsp_trailing_bits()` and the RBSP bytes.
    pub(super) fn finish(mut self) -> Vec<u8> {
        self.flag(true);
        while !self.bits.is_multiple_of(8) {
            self.flag(false);
        }
        self.bytes
    }
}

/// A NAL unit in Annex B form: four-byte start code, header byte, payload
/// with emulation prevention bytes inserted (7.4.1).
pub(super) fn annex_b_nal(nal_ref_idc: u8, nal_type: u8, rbsp: &[u8]) -> Vec<u8> {
    annex_b_unit(&[((nal_ref_idc & 3) << 5) | (nal_type & 0x1f)], rbsp)
}

/// A NAL unit with the header bytes `header` (one for H.264, two for
/// HEVC, H.265 7.3.1.2) in Annex B form; the emulation prevention rule is
/// the same in both standards (H.264 7.4.1, H.265 7.4.2).
pub(super) fn annex_b_unit(header: &[u8], rbsp: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 1];
    out.extend_from_slice(header);
    let mut zeros = 0;
    for &byte in rbsp {
        if zeros >= 2 && byte <= 3 {
            out.push(3);
            zeros = 0;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}
