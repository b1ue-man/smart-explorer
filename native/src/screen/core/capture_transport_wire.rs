//! Serialize directly into one previously admitted wire buffer.
use super::*;
pub(crate) fn picture_grant(reply:&WorkerReply)->io::Result<Option<CaptureGrant>> {
    match reply {
        WorkerReply::Reference{reply,..}=>picture_grant(reply),
        WorkerReply::Raw(image)=>Ok(Some(image.grant())),
        WorkerReply::Frame(StageFrame::Encoded{records,..})=>Ok(Some(records.grant()?)),
        _=>Ok(None),
    }
}
pub(crate) fn send_owned(writer:&mut impl Write,reply:&WorkerReply,id:u64,grant:&CaptureGrant)->io::Result<()> {
    let WorkerReply::Reference{generation,regions,limits,family_layout_revision,reply}=reply else {
        return Err(invalid("owned worker picture lacks reference metadata"));
    };
    limits.validate_capture()?;
    let (tag,length)=match reply.as_ref() {
        WorkerReply::Raw(image)=>(RAW,image.image().data().len().checked_add(8)),
        WorkerReply::Frame(StageFrame::Encoded{records,encoder,..})=> {
            if encoder.name.len()>256 {return Err(invalid("worker encoder name exceeds bound"));}
            let bytes=records.iter().try_fold(0usize,|n,r|n.checked_add(r.len()).and_then(|n|n.checked_add(4)));
            (ENCODED,bytes.and_then(|n|n.checked_add(24+encoder.name.len())))
        }
        _=>return Err(invalid("owned worker picture has no pixel or record owner")),
    };
    let capacity=length.and_then(|n|n.checked_add(58)).ok_or_else(||invalid("owned worker payload overflow"))?;
    let mut out=AdmittedBytes::buffer(grant,CaptureOwner::Transport,capacity as u64)?;
    out.extend_from_slice(&id.to_be_bytes())?;
    out.extend_from_slice(&grant.generation().value().to_be_bytes())?;
    out.extend_from_slice(&generation.to_be_bytes())?;
    out.extend_from_slice(&[u8::from(*regions)])?;
    append_size(&mut out,limits.max_size)?;
    out.extend_from_slice(&limits.max_pixels.to_be_bytes())?;
    out.extend_from_slice(&limits.working_bytes.to_be_bytes())?;
    out.extend_from_slice(&family_layout_revision.unwrap_or(0).to_be_bytes())?;
    out.extend_from_slice(&[tag])?;
    match reply.as_ref() {
        WorkerReply::Raw(image)=>{append_size(&mut out,image.size())?;out.extend_from_slice(image.image().data())?;}
        WorkerReply::Frame(StageFrame::Encoded{records,keyframe,capture_ts_us,encoder,source_size})=> {
            out.extend_from_slice(&capture_ts_us.to_be_bytes())?;
            out.extend_from_slice(&[u8::from(*keyframe),u8::from(encoder.hardware)])?;
            append_size(&mut out,*source_size)?;
            out.extend_from_slice(&(encoder.name.len() as u16).to_be_bytes())?;
            out.extend_from_slice(encoder.name.as_bytes())?;
            out.extend_from_slice(&(u32::try_from(records.len()).map_err(|_|invalid("too many worker records"))?).to_be_bytes())?;
            for record in records {
                out.extend_from_slice(&(u32::try_from(record.len()).map_err(|_|invalid("worker record size overflow"))?).to_be_bytes())?;
                out.extend_from_slice(record)?;
            }
        }
        _=>return Err(invalid("worker picture changed during serialization")),
    }
    worker_wire::write_message(writer,OWNED_REFERENCE,out.as_slice())
}
fn append_size(out:&mut AdmittedBytes,size:FrameSize)->io::Result<()> {
    out.extend_from_slice(&size.width.to_be_bytes())?;out.extend_from_slice(&size.height.to_be_bytes())
}
