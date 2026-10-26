//! Hybrid v2 records. Every operation carries the same reference and frame stamp.
use std::io;
use lz4_flex::block::{compress, decompress_into};
use super::frame_regions::MoveRect;
use super::pixel_convert::{BgraImage, FrameSize, Rect};
use super::wire::{invalid, BeReader};
use super::wire_binary::MAX_RECORD_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameStamp { pub generation: u64, pub frame_id: u64, pub base_frame_id: u64 }
#[derive(Debug)]
pub(crate) enum Operation {
    Begin,
    Video { keyframe: bool, rects: Vec<Rect>, data: Vec<u8> },
    Moves(Vec<MoveRect>),
    Dirty { rect: Rect, data: Vec<u8> },
    Commit,
}
fn rect_out(out: &mut Vec<u8>, rect: Rect) {
    for n in [rect.x, rect.y, rect.width, rect.height] { out.extend_from_slice(&n.to_be_bytes()); }
}
fn rect_in(reader: &mut BeReader<'_>, size: FrameSize) -> io::Result<Rect> {
    let rect = Rect { x: reader.u32()?, y: reader.u32()?, width: reader.u32()?, height: reader.u32()? };
    if !rect.fits(size) { return Err(invalid("hybrid rectangle exceeds negotiated frame")); }
    Ok(rect)
}
pub(crate) fn encode(stamp: FrameStamp, op: Operation) -> io::Result<Vec<u8>> {
    let payload=match &op {
        Operation::Begin|Operation::Commit=>Some(1),
        Operation::Video {rects,data,..}=>rects.len().checked_mul(16).and_then(|n|n.checked_add(data.len())).and_then(|n|n.checked_add(6)),
        Operation::Moves(moves)=>moves.len().checked_mul(24).and_then(|n|n.checked_add(5)),
        Operation::Dirty {data,..}=>data.len().checked_add(17),
    }.and_then(|n|n.checked_add(24)).filter(|n|*n<=MAX_RECORD_BYTES)
        .ok_or_else(||invalid("hybrid record exceeds 16 MiB"))?;
    let mut out = Vec::with_capacity(payload);
    out.extend_from_slice(&stamp.generation.to_be_bytes());
    out.extend_from_slice(&stamp.frame_id.to_be_bytes());
    out.extend_from_slice(&stamp.base_frame_id.to_be_bytes());
    match op {
        Operation::Begin => out.push(0),
        Operation::Video { keyframe, rects, data } => {
            out.push(1); out.push(u8::from(keyframe));
            out.extend_from_slice(&u32::try_from(rects.len()).map_err(|_| invalid("too many video rectangles"))?.to_be_bytes());
            for rect in rects { rect_out(&mut out, rect); }
            out.extend_from_slice(&data);
        }
        Operation::Moves(moves) => {
            out.push(2); out.extend_from_slice(&u32::try_from(moves.len()).map_err(|_| invalid("too many moves"))?.to_be_bytes());
            for moved in moves { out.extend_from_slice(&moved.source_x.to_be_bytes());
                out.extend_from_slice(&moved.source_y.to_be_bytes()); rect_out(&mut out, moved.destination); }
        }
        Operation::Dirty { rect, data } => { out.push(3); rect_out(&mut out, rect); out.extend_from_slice(&data); }
        Operation::Commit => out.push(4),
    }
    if out.len() > MAX_RECORD_BYTES { return Err(invalid("hybrid record exceeds 16 MiB")); }
    Ok(out)
}
pub(crate) fn decode(bytes: &[u8], size: FrameSize) -> io::Result<(FrameStamp, Operation)> {
    if bytes.len() > MAX_RECORD_BYTES { return Err(invalid("hybrid record exceeds 16 MiB")); }
    let mut reader = BeReader::new(bytes);
    let stamp = FrameStamp { generation: reader.u64()?, frame_id: reader.u64()?, base_frame_id: reader.u64()? };
    if stamp.generation == 0 || stamp.frame_id == 0 { return Err(invalid("zero hybrid reference or frame id")); }
    let max_rects = size.width.div_ceil(64) as usize * size.height.div_ceil(64) as usize;
    let op = match reader.u8()? {
        0 => Operation::Begin,
        1 => {
            let keyframe = match reader.u8()? { 0 => false, 1 => true, _ => return Err(invalid("invalid hybrid keyframe flag")) };
            let count = reader.u32()? as usize;
            if count == 0 || count > max_rects || count > reader.remaining() / 16 { return Err(invalid("invalid video rectangle count")); }
            let mut rects = Vec::with_capacity(count);
            for _ in 0..count { rects.push(rect_in(&mut reader, size)?); }
            Operation::Video { keyframe, rects, data: reader.bytes(reader.remaining())?.to_vec() }
        }
        2 => {
            let count = reader.u32()? as usize;
            if count > max_rects.saturating_mul(2) || count > reader.remaining() / 24 { return Err(invalid("invalid hybrid move count")); }
            let mut moves = Vec::with_capacity(count);
            for _ in 0..count { let source_x = reader.u32()?; let source_y = reader.u32()?;
                let destination = rect_in(&mut reader, size)?; let moved = MoveRect { source_x, source_y, destination };
                moved.validate(size)?; moves.push(moved); }
            Operation::Moves(moves)
        }
        3 => {
            let rect = rect_in(&mut reader, size)?;
            if rect.width > 64 || rect.height > 64 || reader.remaining() > 64 * 64 * 4 + 128 {
                return Err(invalid("hybrid lossless rectangle exceeds tile budget"));
            }
            Operation::Dirty { rect, data: reader.bytes(reader.remaining())?.to_vec() }
        }
        4 => Operation::Commit,
        _ => return Err(invalid("unknown hybrid operation")),
    };
    reader.finish()?;
    Ok((stamp, op))
}
pub(crate) fn lossless(image: &BgraImage, rect: Rect) -> io::Result<Operation> {
    if !rect.fits(image.size()) || rect.width > 64 || rect.height > 64 { return Err(invalid("invalid lossless region")); }
    let mut data = Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
    for y in rect.y..rect.y+rect.height {
        data.extend_from_slice(&image.view().row(y)[rect.x as usize*4..(rect.x+rect.width) as usize*4]);
    }
    for pixel in data.chunks_exact_mut(4) { pixel[3] = 255; }
    Ok(Operation::Dirty { rect, data: compress(&data) })
}
pub(crate) fn apply_dirty(target: &mut BgraImage, rect: Rect, data: &[u8]) -> io::Result<()> {
    if !rect.fits(target.size()) || rect.width > 64 || rect.height > 64 { return Err(invalid("invalid lossless rectangle")); }
    let len = rect.width as usize * rect.height as usize * 4;
    let mut raw = vec![0; len];
    if decompress_into(data, &mut raw).map_err(|_| invalid("corrupt hybrid LZ4 region"))? != len {
        return Err(invalid("hybrid region has wrong decoded length"));
    }
    let row = rect.width as usize * 4;
    for (y, source) in raw.chunks_exact(row).enumerate() {
        target.row_mut(rect.y+y as u32)[rect.x as usize*4..rect.x as usize*4+row].copy_from_slice(source);
    }
    Ok(())
}
