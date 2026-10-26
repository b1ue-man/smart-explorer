//! Bounded worker picture payloads and optional source-reference envelope.
use super::*;

pub(super) fn absolute_budget()->io::Result<super::super::frame_size_caps::PacketBudget> {
    let mut budget=super::super::frame_size_caps::PacketBudget::for_limits(FrameLimits::RESOURCE)?;
    let legacy=super::super::frame_size_caps::PacketBudget::for_limits(FrameLimits::LEGACY)?;
    budget.raw_bytes=budget.raw_bytes.max(legacy.raw_bytes);
    budget.encoded_bytes=budget.encoded_bytes.max(legacy.encoded_bytes);
    budget.records=budget.records.max(legacy.records);
    budget.record_bytes=budget.record_bytes.max(legacy.record_bytes);
    budget.pull_bytes=budget.pull_bytes.max(legacy.pull_bytes);
    Ok(budget)
}

pub(super) fn negotiated_payload_limit(tag:u8,limits:FrameLimits)->io::Result<usize> {
    let contract=super::super::frame_budget::FrameContract {
        version:super::super::frame_budget::FRAME_PROTOCOL,limits,hybrid_regions:false,
    };
    let budget=super::super::frame_size_caps::PacketBudget::for_contract(Some(contract))?;
    Ok(match tag {ENCODED=>budget.encoded_bytes,RAW=>budget.raw_bytes,
        _=>tag_limit(tag).ok_or_else(||invalid("unknown worker reply"))?})
}

pub(super) fn read_reference_payload(reader:&mut impl Read,len:usize,expected:Option<FrameLimits>)->io::Result<Vec<u8>> {
    if len<42 {return Err(invalid("truncated worker reference header"));}
    let mut prefix=[0u8;42];reader.read_exact(&mut prefix)?;
    let mut fields=BeReader::new(&prefix);
    fields.u64()?;let regions=fields.u8()?;
    if regions>1 {return Err(invalid("invalid worker reference regions"));}
    let source=FrameLimits {max_size:read_size(&mut fields)?,max_pixels:fields.u64()?,working_bytes:fields.u64()?};
    source.validate()?;fields.u64()?;let tag=fields.u8()?;
    let bound=if let Some(limits)=expected {negotiated_payload_limit(tag,limits)?} else {
        let budget=super::super::frame_size_caps::PacketBudget::for_limits(source)?;
        match tag {ENCODED=>budget.encoded_bytes,RAW=>budget.raw_bytes,
            _=>tag_limit(tag).ok_or_else(||invalid("unknown referenced worker reply"))?}
    };
    if tag==REFERENCE || len-42>bound {return Err(invalid("referenced worker reply exceeds its working memory"));}
    let mut payload=vec![0;len];payload[..42].copy_from_slice(&prefix);
    reader.read_exact(&mut payload[42..])?;Ok(payload)
}

pub(super) fn encode_frame(frame: &StageFrame) -> io::Result<(u8, Vec<u8>)> {
    let budget = packet_budget()?;
    match frame {
        StageFrame::Encoded {
            records,
            keyframe,
            capture_ts_us,
            encoder,
            source_size,
        } => {
            let name = encoder.name.as_bytes();
            if name.len() > MAX_NAME_BYTES || records.is_empty() || records.len() > budget.records {
                return Err(invalid("encoded picture does not fit a host process frame"));
            }
            let total=records.iter().try_fold(0usize,|total,record| {
                if record.len()>MAX_RECORD_BYTES {return Err(invalid("encoded record exceeds its limit"));}
                total.checked_add(4).and_then(|n|n.checked_add(record.len())).ok_or_else(||invalid("encoded frame size overflow"))
            })?;
            let capacity=total.checked_add(name.len()).and_then(|n|n.checked_add(24)).ok_or_else(||invalid("encoded frame size overflow"))?;
            if capacity>budget.encoded_bytes {return Err(invalid("encoded frame exceeds worker packet budget"));}
            let mut out = Vec::with_capacity(capacity);
            out.extend_from_slice(&capture_ts_us.to_be_bytes());
            out.push(u8::from(*keyframe));
            out.push(u8::from(encoder.hardware));
            push_size(&mut out, *source_size);
            out.extend_from_slice(&(name.len() as u16).to_be_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&(records.len() as u32).to_be_bytes());
            for record in records {
                if record.len() > MAX_RECORD_BYTES {
                    return Err(invalid("encoded record exceeds its limit"));
                }
                out.extend_from_slice(&(record.len() as u32).to_be_bytes());
                out.extend_from_slice(record);
            }
            Ok((ENCODED, out))
        }
        StageFrame::Unchanged { source_size } => {
            let mut out = Vec::with_capacity(8);
            push_size(&mut out, *source_size);
            Ok((UNCHANGED, out))
        }
        StageFrame::Paused(state) => Ok((PAUSED, to_json(&PausedWire { state: *state })?)),
    }
}

pub(super) fn decode_encoded(payload: &[u8]) -> io::Result<StageFrame> {
    let budget = packet_budget()?;
    if payload.len() > budget.encoded_bytes { return Err(invalid("encoded frame exceeds worker packet budget")); }
    let mut reader = BeReader::new(payload);
    let capture_ts_us = reader.u64()?;
    let keyframe = reader.u8()? != 0;
    let hardware = reader.u8()? != 0;
    let source_size = read_size(&mut reader)?;
    let name_len = usize::from(reader.u16()?);
    if name_len > MAX_NAME_BYTES {
        return Err(invalid("encoder name exceeds its limit"));
    }
    let name = String::from_utf8(reader.bytes(name_len)?.to_vec())
        .map_err(|_| invalid("encoder name is not UTF-8"))?;
    let count = reader.u32()? as usize;
    if count == 0 || count > budget.records {
        return Err(invalid("encoded picture has no or too many records"));
    }
    // Every record needs at least its length field.
    let mut records = Vec::with_capacity(count.min(reader.remaining() / 4));
    for _ in 0..count {
        let len = reader.u32()? as usize;
        if len > MAX_RECORD_BYTES {
            return Err(invalid("encoded record exceeds its limit"));
        }
        records.push(reader.bytes(len)?.to_vec());
    }
    reader.finish()?;
    Ok(StageFrame::Encoded {
        records,
        keyframe,
        capture_ts_us,
        encoder: StageEncoder { name, hardware },
        source_size,
    })
}

pub(super) fn encode_reference(generation:u64,regions:bool,limits:FrameLimits,family_layout_revision:Option<u64>,reply:&WorkerReply) -> io::Result<(u8,Vec<u8>)> {
    limits.validate()?;
    if matches!(reply,WorkerReply::Reference {..}) {return Err(invalid("nested worker reference envelope"));}
    let budget=super::super::frame_size_caps::PacketBudget::for_limits(limits)?;
    if let WorkerReply::Frame(StageFrame::Encoded {records,..})=reply {
        let total=records.iter().try_fold(0usize,|n,r|n.checked_add(4).and_then(|n|n.checked_add(r.len())));
        if records.len()>budget.records || total.is_none_or(|n|n.saturating_add(24+MAX_NAME_BYTES)>budget.encoded_bytes) {
            return Err(invalid("worker reference exceeds source packet budget"));
        }
    }
    if let WorkerReply::Raw(image)=reply {
        if image.data().len().saturating_add(8)>budget.raw_bytes {return Err(invalid("raw worker reference exceeds source pixel budget"));}
    }
    let (tag,payload)=reply.encode()?;
    let limit=tag_limit(tag).ok_or_else(||invalid("unknown referenced worker reply"))?;
    if payload.len()>limit {return Err(invalid("referenced worker reply exceeds tag limit"));}
    let mut out=Vec::with_capacity(payload.len().checked_add(42).ok_or_else(||invalid("worker reference size overflow"))?);
    out.extend_from_slice(&generation.to_be_bytes());out.push(u8::from(regions));
    push_size(&mut out,limits.max_size);
    out.extend_from_slice(&limits.max_pixels.to_be_bytes());
    out.extend_from_slice(&limits.working_bytes.to_be_bytes());
    if family_layout_revision == Some(0) {return Err(invalid("zero family layout revision"));}
    out.extend_from_slice(&family_layout_revision.unwrap_or(0).to_be_bytes());
    out.push(tag);out.extend_from_slice(&payload);Ok((REFERENCE,out))
}
pub(super) fn decode_reference(payload:&[u8]) -> io::Result<WorkerReply> {
    let mut reader=BeReader::new(payload);let generation=reader.u64()?;let regions=match reader.u8()? {0=>false,1=>true,_=>return Err(invalid("invalid worker source capability"))};
    let limits=FrameLimits {max_size:read_size(&mut reader)?,max_pixels:reader.u64()?,working_bytes:reader.u64()?};
    limits.validate()?;
    let revision=reader.u64()?;
    let family_layout_revision=(revision!=0).then_some(revision);
    let tag=reader.u8()?;
    if !matches!(tag,OPENED|REFUSED|ENCODED|UNCHANGED|PAUSED|FAILED|RAW|INPUT_DONE|CAPTURE_ACTIVE|FAMILY_REPLY) {
        return Err(invalid("invalid referenced worker reply"));
    }
    let bytes=reader.bytes(reader.remaining())?;
    if bytes.len()>tag_limit(tag).ok_or_else(||invalid("unknown referenced worker reply"))? {return Err(invalid("referenced worker reply exceeds tag limit"));}
    let budget=super::super::frame_size_caps::PacketBudget::for_limits(limits)?;
    if (tag==ENCODED && bytes.len()>budget.encoded_bytes) || (tag==RAW && bytes.len()>budget.raw_bytes) {
        return Err(invalid("referenced worker payload exceeds source working memory"));
    }
    Ok(WorkerReply::Reference {generation,regions,limits,family_layout_revision,reply:Box::new(WorkerReply::decode(tag,bytes)?)})
}
