//! Wire layout of the keyed walk and the attribute listing and status
//! (tags 85..=90): `WalkKeyed` (`walk-keys-v1`) answered by `FolderRun`
//! marks, `HashPart`s and `FolderKey` marks, `ListRuns` (90) answered by
//! `FolderRun` marks and `HashPart`s, `ListDirMeta` and `StatMeta` (`meta-list-v1`) answered
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
/// `WalkKeyed` option byte: keys only.
const KEYS_ONLY: u8 = 1;
/// Folders one `ListRuns` may name.
const MAX_RUN_FOLDERS: usize = 1 << 20;

/// Encoded body length after the request id and tag, `None` for other frames.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::WalkKeyed { root, known, .. } => (|| {
            if known.len() > MAX_KNOWN_BYTES || known.len() % 16 != 0 {
                return Err(bad("known folder keys out of bounds"));
            }
            let mut length = super::string_len(root)?;
            add_len(&mut length, 10)?;
            add_len(&mut length, 4 + known.len())?;
            Ok(length)
        })(),
        Frame::ListRuns { root, folders, .. } => (|| {
            if folders.len() > MAX_RUN_FOLDERS {
                return Err(bad("too many folders for one run listing"));
            }
            let mut length = super::string_len(root)?;
            add_len(&mut length, 5)?;
            for folder in folders {
                add_len(&mut length, super::string_len(folder)?)?;
            }
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
            keys_only,
        } => {
            b.push(85);
            put_str(b, root);
            b.push(*algorithm);
            put_u64(b, *min_bytes);
            put_bytes(b, known);
            b.push(if *keys_only { KEYS_ONLY } else { 0 });
        }
        Frame::ListRuns {
            root,
            algorithm,
            folders,
        } => {
            b.push(90);
            put_str(b, root);
            b.push(*algorithm);
            b.extend_from_slice(&(folders.len() as u32).to_le_bytes());
            for folder in folders {
                put_str(b, folder);
            }
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
            let keys_only = match r.u8()? {
                0 => false,
                KEYS_ONLY => true,
                _ => return Err(bad("unknown keyed walk option")),
            };
            Frame::WalkKeyed {
                root,
                algorithm,
                min_bytes,
                known,
                keys_only,
            }
        }
        90 => {
            let root = r.string()?;
            let algorithm = r.u8()?;
            let count = r.u32()? as usize;
            if count > MAX_RUN_FOLDERS || count > r.remaining() / 4 {
                return Err(bad("too many folders for one run listing"));
            }
            let mut folders = Vec::with_capacity(count);
            for _ in 0..count {
                folders.push(r.string()?);
            }
            Frame::ListRuns {
                root,
                algorithm,
                folders,
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
