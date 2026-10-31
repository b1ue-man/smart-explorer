//! Wire layout of the hash walk in portions (`walk-meta-v1`, tags 79..=80):
//! `WalkHashed3` and its `HashPart`s. Same little-endian, length-prefixed
//! rules as the other frames; a server without the label answers the
//! unknown tag with an error, so a client sends it only after the label.
//!
//! `HashPart` entry: `rel`, a flag byte (`ENTRY_*`), size, time, the optional
//! digest and, with `ENTRY_META`, the creation time.
use std::io;

use super::super::super::types::{Frame, WireHashEntry, WireHashMeta};
use super::super::{bad, Reader};
use super::frame_ops::{code, counted, get_omission, omission_len, put_omission};
use super::{add_len, optional_string_len, put_i64, put_opt_str, put_str, put_u32, put_u64};

const ENTRY_DIR: u8 = 1;
const ENTRY_META: u8 = 2;
const ENTRY_HIDDEN: u8 = 4;
const ENTRY_SYSTEM: u8 = 8;
const ENTRY_LINK: u8 = 16;
const ENTRY_SPECIAL: u8 = 32;
/// Flags that exist only next to `ENTRY_META`.
const META_ONLY: u8 = ENTRY_HIDDEN | ENTRY_SYSTEM | ENTRY_LINK | ENTRY_SPECIAL;
/// `WalkHashed3` option byte: entries carry their listing attributes.
const WALK_META: u8 = 1;
/// Smallest encoded entry: path prefix, flags, size, time, digest flag.
const MIN_ENTRY_BYTES: usize = 22;
/// Smallest encoded omission: two length prefixes and the reason.
const MIN_OMISSION_BYTES: usize = 9;

/// Encoded body length after the request id and tag, `None` for other frames.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::WalkHashed3 { root, .. } => super::string_len(root).and_then(|len| sum(len, 10)),
        Frame::HashPart { entries, omitted } => part_len(entries, omitted),
        _ => return None,
    })
}

/// Append the tag and fields; false for other frames.
pub(in super::super) fn encode(frame: &Frame, b: &mut Vec<u8>) -> bool {
    match frame {
        Frame::WalkHashed3 {
            root,
            algorithm,
            min_bytes,
            meta,
        } => {
            b.push(79);
            put_str(b, root);
            b.push(*algorithm);
            put_u64(b, *min_bytes);
            b.push(if *meta { WALK_META } else { 0 });
        }
        Frame::HashPart { entries, omitted } => {
            b.push(80);
            put_u32(b, entries.len() as u32);
            for entry in entries {
                put_entry(b, entry);
            }
            put_u32(b, omitted.len() as u32);
            for omission in omitted {
                put_omission(b, omission);
            }
        }
        _ => return false,
    }
    true
}

/// Decode a frame of this set; `None` for an unknown tag.
pub(in super::super) fn decode(tag: u8, r: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(Some(match tag {
        79 => Frame::WalkHashed3 {
            root: r.string()?,
            algorithm: code(r, 2)?,
            min_bytes: r.u64()?,
            meta: match r.u8()? {
                0 => false,
                WALK_META => true,
                _ => return Err(bad("unknown hash walk options")),
            },
        },
        80 => {
            let count = counted(r, MIN_ENTRY_BYTES)?;
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                entries.push(get_entry(r)?);
            }
            let count = counted(r, MIN_OMISSION_BYTES)?;
            let mut omitted = Vec::with_capacity(count);
            for _ in 0..count {
                omitted.push(get_omission(r)?);
            }
            Frame::HashPart { entries, omitted }
        }
        _ => return Ok(None),
    }))
}

fn flags(entry: &WireHashEntry) -> u8 {
    let mut flags = if entry.is_dir { ENTRY_DIR } else { 0 };
    if let Some(meta) = entry.meta {
        flags |= ENTRY_META;
        for (set, flag) in [
            (meta.hidden, ENTRY_HIDDEN),
            (meta.system, ENTRY_SYSTEM),
            (meta.is_symlink, ENTRY_LINK),
            (meta.special, ENTRY_SPECIAL),
        ] {
            if set {
                flags |= flag;
            }
        }
    }
    flags
}

fn put_entry(b: &mut Vec<u8>, entry: &WireHashEntry) {
    put_str(b, &entry.rel);
    b.push(flags(entry));
    put_u64(b, entry.size);
    put_i64(b, entry.mtime_ms);
    put_opt_str(b, &entry.digest);
    if let Some(meta) = entry.meta {
        put_i64(b, meta.btime_ms);
    }
}

fn get_entry(r: &mut Reader) -> io::Result<WireHashEntry> {
    let rel = r.string()?;
    let flags = r.u8()?;
    if flags & !(ENTRY_DIR | ENTRY_META | META_ONLY) != 0
        || (flags & ENTRY_META == 0 && flags & META_ONLY != 0)
    {
        return Err(bad("unknown hash walk entry flags"));
    }
    let size = r.u64()?;
    let mtime_ms = r.i64()?;
    let digest = r.opt_str()?;
    let meta = if flags & ENTRY_META != 0 {
        Some(WireHashMeta {
            hidden: flags & ENTRY_HIDDEN != 0,
            system: flags & ENTRY_SYSTEM != 0,
            btime_ms: r.i64()?,
            is_symlink: flags & ENTRY_LINK != 0,
            special: flags & ENTRY_SPECIAL != 0,
        })
    } else {
        None
    };
    Ok(WireHashEntry {
        rel,
        is_dir: flags & ENTRY_DIR != 0,
        size,
        mtime_ms,
        digest,
        meta,
    })
}

fn entry_len(entry: &WireHashEntry) -> io::Result<usize> {
    let mut length = super::string_len(&entry.rel)?;
    add_len(&mut length, 17)?;
    add_len(&mut length, optional_string_len(&entry.digest)?)?;
    if entry.meta.is_some() {
        add_len(&mut length, 8)?;
    }
    Ok(length)
}

fn part_len(
    entries: &[WireHashEntry],
    omitted: &[super::super::super::types::WireOmission],
) -> io::Result<usize> {
    u32::try_from(entries.len()).map_err(|_| bad("frame too large"))?;
    u32::try_from(omitted.len()).map_err(|_| bad("frame too large"))?;
    let mut length = 8;
    for entry in entries {
        add_len(&mut length, entry_len(entry)?)?;
    }
    for omission in omitted {
        add_len(&mut length, omission_len(omission)?)?;
    }
    Ok(length)
}

fn sum(mut left: usize, right: usize) -> io::Result<usize> {
    add_len(&mut left, right)?;
    Ok(left)
}

#[cfg(test)]
#[path = "walk_meta_frames_tests.rs"]
mod tests;
