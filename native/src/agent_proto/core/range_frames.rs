//! Wire layout of the frames of ranged uploads (`range-v1`, tags 63..=66):
//! `SegStageOpen`, `SegStageReady`, `WriteAt` and `SegStageFinish`. They
//! follow the byte layout of the other frames (little endian, length-prefixed
//! strings). A server that does not know a tag answers with an error and
//! never ignores the frame, so a client sends them only after the server
//! announced the label.
use std::io;

use super::super::super::types::Frame;
use super::super::Reader;
use super::{add_len, put_str, put_u64, string_len};

/// Encoded body length after the request id and tag, `None` for other frames.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::SegStageOpen { path, .. } => text_and(path, 8),
        Frame::SegStageReady { stage } | Frame::SegStageFinish { stage } => text_and(stage, 0),
        Frame::WriteAt { stage, .. } => text_and(stage, 16),
        _ => return None,
    })
}

fn text_and(text: &str, fixed: usize) -> io::Result<usize> {
    let mut length = fixed;
    add_len(&mut length, string_len(text)?)?;
    Ok(length)
}

/// Append the tag and fields; false for other frames.
pub(in super::super) fn encode(frame: &Frame, b: &mut Vec<u8>) -> bool {
    match frame {
        Frame::SegStageOpen { path, size } => {
            b.push(63);
            put_str(b, path);
            put_u64(b, *size);
        }
        Frame::SegStageReady { stage } => {
            b.push(64);
            put_str(b, stage);
        }
        Frame::WriteAt { stage, offset, len } => {
            b.push(65);
            put_str(b, stage);
            put_u64(b, *offset);
            put_u64(b, *len);
        }
        Frame::SegStageFinish { stage } => {
            b.push(66);
            put_str(b, stage);
        }
        _ => return false,
    }
    true
}

/// Decode a frame of this set; `None` for an unknown tag.
pub(in super::super) fn decode(tag: u8, r: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(Some(match tag {
        63 => Frame::SegStageOpen {
            path: r.string()?,
            size: r.u64()?,
        },
        64 => Frame::SegStageReady { stage: r.string()? },
        65 => Frame::WriteAt {
            stage: r.string()?,
            offset: r.u64()?,
            len: r.u64()?,
        },
        66 => Frame::SegStageFinish { stage: r.string()? },
        _ => return Ok(None),
    }))
}
