//! Wire layout of the keyed walk and the attribute listing and status
//! (tags 85..=89): `WalkKeyed` (`walk-keys-v1`) answered by `FolderRun`
//! marks, `HashPart`s and `FolderKey` marks, `ListDirMeta` and `StatMeta` (`meta-list-v1`) answered
//! by `HashPart`s. Same little-endian, length-prefixed rules as the other
//! frames; a server without the label answers the unknown tag with an
//! error, so a client sends it only after the label.
use std::io;

use super::super::super::types::Frame;
use super::super::{bad, Reader};
use super::{add_len, put_bytes, put_str, put_u64};

/// Known key prefixes one request may carry.
const MAX_KNOWN_BYTES: usize = 16 * super::super::super::folder_key::MAX_KNOWN_KEYS;
const MARK_SAME: u8 = 1;

/// Encoded body length after the request id and tag, `None` for other frames.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::WalkKeyed { root, known, .. } => (|| {
            if known.len() > MAX_KNOWN_BYTES || known.len() % 16 != 0 {
                return Err(bad("known folder keys out of bounds"));
            }
            let mut length = super::string_len(root)?;
            add_len(&mut length, 9)?;
            add_len(&mut length, 4 + known.len())?;
            Ok(length)
        })(),
        Frame::FolderKey { rel, .. } => super::string_len(rel).and_then(|len| {
            let mut length = len;
            add_len(&mut length, 33)?;
            Ok(length)
        }),
        Frame::ListDirMeta(path) | Frame::StatMeta(path) | Frame::FolderRun(path) => {
            super::string_len(path)
        }
        _ => return None,
    })
}

/// Append the tag and fields; false for other frames.
pub(in super::super) fn encode(frame: &Frame, b: &mut Vec<u8>) -> bool {
    match frame {
        Frame::WalkKeyed {
            root,
            algorithm,
            min_bytes,
            known,
        } => {
            b.push(85);
            put_str(b, root);
            b.push(*algorithm);
            put_u64(b, *min_bytes);
            put_bytes(b, known);
        }
        Frame::FolderKey { rel, key, same } => {
            b.push(86);
            put_str(b, rel);
            b.extend_from_slice(key);
            b.push(if *same { MARK_SAME } else { 0 });
        }
        Frame::ListDirMeta(path) => {
            b.push(87);
            put_str(b, path);
        }
        Frame::StatMeta(path) => {
            b.push(88);
            put_str(b, path);
        }
        Frame::FolderRun(rel) => {
            b.push(89);
            put_str(b, rel);
        }
        _ => return false,
    }
    true
}

/// Decode a frame of this set; `None` for an unknown tag.
pub(in super::super) fn decode(tag: u8, r: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(Some(match tag {
        85 => {
            let root = r.string()?;
            let algorithm = r.u8()?;
            let min_bytes = r.u64()?;
            let known = r.bytes()?;
            if known.len() > MAX_KNOWN_BYTES || known.len() % 16 != 0 {
                return Err(bad("known folder keys out of bounds"));
            }
            Frame::WalkKeyed {
                root,
                algorithm,
                min_bytes,
                known,
            }
        }
        86 => {
            let rel = r.string()?;
            let mut key = [0u8; 32];
            key.copy_from_slice(r.take(32)?);
            let same = match r.u8()? {
                0 => false,
                MARK_SAME => true,
                _ => return Err(bad("unknown folder key mark")),
            };
            Frame::FolderKey { rel, key, same }
        }
        87 => Frame::ListDirMeta(r.string()?),
        88 => Frame::StatMeta(r.string()?),
        89 => Frame::FolderRun(r.string()?),
        _ => return Ok(None),
    }))
}

#[cfg(test)]
#[path = "walk_key_frames_tests.rs"]
mod tests;
