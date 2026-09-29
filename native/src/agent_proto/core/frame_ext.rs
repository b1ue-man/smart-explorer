//! Wire layout of the transfer-engine frames (tags 34..=44): flow-control
//! credit, batches and the stage/directory unit operations. Kept apart from
//! the original frame set so either file stays small; the byte layout follows
//! the same little-endian, length-prefixed rules.
use std::io;

use super::super::super::batch_limits::{BATCH_MAX_FILES, ITEM_PATH_MAX, ITEM_TEXT_MAX};
use super::super::super::types::{BatchEntry, BatchItem, Frame};
use super::super::{bad, Reader};
use super::{
    add_len, optional_string_len, put_bool, put_opt_str, put_str, put_u32, put_u64, string_len,
};

/// Smallest encoded batch entry: path length prefix, size and nonce.
const MIN_BATCH_ENTRY_BYTES: usize = 20;
/// Smallest encoded batch item: path length prefix, id flag and size.
const MIN_BATCH_ITEM_BYTES: usize = 13;

/// Encoded body length after the request id and tag, `None` for frames of
/// the original set.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::Credit { .. } => Ok(8),
        Frame::BatchPut { entries } => entries_len(entries),
        Frame::BatchGet { items } => items_len(items),
        Frame::ItemBegin { .. } => Ok(12),
        Frame::ItemEnd { error, .. } => within(error.as_deref(), ITEM_TEXT_MAX)
            .and_then(|()| optional_string_len(error))
            .and_then(|len| sum(4, len)),
        Frame::ItemPublished { path, .. } => within(Some(path.as_str()), ITEM_PATH_MAX)
            .and_then(|()| string_len(path))
            .and_then(|len| sum(4, len)),
        Frame::ItemFailed { message, .. } => within(Some(message.as_str()), ITEM_TEXT_MAX)
            .and_then(|()| string_len(message))
            .and_then(|len| sum(4, len)),
        Frame::CopyToStage { src, stage, .. } => string_len(src)
            .and_then(|src| sum(src, string_len(stage)?))
            .and_then(|len| sum(len, 8)),
        Frame::Copied(copied) => Ok(if copied.is_some() { 9 } else { 1 }),
        Frame::CreateDir { path, .. } => string_len(path).and_then(|len| sum(len, 1)),
        Frame::DiscardStage(path) => string_len(path),
        _ => return None,
    })
}

/// Append the tag and fields; false for frames of the original set.
pub(in super::super) fn encode(frame: &Frame, b: &mut Vec<u8>) -> bool {
    match frame {
        Frame::Credit { bytes } => {
            b.push(34);
            put_u64(b, *bytes);
        }
        Frame::BatchPut { entries } => {
            b.push(35);
            put_u32(b, entries.len() as u32);
            for entry in entries {
                put_str(b, &entry.path);
                put_u64(b, entry.size);
                put_u64(b, entry.nonce);
            }
        }
        Frame::BatchGet { items } => {
            b.push(36);
            put_u32(b, items.len() as u32);
            for item in items {
                put_str(b, &item.path);
                put_opt_str(b, &item.id);
                put_u64(b, item.size);
            }
        }
        Frame::ItemBegin { index, size } => {
            b.push(37);
            put_u32(b, *index);
            put_u64(b, *size);
        }
        Frame::ItemEnd { index, error } => {
            b.push(38);
            put_u32(b, *index);
            put_opt_str(b, error);
        }
        Frame::ItemPublished { index, path } => {
            b.push(39);
            put_u32(b, *index);
            put_str(b, path);
        }
        Frame::ItemFailed { index, message } => {
            b.push(40);
            put_u32(b, *index);
            put_str(b, message);
        }
        Frame::CopyToStage { src, stage, size } => {
            b.push(41);
            put_str(b, src);
            put_str(b, stage);
            put_u64(b, *size);
        }
        Frame::Copied(copied) => {
            b.push(42);
            put_bool(b, copied.is_some());
            if let Some(copied) = copied {
                put_u64(b, *copied);
            }
        }
        Frame::CreateDir { path, exclusive } => {
            b.push(43);
            put_str(b, path);
            put_bool(b, *exclusive);
        }
        Frame::DiscardStage(path) => {
            b.push(44);
            put_str(b, path);
        }
        _ => return false,
    }
    true
}

/// Decode a frame of this set; `None` for an unknown tag.
pub(in super::super) fn decode(tag: u8, r: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(Some(match tag {
        34 => Frame::Credit { bytes: r.u64()? },
        35 => {
            let count = batch_count(r)?;
            if count > r.remaining() / MIN_BATCH_ENTRY_BYTES {
                return Err(bad("batch entry count exceeds the remaining frame bytes"));
            }
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                entries.push(BatchEntry {
                    path: r.string()?,
                    size: r.u64()?,
                    nonce: r.u64()?,
                });
            }
            Frame::BatchPut { entries }
        }
        36 => {
            let count = batch_count(r)?;
            if count > r.remaining() / MIN_BATCH_ITEM_BYTES {
                return Err(bad("batch item count exceeds the remaining frame bytes"));
            }
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(BatchItem {
                    path: r.string()?,
                    id: r.opt_str()?,
                    size: r.u64()?,
                });
            }
            Frame::BatchGet { items }
        }
        37 => Frame::ItemBegin {
            index: r.u32()?,
            size: r.u64()?,
        },
        38 => Frame::ItemEnd {
            index: r.u32()?,
            error: if r.bool()? {
                Some(bounded_string(r, ITEM_TEXT_MAX)?)
            } else {
                None
            },
        },
        39 => Frame::ItemPublished {
            index: r.u32()?,
            path: bounded_string(r, ITEM_PATH_MAX)?,
        },
        40 => Frame::ItemFailed {
            index: r.u32()?,
            message: bounded_string(r, ITEM_TEXT_MAX)?,
        },
        41 => Frame::CopyToStage {
            src: r.string()?,
            stage: r.string()?,
            size: r.u64()?,
        },
        42 => Frame::Copied(if r.bool()? { Some(r.u64()?) } else { None }),
        43 => Frame::CreateDir {
            path: r.string()?,
            exclusive: r.bool()?,
        },
        44 => Frame::DiscardStage(r.string()?),
        _ => return Ok(None),
    }))
}

/// The element count of a batch header, refused above the protocol limit
/// before anything is allocated for it.
fn batch_count(r: &mut Reader) -> io::Result<usize> {
    let count = r.u32()? as usize;
    if count > BATCH_MAX_FILES {
        return Err(bad("batch exceeds its file limit"));
    }
    Ok(count)
}

/// A string field of at most `max` bytes, refused before it is copied.
fn bounded_string(r: &mut Reader, max: usize) -> io::Result<String> {
    let length = r.u32()? as usize;
    if length > max {
        return Err(bad("text field exceeds its protocol limit"));
    }
    String::from_utf8(r.take(length)?.to_vec()).map_err(|_| bad("invalid utf8"))
}

/// Refuse to encode a text the receiver would refuse to decode.
fn within(text: Option<&str>, max: usize) -> io::Result<()> {
    if text.is_some_and(|text| text.len() > max) {
        return Err(bad("text field exceeds its protocol limit"));
    }
    Ok(())
}

fn entries_len(entries: &[BatchEntry]) -> io::Result<usize> {
    if entries.len() > BATCH_MAX_FILES {
        return Err(bad("batch exceeds its file limit"));
    }
    let mut length = 4;
    for entry in entries {
        add_len(&mut length, string_len(&entry.path)?)?;
        add_len(&mut length, 16)?;
    }
    Ok(length)
}

fn items_len(items: &[BatchItem]) -> io::Result<usize> {
    if items.len() > BATCH_MAX_FILES {
        return Err(bad("batch exceeds its file limit"));
    }
    let mut length = 4;
    for item in items {
        add_len(&mut length, string_len(&item.path)?)?;
        add_len(&mut length, optional_string_len(&item.id)?)?;
        add_len(&mut length, 8)?;
    }
    Ok(length)
}

fn sum(mut left: usize, right: usize) -> io::Result<usize> {
    add_len(&mut left, right)?;
    Ok(left)
}
