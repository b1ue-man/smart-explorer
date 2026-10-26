//! Bounded worker picture payloads and optional source-reference envelope.
use super::*;

pub(super) fn absolute_budget() -> io::Result<super::super::frame_size_caps::PacketBudget> {
    let mut budget =
        super::super::frame_size_caps::PacketBudget::for_limits(FrameLimits::RESOURCE)?;
    let legacy = super::super::frame_size_caps::PacketBudget::for_limits(FrameLimits::LEGACY)?;
    budget.raw_bytes = budget.raw_bytes.max(legacy.raw_bytes);
    budget.encoded_bytes = budget.encoded_bytes.max(legacy.encoded_bytes);
    budget.records = budget.records.max(legacy.records);
    budget.record_bytes = budget.record_bytes.max(legacy.record_bytes);
    budget.pull_bytes = budget.pull_bytes.max(legacy.pull_bytes);
    Ok(budget)
}

pub(super) fn negotiated_payload_limit(tag: u8, limits: FrameLimits) -> io::Result<usize> {
    let contract = super::super::frame_budget::FrameContract {
        version: super::super::frame_budget::FRAME_PROTOCOL,
        limits,
        hybrid_regions: false,
    };
    let budget = super::super::frame_size_caps::PacketBudget::for_contract(Some(contract))?;
    Ok(match tag {
        ENCODED => budget.encoded_bytes,
        RAW => budget.raw_bytes,
        _ => tag_limit(tag).ok_or_else(|| invalid("unknown worker reply"))?,
    })
}

pub(super) fn read_reference_payload(
    reader: &mut impl Read,
    len: usize,
    expected: Option<FrameLimits>,
) -> io::Result<Vec<u8>> {
    if len < 42 {
        return Err(invalid("truncated worker reference header"));
    }
    let mut prefix = [0u8; 42];
    reader.read_exact(&mut prefix)?;
    let mut fields = BeReader::new(&prefix);
    fields.u64()?;
    let regions = fields.u8()?;
    if regions > 1 {
        return Err(invalid("invalid worker reference regions"));
    }
    let source = FrameLimits {
        max_size: read_size(&mut fields)?,
        max_pixels: fields.u64()?,
        working_bytes: fields.u64()?,
    };
    source.validate_capture()?;
    fields.u64()?;
    let tag = fields.u8()?;
    let bound = if let Some(limits) = expected {
        negotiated_payload_limit(tag, limits)?
    } else {
        let budget = super::super::frame_size_caps::PacketBudget::for_limits(source)?;
        match tag {
            ENCODED => budget.encoded_bytes,
            RAW => budget.raw_bytes,
            _ => tag_limit(tag).ok_or_else(|| invalid("unknown referenced worker reply"))?,
        }
    };
    if matches!(
        tag,
        REFERENCE | RAW | ENCODED | super::super::worker_capture_wire::OWNED_REFERENCE
    ) || len - 42 > bound
    {
        return Err(invalid(
            "referenced worker reply exceeds its working memory",
        ));
    }
    let mut payload = vec![0; len];
    payload[..42].copy_from_slice(&prefix);
    reader.read_exact(&mut payload[42..])?;
    Ok(payload)
}

pub(super) fn encode_frame(frame: &StageFrame) -> io::Result<(u8, Vec<u8>)> {
    let budget = packet_budget()?;
    match frame {
        StageFrame::Encoded { .. } => {
            Err(invalid("worker records require owned capture transport"))
        }
        StageFrame::Unchanged { source_size } => {
            let mut out = Vec::with_capacity(8);
            push_size(&mut out, *source_size);
            Ok((UNCHANGED, out))
        }
        StageFrame::Paused(state) => Ok((PAUSED, to_json(&PausedWire { state: *state })?)),
    }
}

pub(super) fn decode_encoded(_payload: &[u8]) -> io::Result<StageFrame> {
    Err(invalid("worker records require a prior capture ticket"))
}

pub(super) fn encode_reference(
    generation: u64,
    regions: bool,
    limits: FrameLimits,
    family_layout_revision: Option<u64>,
    reply: &WorkerReply,
) -> io::Result<(u8, Vec<u8>)> {
    limits.validate_capture()?;
    if matches!(reply, WorkerReply::Reference { .. }) {
        return Err(invalid("nested worker reference envelope"));
    }
    let budget = super::super::frame_size_caps::PacketBudget::for_limits(limits)?;
    if let WorkerReply::Frame(StageFrame::Encoded { records, .. }) = reply {
        let total = records.iter().try_fold(0usize, |n, r| {
            n.checked_add(4).and_then(|n| n.checked_add(r.len()))
        });
        if records.len() > budget.records
            || total.is_none_or(|n| n.saturating_add(24 + MAX_NAME_BYTES) > budget.encoded_bytes)
        {
            return Err(invalid("worker reference exceeds source packet budget"));
        }
    }
    if let WorkerReply::Raw(image) = reply {
        if image.image().data().len().saturating_add(8) > budget.raw_bytes {
            return Err(invalid("raw worker reference exceeds source pixel budget"));
        }
    }
    let (tag, payload) = reply.encode()?;
    let limit = tag_limit(tag).ok_or_else(|| invalid("unknown referenced worker reply"))?;
    if payload.len() > limit {
        return Err(invalid("referenced worker reply exceeds tag limit"));
    }
    let mut out = Vec::with_capacity(
        payload
            .len()
            .checked_add(42)
            .ok_or_else(|| invalid("worker reference size overflow"))?,
    );
    out.extend_from_slice(&generation.to_be_bytes());
    out.push(u8::from(regions));
    push_size(&mut out, limits.max_size);
    out.extend_from_slice(&limits.max_pixels.to_be_bytes());
    out.extend_from_slice(&limits.working_bytes.to_be_bytes());
    if family_layout_revision == Some(0) {
        return Err(invalid("zero family layout revision"));
    }
    out.extend_from_slice(&family_layout_revision.unwrap_or(0).to_be_bytes());
    out.push(tag);
    out.extend_from_slice(&payload);
    Ok((REFERENCE, out))
}
pub(super) fn decode_reference(payload: &[u8]) -> io::Result<WorkerReply> {
    let mut reader = BeReader::new(payload);
    let generation = reader.u64()?;
    let regions = match reader.u8()? {
        0 => false,
        1 => true,
        _ => return Err(invalid("invalid worker source capability")),
    };
    let limits = FrameLimits {
        max_size: read_size(&mut reader)?,
        max_pixels: reader.u64()?,
        working_bytes: reader.u64()?,
    };
    limits.validate_capture()?;
    let revision = reader.u64()?;
    let family_layout_revision = (revision != 0).then_some(revision);
    let tag = reader.u8()?;
    if !matches!(
        tag,
        OPENED
            | REFUSED
            | ENCODED
            | UNCHANGED
            | PAUSED
            | FAILED
            | RAW
            | INPUT_DONE
            | CAPTURE_ACTIVE
            | FAMILY_REPLY
    ) {
        return Err(invalid("invalid referenced worker reply"));
    }
    let bytes = reader.bytes(reader.remaining())?;
    if bytes.len() > tag_limit(tag).ok_or_else(|| invalid("unknown referenced worker reply"))? {
        return Err(invalid("referenced worker reply exceeds tag limit"));
    }
    let budget = super::super::frame_size_caps::PacketBudget::for_limits(limits)?;
    if (tag == ENCODED && bytes.len() > budget.encoded_bytes)
        || (tag == RAW && bytes.len() > budget.raw_bytes)
    {
        return Err(invalid(
            "referenced worker payload exceeds source working memory",
        ));
    }
    Ok(WorkerReply::Reference {
        generation,
        regions,
        limits,
        family_layout_revision,
        reply: Box::new(WorkerReply::decode(tag, bytes)?),
    })
}
