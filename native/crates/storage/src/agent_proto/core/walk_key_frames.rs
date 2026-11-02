//! Wire layout of the keyed walk and the attribute listing and status
//! (tags 85..=90): `WalkKeyed` (`walk-keys-v1`) answered by `FolderRun`
//! marks, `HashPart`s and `FolderKey` marks, `ListRuns` (90) answered by
//! `FolderRun` marks and `HashPart`s, `ListDirMeta` and `StatMeta` (`meta-list-v1`) answered
//! by `HashPart`s. Same little-endian, length-prefixed rules as the other
//! frames; a server without the label answers the unknown tag with an
//! error, so a client sends it only after the label. `walk-filter-v1` adds
//! the scope bits of `walk_scope_frames.rs` to the `WalkKeyed` option byte
//! and, to `ListRuns`, one option byte after the folders (only when the
//! scope is not the default), each followed by the filter when it is set.
use std::io;

use super::super::super::types::Frame;
use super::super::{bad, Reader};
use super::{add_len, put_bytes, put_str, put_u64};
use crate::agent_proto::KeyTally;

/// Known key prefixes one request may carry.
const MAX_KNOWN_BYTES: usize = 16 * super::super::super::folder_key::MAX_KNOWN_KEYS;
const MARK_SAME: u8 = 1;
/// `FolderKey` mark bit: the subtree's tally follows the key.
const MARK_TALLY: u8 = 2;
/// `WalkKeyed` option byte: keys only.
const KEYS_ONLY: u8 = 1;
/// Folders one `ListRuns` may name.
const MAX_RUN_FOLDERS: usize = 1 << 20;

/// Length of `value` as a LEB128 varint.
fn varint_len(value: u64) -> usize {
    (64 - value.max(1).leading_zeros() as usize).div_ceil(7)
}

fn tally_len(tally: &KeyTally) -> usize {
    [
        tally.files,
        tally.bytes,
        tally.omitted,
        tally.untimed,
        u64::from(tally.height),
    ]
    .into_iter()
    .map(varint_len)
    .sum()
}

fn put_varint(b: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        b.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    b.push(value as u8);
}

fn take_varint(r: &mut Reader) -> io::Result<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = r.u8()?;
        let bits = u64::from(byte & 0x7f);
        if shift == 63 && bits > 1 {
            return Err(bad("folder tally number out of range"));
        }
        value |= bits << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(bad("folder tally number out of range"))
}

fn put_tally(b: &mut Vec<u8>, tally: &KeyTally) {
    for value in [
        tally.files,
        tally.bytes,
        tally.omitted,
        tally.untimed,
        u64::from(tally.height),
    ] {
        put_varint(b, value);
    }
}

fn take_tally(r: &mut Reader) -> io::Result<KeyTally> {
    Ok(KeyTally {
        files: take_varint(r)?,
        bytes: take_varint(r)?,
        omitted: take_varint(r)?,
        untimed: take_varint(r)?,
        height: u32::try_from(take_varint(r)?).map_err(|_| bad("folder depth out of range"))?,
    })
}

/// Encoded body length after the request id and tag, `None` for other frames.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::WalkKeyed {
            root, known, scope, ..
        } => (|| {
            if known.len() > MAX_KNOWN_BYTES || known.len() % 16 != 0 {
                return Err(bad("known folder keys out of bounds"));
            }
            let mut length = super::string_len(root)?;
            add_len(&mut length, 10)?;
            add_len(&mut length, 4 + known.len())?;
            add_len(&mut length, super::walk_scope_frames::len(scope)?)?;
            Ok(length)
        })(),
        Frame::ListRuns {
            root,
            folders,
            scope,
            ..
        } => (|| {
            if folders.len() > MAX_RUN_FOLDERS {
                return Err(bad("too many folders for one run listing"));
            }
            let mut length = super::string_len(root)?;
            add_len(&mut length, 5)?;
            for folder in folders {
                add_len(&mut length, super::string_len(folder)?)?;
            }
            if !scope.is_default() {
                add_len(&mut length, 1 + super::walk_scope_frames::len(scope)?)?;
            }
            Ok(length)
        })(),
        Frame::FolderKey { rel, tally, .. } => super::string_len(rel).and_then(|len| {
            let mut length = len;
            add_len(&mut length, 33)?;
            if let Some(tally) = tally {
                add_len(&mut length, tally_len(tally))?;
            }
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
            scope,
        } => {
            b.push(85);
            put_str(b, root);
            b.push(*algorithm);
            put_u64(b, *min_bytes);
            put_bytes(b, known);
            let options = if *keys_only { KEYS_ONLY } else { 0 };
            b.push(options | super::walk_scope_frames::bits(scope));
            super::walk_scope_frames::put(b, scope);
        }
        Frame::ListRuns {
            root,
            algorithm,
            folders,
            scope,
        } => {
            b.push(90);
            put_str(b, root);
            b.push(*algorithm);
            b.extend_from_slice(&(folders.len() as u32).to_le_bytes());
            for folder in folders {
                put_str(b, folder);
            }
            // Without a scope the frame stays the one of `walk-keys-v1`.
            if !scope.is_default() {
                b.push(super::walk_scope_frames::bits(scope));
                super::walk_scope_frames::put(b, scope);
            }
        }
        Frame::FolderKey {
            rel,
            key,
            same,
            tally,
        } => {
            b.push(86);
            put_str(b, rel);
            b.extend_from_slice(key);
            let mut mark = if *same { MARK_SAME } else { 0 };
            if tally.is_some() {
                mark |= MARK_TALLY;
            }
            b.push(mark);
            if let Some(tally) = tally {
                put_tally(b, tally);
            }
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
            let options = r.u8()?;
            if options & !(KEYS_ONLY | super::walk_scope_frames::SCOPE_BITS) != 0 {
                return Err(bad("unknown keyed walk option"));
            }
            let scope = super::walk_scope_frames::take(options, r)?;
            Frame::WalkKeyed {
                root,
                algorithm,
                min_bytes,
                known,
                keys_only: options & KEYS_ONLY != 0,
                scope,
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
            // `walk-filter-v1`: an option byte with the scope bits follows.
            let scope = if r.remaining() > 0 {
                let options = r.u8()?;
                if options & !super::walk_scope_frames::SCOPE_BITS != 0 {
                    return Err(bad("unknown run listing option"));
                }
                super::walk_scope_frames::take(options, r)?
            } else {
                Default::default()
            };
            Frame::ListRuns {
                root,
                algorithm,
                folders,
                scope,
            }
        }
        86 => {
            let rel = r.string()?;
            let mut key = [0u8; 32];
            key.copy_from_slice(r.take(32)?);
            let mark = r.u8()?;
            if mark & !(MARK_SAME | MARK_TALLY) != 0 {
                return Err(bad("unknown folder key mark"));
            }
            let tally = if mark & MARK_TALLY != 0 {
                Some(take_tally(r)?)
            } else {
                None
            };
            Frame::FolderKey {
                rel,
                key,
                same: mark & MARK_SAME != 0,
                tally,
            }
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
