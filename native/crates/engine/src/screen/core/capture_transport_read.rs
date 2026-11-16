//! Reads one owned worker picture with its decoding (plan 2026-11-14
//! „Bildkette“ BK3b, copy 15 of the BK3 table): the records of an encoded
//! picture go from the pipe straight into their own admitted buffers
//! (`CaptureOwner::CodecOutput`), without a transport buffer they would be
//! copied out of. The checks are those of `read_owned` and `decode_owned`:
//! the grant's generation, the frame bound, the record count, each record's
//! limit, the sum of the records and no byte beyond them. A still image
//! (`RAW`) keeps the transport buffer, from which its pixels are adopted.
use super::*;
use crate::screen::core::frame_size_caps::PacketBudget;

/// Capture time, key frame, hardware, source size and name length.
const FIXED: usize = 8 + 1 + 1 + 8 + 2;
/// Longest encoder name.
const NAME: usize = 256;

pub fn read_owned_reply(
    reader: &mut impl Read,
    length: usize,
    contract: Option<FrameContract>,
    lookup: impl FnOnce(u64) -> io::Result<CaptureGrant>,
) -> io::Result<WorkerReply> {
    if length < 58 {
        return Err(invalid("truncated owned worker prefix"));
    }
    let mut head = [0; 58];
    reader.read_exact(&mut head)?;
    let (id, capture_generation, generation, regions, limits, family_layout_revision, tag) =
        prefix(&head)?;
    let grant = lookup(id)?;
    if grant.generation().value() != capture_generation {
        return Err(invalid("owned worker grant generation mismatch"));
    }
    if tag == RAW {
        let bytes = read_rest(reader, &head, length, &grant, contract, limits, tag)?;
        return decode_owned(&bytes, &grant, contract);
    }
    let budget = PacketBudget::for_contract(contract)?;
    let body = length - 58;
    if body > budget.encoded_bytes {
        return Err(invalid("owned worker body exceeds its bound"));
    }
    let frame = read_records(reader, body, &grant, &budget)?;
    Ok(WorkerReply::Reference {
        generation,
        regions,
        limits,
        family_layout_revision,
        reply: Box::new(WorkerReply::Frame(frame)),
    })
}

/// The encoded picture of `body` bytes, its records read into their own
/// buffers.
fn read_records(
    reader: &mut impl Read,
    body: usize,
    grant: &CaptureGrant,
    budget: &PacketBudget,
) -> io::Result<StageFrame> {
    let mut fixed = [0u8; FIXED];
    reader.read_exact(&mut fixed)?;
    let mut head = BeReader::new(&fixed);
    let capture_ts_us = head.u64()?;
    let keyframe = head.u8()? != 0;
    let hardware = head.u8()? != 0;
    let source_size = size(&mut head)?;
    let name_len = usize::from(head.u16()?);
    if name_len > NAME {
        return Err(invalid("owned worker encoder name exceeds bound"));
    }
    let mut named = [0u8; NAME + 4];
    let records_at = FIXED + name_len + 4;
    let rest = body
        .checked_sub(records_at)
        .ok_or_else(|| invalid("truncated owned worker records"))?;
    reader.read_exact(&mut named[..name_len + 4])?;
    let name = std::str::from_utf8(&named[..name_len])
        .map_err(|_| invalid("worker encoder name is not UTF-8"))?;
    let count_bytes: [u8; 4] = named[name_len..name_len + 4]
        .try_into()
        .map_err(|_| invalid("worker record count is incomplete"))?;
    let count = u32::from_be_bytes(count_bytes) as usize;
    if count == 0 || count > budget.records {
        return Err(invalid("owned worker record count exceeds bound"));
    }
    let capacity = count
        .checked_mul(4)
        .and_then(|prefixes| rest.checked_sub(prefixes))
        .ok_or_else(|| invalid("owned worker records contradict their length"))?;
    if capacity > budget.encoded_bytes {
        return Err(invalid("owned worker records exceed frame budget"));
    }
    let records = CapturedRecords::from_allocator(grant, capacity as u64, count as u64, || {
        let mut records = Vec::new();
        records
            .try_reserve_exact(count)
            .map_err(|_| io::Error::other("worker record container allocation failed"))?;
        let mut left = capacity;
        for _ in 0..count {
            let mut prefix = [0u8; 4];
            reader.read_exact(&mut prefix)?;
            let length = u32::from_be_bytes(prefix) as usize;
            if length > budget.record_bytes || length > left {
                return Err(invalid("owned worker record exceeds accepted limit"));
            }
            left -= length;
            let mut record = Vec::new();
            record
                .try_reserve_exact(length)
                .map_err(|_| io::Error::other("worker record allocation failed"))?;
            // From the pipe into the record: the reader fills the reserved
            // capacity and stops at `length` (`Take`), so it never grows.
            let read = reader
                .by_ref()
                .take(length as u64)
                .read_to_end(&mut record)?;
            if read != length {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            records.push(record);
        }
        if left != 0 {
            return Err(invalid("owned worker records contradict their length"));
        }
        Ok(records)
    })?;
    Ok(StageFrame::Encoded {
        records,
        keyframe,
        capture_ts_us,
        encoder: StageEncoder {
            name: name.into(),
            hardware,
        },
        source_size,
    })
}

#[cfg(test)]
#[path = "bildkette_transport_task_tests.rs"]
mod transport_tests;
