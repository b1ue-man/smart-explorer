//! Grant-bound worker picture framing. Fixed prefixes are read before a large
//! payload allocation, and encoding writes one final buffer without inner copies.
use super::capture_admission::CaptureGrant;
use super::capture_bytes::AdmittedBytes;
use super::capture_image::CaptureImageBuilder;
use super::capture_plan::CaptureOwner;
use super::capture_records::CapturedRecords;
use super::frame_budget::{FrameContract, FrameLimits};
use super::host_stage::{StageEncoder, StageFrame};
use super::pixel_convert::FrameSize;
use super::wire::{invalid, BeReader};
use super::worker_capture_wire::OWNED_REFERENCE;
use super::worker_wire::{self, WorkerReply, ENCODED, RAW};
use std::io::{self, Read, Write};

pub(crate) fn read_header(reader: &mut impl Read) -> io::Result<Option<(u8, usize)>> {
    let mut head = [0u8; 5];
    loop {
        match reader.read(&mut head[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    reader.read_exact(&mut head[1..])?;
    let length = u32::from_be_bytes(
        head[1..]
            .try_into()
            .map_err(|_| invalid("worker length is incomplete"))?,
    ) as usize;
    let limit = worker_wire::tag_limit(head[0]).ok_or_else(|| invalid("unknown worker frame"))?;
    if length > limit {
        return Err(invalid("worker frame exceeds tag limit"));
    }
    Ok(Some((head[0], length)))
}
pub(crate) fn read_control(reader: &mut impl Read, length: usize) -> io::Result<Vec<u8>> {
    if length > super::wire::MAX_WINDOWS_BYTES {
        return Err(invalid("unowned worker payload exceeds control bound"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn size(reader: &mut BeReader<'_>) -> io::Result<FrameSize> {
    let size = FrameSize::new(reader.u32()?, reader.u32()?);
    super::capture_plan::bgra_bytes(size)?;
    Ok(size)
}
fn prefix(bytes: &[u8]) -> io::Result<(u64, u64, u64, bool, FrameLimits, Option<u64>, u8)> {
    let mut reader = BeReader::new(bytes);
    let id = reader.u64()?;
    let capture_generation = reader.u64()?;
    let generation = reader.u64()?;
    let regions = match reader.u8()? {
        0 => false,
        1 => true,
        _ => return Err(invalid("invalid owned worker regions flag")),
    };
    let limits = FrameLimits {
        max_size: size(&mut reader)?,
        max_pixels: reader.u64()?,
        working_bytes: reader.u64()?,
    };
    limits.validate_capture()?;
    let revision = reader.u64()?;
    let tag = reader.u8()?;
    reader.finish()?;
    if !matches!(tag, RAW | ENCODED) {
        return Err(invalid("owned worker envelope has no picture"));
    }
    Ok((
        id,
        capture_generation,
        generation,
        regions,
        limits,
        (revision != 0).then_some(revision),
        tag,
    ))
}
pub(crate) fn read_owned(
    reader: &mut impl Read,
    length: usize,
    contract: Option<FrameContract>,
    lookup: impl FnOnce(u64) -> io::Result<CaptureGrant>,
) -> io::Result<(u64, CaptureGrant, AdmittedBytes)> {
    if length < 58 {
        return Err(invalid("truncated owned worker prefix"));
    }
    let mut head = [0; 58];
    reader.read_exact(&mut head)?;
    let (id, generation, _, _, limits, _, tag) = prefix(&head)?;
    let grant = lookup(id)?;
    if grant.generation().value() != generation {
        return Err(invalid("owned worker grant generation mismatch"));
    }
    let budget = if tag == RAW && contract.is_none() {
        super::frame_size_caps::PacketBudget::for_limits(limits)?
    } else {
        super::frame_size_caps::PacketBudget::for_contract(contract)?
    };
    let bound = if tag == RAW {
        budget.raw_bytes
    } else {
        budget.encoded_bytes
    };
    if length - 58 > bound {
        return Err(invalid("owned worker body exceeds its bound"));
    }
    let mut bytes = AdmittedBytes::new(&grant, CaptureOwner::Transport, length as u64)?;
    bytes.as_mut_slice()[..58].copy_from_slice(&head);
    reader.read_exact(&mut bytes.as_mut_slice()[58..])?;
    Ok((id, grant, bytes))
}
pub(crate) fn decode_owned(
    bytes: &AdmittedBytes,
    grant: &CaptureGrant,
    contract: Option<FrameContract>,
) -> io::Result<WorkerReply> {
    let (_, capture_generation, generation, regions, limits, family_layout_revision, tag) = prefix(
        bytes
            .as_slice()
            .get(..58)
            .ok_or_else(|| invalid("truncated owned worker prefix"))?,
    )?;
    if capture_generation != grant.generation().value() {
        return Err(invalid("decoded worker capture generation mismatch"));
    }
    let payload = &bytes.as_slice()[58..];
    let reply = if tag == RAW {
        let mut reader = BeReader::new(payload);
        let size = size(&mut reader)?;
        if let Some(contract) = contract {
            contract.limits.check(size)?;
        } else {
            limits.check(size)?;
        }
        let length = size.checked_bgra_len()?;
        if reader.remaining() != length {
            return Err(invalid("owned worker raw size contradicts payload"));
        }
        let data = AdmittedBytes::copy(
            grant,
            CaptureOwner::CompositionPixels,
            reader.bytes(length)?,
        )?;
        WorkerReply::Raw(CaptureImageBuilder::from_bytes(size, limits, grant, data)?.freeze())
    } else {
        WorkerReply::Frame(decode_records(payload, grant, contract)?)
    };
    Ok(WorkerReply::Reference {
        generation,
        regions,
        limits,
        family_layout_revision,
        reply: Box::new(reply),
    })
}
fn decode_records(
    payload: &[u8],
    grant: &CaptureGrant,
    contract: Option<FrameContract>,
) -> io::Result<StageFrame> {
    let budget = super::frame_size_caps::PacketBudget::for_contract(contract)?;
    let mut reader = BeReader::new(payload);
    let capture_ts_us = reader.u64()?;
    let keyframe = reader.u8()? != 0;
    let hardware = reader.u8()? != 0;
    let source_size = size(&mut reader)?;
    let name_len = usize::from(reader.u16()?);
    if name_len > 256 {
        return Err(invalid("owned worker encoder name exceeds bound"));
    }
    let name = std::str::from_utf8(reader.bytes(name_len)?)
        .map_err(|_| invalid("worker encoder name is not UTF-8"))?;
    let count = reader.u32()? as usize;
    if count == 0 || count > budget.records {
        return Err(invalid("owned worker record count exceeds bound"));
    }
    let body = reader.bytes(reader.remaining())?;
    let mut inspect = BeReader::new(body);
    let mut capacity = 0u64;
    for _ in 0..count {
        let length = inspect.u32()? as usize;
        if length > budget.record_bytes {
            return Err(invalid("owned worker record exceeds accepted limit"));
        }
        inspect.bytes(length)?;
        capacity = capacity
            .checked_add(length as u64)
            .ok_or_else(|| invalid("worker records overflow"))?;
    }
    inspect.finish()?;
    if capacity > budget.encoded_bytes as u64 {
        return Err(invalid("owned worker records exceed frame budget"));
    }
    let records = CapturedRecords::from_allocator(grant, capacity, count as u64, || {
        let mut records = Vec::new();
        records
            .try_reserve_exact(count)
            .map_err(|_| io::Error::other("worker record container allocation failed"))?;
        let mut input = BeReader::new(body);
        for _ in 0..count {
            let length = input.u32()? as usize;
            let mut record = Vec::new();
            record
                .try_reserve_exact(length)
                .map_err(|_| io::Error::other("worker record allocation failed"))?;
            record.extend_from_slice(input.bytes(length)?);
            records.push(record);
        }
        input.finish()?;
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

#[path = "capture_transport_wire.rs"]
mod sending;
#[cfg(test)]
pub(crate) use sending::send_owned;
pub(crate) use sending::{picture_grant, prepare_owned, send_prepared};

#[cfg(test)]
#[path = "batches_capture_integration_fix_task_tests.rs"]
mod integration_fix_tests;
