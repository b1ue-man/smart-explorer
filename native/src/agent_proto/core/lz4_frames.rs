//! LZ4 block format of file content (`lz4-v1`, U44-10, E13-S3) and the wire
//! layout of the `DataLz4` and `ReadLz4` frames (tags 67 and 68). Only file
//! content is ever packed, never a path, a name, an error text or a listing.
//!
//! A block is one chunk of at most `CHUNK` raw bytes, packed on its own, with
//! no dictionary and no state shared with other blocks or files:
//! `[0][raw bytes]` or `[1][raw length: u32 le][LZ4 block]`. The receiver
//! refuses a raw length above `CHUNK` before it unpacks anything, unpacks
//! into a buffer of exactly the announced length, and reports `InvalidData`
//! when the unpacked length differs or the block is cut off. Nothing in a
//! block depends on another one, so a CRIME-style oracle (attacker bytes
//! next to a secret in one context) cannot arise; an observer learns at most
//! how well single 256 KiB blocks pack.
use std::io;
use std::time::{Duration, Instant};

use lz4_flex::block::{compress_into, decompress_into, get_maximum_output_size};

use crate::transfer::compress_policy::{Policy, PROBE_BYTES};

use super::super::super::types::{Frame, CHUNK};
use super::super::{bad, Reader};
use super::{add_len, put_bytes, put_str, put_u64, string_len};

/// Largest raw length of a block: the receive buffer, `CHUNK`.
pub(crate) const MAX_RAW: usize = CHUNK;
const KIND_RAW: u8 = 0;
const KIND_LZ4: u8 = 1;
/// Kind byte and raw length of an LZ4 block.
const LZ4_HEADER: usize = 5;
/// Largest block on the wire (a raw block is one byte longer than its data).
pub(crate) const MAX_BLOCK: usize = CHUNK + LZ4_HEADER;

/// The raw block: kind byte and the data.
pub(crate) fn pack_raw(raw: &[u8]) -> Vec<u8> {
    let mut block = Vec::with_capacity(raw.len() + 1);
    block.push(KIND_RAW);
    block.extend_from_slice(raw);
    block
}

/// The LZ4 block of `raw`, or `None` when it would not be smaller than the
/// raw block (incompressible data is sent raw).
pub(crate) fn pack_lz4(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.is_empty() || raw.len() > MAX_RAW {
        return None;
    }
    let mut block = vec![0u8; LZ4_HEADER + get_maximum_output_size(raw.len())];
    let written = compress_into(raw, &mut block[LZ4_HEADER..]).ok()?;
    if LZ4_HEADER + written > raw.len() {
        return None;
    }
    block[0] = KIND_LZ4;
    block[1..LZ4_HEADER].copy_from_slice(&(raw.len() as u32).to_le_bytes());
    block.truncate(LZ4_HEADER + written);
    Some(block)
}

/// The raw bytes of a block. Any deviation is `InvalidData`: an unknown kind,
/// a raw length of zero or above `MAX_RAW` (refused before unpacking), a cut
/// block, or an unpacked length different from the announced one.
pub(crate) fn unpack(block: &[u8]) -> io::Result<Vec<u8>> {
    let Some((&kind, rest)) = block.split_first() else {
        return Err(bad("empty LZ4 content block"));
    };
    match kind {
        KIND_RAW => {
            if rest.len() > MAX_RAW {
                return Err(bad("raw content block exceeds the protocol chunk size"));
            }
            Ok(rest.to_vec())
        }
        KIND_LZ4 => {
            let length = declared_len(rest)?;
            let mut raw = vec![0u8; length];
            let written = decompress_into(&rest[4..], &mut raw)
                .map_err(|_| bad("LZ4 content block is corrupt or cut off"))?;
            if written != length {
                return Err(bad(
                    "LZ4 content block has a different length than announced",
                ));
            }
            Ok(raw)
        }
        _ => Err(bad("unknown content block kind")),
    }
}

/// The raw length an LZ4 block announces (after its kind byte).
fn declared_len(after_kind: &[u8]) -> io::Result<usize> {
    let bytes: [u8; 4] = after_kind
        .get(..4)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| bad("LZ4 content block is cut off"))?;
    let length = u32::from_le_bytes(bytes) as usize;
    if length == 0 || length > MAX_RAW {
        return Err(bad("LZ4 content block announces an invalid raw length"));
    }
    Ok(length)
}

/// Raw bytes a block carries, read from its header: what flow-control credit
/// is charged (the receiver's memory holds raw bytes), without unpacking.
pub(crate) fn raw_len(block: &[u8]) -> io::Result<usize> {
    match block.split_first() {
        Some((&KIND_RAW, rest)) => Ok(rest.len()),
        Some((&KIND_LZ4, rest)) => declared_len(rest),
        _ => Err(bad("unknown content block kind")),
    }
}

/// Cores of this machine that no process uses right now, from the sensors of
/// the transfer engine (`cores_available - process_cpu_cores`); one free core
/// is assumed while they cannot tell (the policy's default for an unknown
/// value).
pub(crate) fn free_cores() -> f64 {
    crate::transfer::sensors::latest()
        .free_cores()
        .unwrap_or(1.0)
}

/// Packs the content blocks of one stream direction: decides per block
/// whether to compress (`Policy`), samples the ratio and the core rate, and
/// learns from the writer how long the wire made it wait.
pub(crate) struct BlockSender {
    policy: Policy,
    /// Where the free cores come from (the sensors; a test fixes them).
    cores: fn() -> f64,
}

impl Default for BlockSender {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockSender {
    pub(crate) fn new() -> Self {
        Self {
            policy: Policy::new(),
            cores: free_cores,
        }
    }

    /// A sender whose policy already has measurements and four free cores
    /// (tests: the result must not depend on the machine's load).
    #[cfg(test)]
    pub(crate) fn with_policy(policy: Policy) -> Self {
        Self {
            policy,
            cores: || 4.0,
        }
    }

    /// A new file (or range) starts on this stream: its first block is sampled.
    pub(crate) fn start_file(&mut self) {
        self.policy.start_file();
    }

    /// The LZ4 block for `raw` (at most `MAX_RAW` bytes) that starts at
    /// `file_offset` of its file, when the policy says to compress and the
    /// data shrinks. `None`: send the bytes as they are (a plain `Data`
    /// frame), which is also what an incompressible block gets.
    pub(crate) fn try_pack(&mut self, raw: &[u8], file_offset: u64) -> Option<Vec<u8>> {
        let probe = self.policy.probe_due(file_offset);
        if self.policy.decide(self.cores) {
            let started = Instant::now();
            let packed = pack_lz4(raw);
            let took = started.elapsed();
            let size = packed
                .as_ref()
                .map_or(raw.len(), |block| block.len() - LZ4_HEADER);
            self.policy.observe_compression(raw.len(), size, took);
            return packed;
        }
        if probe {
            // Only a sample is packed to measure; the bytes go out as they are.
            let sample = &raw[..raw.len().min(PROBE_BYTES)];
            let started = Instant::now();
            let packed = pack_lz4(sample);
            let took = started.elapsed();
            let size = packed
                .as_ref()
                .map_or(sample.len(), |block| block.len() - LZ4_HEADER);
            self.policy.observe_compression(sample.len(), size, took);
        }
        None
    }

    /// `bytes` of raw content went to the wire; the writer waited `blocked`.
    pub(crate) fn record_send(&mut self, bytes: usize, blocked: Duration, now: Instant) {
        self.policy.record_send(bytes as u64, blocked, now);
    }
}

/// Encoded body length after the request id and tag, `None` for other frames.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::DataLz4(block) => {
            if block.len() > MAX_BLOCK {
                Err(bad("content block exceeds the protocol chunk size"))
            } else {
                Ok(4 + block.len())
            }
        }
        Frame::ReadLz4 { path, .. } => {
            let mut length = 16;
            string_len(path).and_then(|path| {
                add_len(&mut length, path)?;
                Ok(length)
            })
        }
        _ => return None,
    })
}

/// Append the tag and fields; false for other frames.
pub(in super::super) fn encode(frame: &Frame, b: &mut Vec<u8>) -> bool {
    match frame {
        Frame::DataLz4(block) => {
            b.push(67);
            put_bytes(b, block);
        }
        Frame::ReadLz4 { path, offset, len } => {
            b.push(68);
            put_str(b, path);
            put_u64(b, *offset);
            put_u64(b, *len);
        }
        _ => return false,
    }
    true
}

/// Decode a frame of this set; `None` for an unknown tag.
pub(in super::super) fn decode(tag: u8, r: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(Some(match tag {
        67 => Frame::DataLz4(r.bounded_bytes(MAX_BLOCK)?),
        68 => Frame::ReadLz4 {
            path: r.string()?,
            offset: r.u64()?,
            len: r.u64()?,
        },
        _ => return Ok(None),
    }))
}
