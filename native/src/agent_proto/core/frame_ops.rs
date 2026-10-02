//! Wire layout of the extension frames (tags 45..=61, `ext-v1`): tolerant
//! listings, hash walks with omissions, the host's duplicate search,
//! queries, recycling, stage finishing, target limits and change
//! subscriptions. Same little-endian, length-prefixed rules as the original
//! set; every count is checked against the bytes that remain before
//! anything is allocated for it, every code against its known range.
use std::io;

use super::super::super::types::{
    Frame, WireChange, WireDuplicateGroup, WireDuplicateItem, WireDuplicateSummary, WireMeta,
    WireOmission, WireReclaimProgress, WireTargetLimits,
};
use super::super::{bad, get_meta, Reader};
use super::{
    add_len, metadata_len, optional_string_len, put_bool, put_i64, put_meta, put_opt_str, put_str,
    put_u32, put_u64, string_len,
};

/// Smallest encoded omission: two length prefixes and the reason.
const MIN_OMISSION_BYTES: usize = 9;
/// Smallest encoded duplicate item: two length prefixes, size, time, id flag.
const MIN_ITEM_BYTES: usize = 25;
/// Smallest encoded string.
const MIN_STRING_BYTES: usize = 4;
/// Smallest encoded protected area: name prefix and entry count.
const MIN_AREA_BYTES: usize = 12;

/// Encoded body length after the request id and tag, `None` for frames of
/// other sets.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::ListTolerant(path) | Frame::TargetLimits(path) => string_len(path),
        Frame::DirPart { entries, omitted } => dir_part_len(entries, omitted),
        Frame::WalkHashed2 { root, .. } => string_len(root).and_then(|len| sum(len, 9)),
        Frame::HashOmitted(omission) => omission_len(omission),
        Frame::FindDuplicates { root, .. } => string_len(root).and_then(|len| sum(len, 8)),
        Frame::DupProgress(progress) => string_len(&progress.current).and_then(|len| sum(len, 73)),
        Frame::DupGroup(group) => group_len(group),
        Frame::DupSummary(summary) => summary_len(summary),
        Frame::Query { path, .. } => string_len(path).and_then(|len| sum(len, 1)),
        Frame::Answer(_) => Ok(8),
        Frame::Recycle { path, sha256, .. } => string_len(path)
            .and_then(|len| sum(len, 8))
            .and_then(|len| sum(len, optional_string_len(sha256)?)),
        Frame::FinishStage {
            stage,
            mtime_ms,
            mode,
            ..
        } => string_len(stage).and_then(|len| {
            sum(
                len,
                3 + 8 * usize::from(mtime_ms.is_some()) + 4 * usize::from(mode.is_some()),
            )
        }),
        Frame::StageDone { .. } => Ok(2),
        Frame::Limits(limits) => Ok(12 + 8 * usize::from(limits.max_file_size.is_some())),
        Frame::Watch { root, .. } => string_len(root).and_then(|len| sum(len, 8)),
        Frame::Change(change) => change_len(change),
        _ => return None,
    })
}

/// Append the tag and fields; false for frames of other sets.
pub(in super::super) fn encode(frame: &Frame, b: &mut Vec<u8>) -> bool {
    match frame {
        Frame::ListTolerant(path) => {
            b.push(45);
            put_str(b, path);
        }
        Frame::DirPart { entries, omitted } => {
            b.push(46);
            put_u32(b, entries.len() as u32);
            for entry in entries {
                put_meta(b, entry);
            }
            put_u32(b, omitted.len() as u32);
            for omission in omitted {
                put_omission(b, omission);
            }
        }
        Frame::WalkHashed2 {
            root,
            algorithm,
            min_bytes,
        } => {
            b.push(47);
            put_str(b, root);
            b.push(*algorithm);
            put_u64(b, *min_bytes);
        }
        Frame::HashOmitted(omission) => {
            b.push(48);
            put_omission(b, omission);
        }
        Frame::FindDuplicates { root, min_bytes } => {
            b.push(49);
            put_str(b, root);
            put_u64(b, *min_bytes);
        }
        Frame::DupProgress(progress) => {
            b.push(50);
            put_progress(b, progress);
        }
        Frame::DupGroup(group) => {
            b.push(51);
            put_group(b, group);
        }
        Frame::DupSummary(summary) => {
            b.push(52);
            put_summary(b, summary);
        }
        Frame::Query { kind, path } => {
            b.push(53);
            b.push(*kind);
            put_str(b, path);
        }
        Frame::Answer(value) => {
            b.push(54);
            put_u64(b, *value);
        }
        Frame::Recycle { path, size, sha256 } => {
            b.push(55);
            put_str(b, path);
            put_u64(b, *size);
            put_opt_str(b, sha256);
        }
        Frame::FinishStage {
            stage,
            mtime_ms,
            mode,
            durability,
        } => {
            b.push(56);
            put_str(b, stage);
            put_bool(b, mtime_ms.is_some());
            if let Some(mtime_ms) = mtime_ms {
                put_i64(b, *mtime_ms);
            }
            put_bool(b, mode.is_some());
            if let Some(mode) = mode {
                put_u32(b, *mode);
            }
            b.push(*durability);
        }
        Frame::StageDone {
            mtime_applied,
            durable,
        } => {
            b.push(57);
            put_bool(b, *mtime_applied);
            put_bool(b, *durable);
        }
        Frame::TargetLimits(root) => {
            b.push(58);
            put_str(b, root);
        }
        Frame::Limits(limits) => {
            b.push(59);
            put_bool(b, limits.windows_names);
            b.push(limits.name_limit);
            put_u64(b, limits.name_max);
            put_bool(b, limits.max_file_size.is_some());
            if let Some(size) = limits.max_file_size {
                put_u64(b, size);
            }
            b.push(limits.precision);
        }
        Frame::Watch { root, poll_ms } => {
            b.push(60);
            put_str(b, root);
            put_u64(b, *poll_ms);
        }
        Frame::Change(change) => {
            b.push(61);
            b.push(change.kind);
            put_bool(b, change.generation.is_some());
            if let Some(generation) = change.generation {
                put_u64(b, generation);
            }
            put_strings(b, &change.paths);
            put_str(b, &change.text);
        }
        _ => return false,
    }
    true
}

/// Decode a frame of this set; `None` for an unknown tag.
pub(in super::super) fn decode(tag: u8, r: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(Some(match tag {
        45 => Frame::ListTolerant(r.string()?),
        46 => {
            let count = counted(r, super::super::MIN_WIRE_META_BYTES)?;
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                entries.push(get_meta(r)?);
            }
            let count = counted(r, MIN_OMISSION_BYTES)?;
            let mut omitted = Vec::with_capacity(count);
            for _ in 0..count {
                omitted.push(get_omission(r)?);
            }
            Frame::DirPart { entries, omitted }
        }
        47 => Frame::WalkHashed2 {
            root: r.string()?,
            algorithm: code(r, 2)?,
            min_bytes: r.u64()?,
        },
        48 => Frame::HashOmitted(get_omission(r)?),
        49 => Frame::FindDuplicates {
            root: r.string()?,
            min_bytes: r.u64()?,
        },
        50 => Frame::DupProgress(get_progress(r)?),
        51 => Frame::DupGroup(get_group(r)?),
        52 => Frame::DupSummary(get_summary(r)?),
        53 => Frame::Query {
            kind: r.u8()?,
            path: r.string()?,
        },
        54 => Frame::Answer(r.u64()?),
        55 => Frame::Recycle {
            path: r.string()?,
            size: r.u64()?,
            sha256: r.opt_str()?,
        },
        56 => Frame::FinishStage {
            stage: r.string()?,
            mtime_ms: if r.bool()? { Some(r.i64()?) } else { None },
            mode: if r.bool()? { Some(r.u32()?) } else { None },
            durability: code(r, 2)?,
        },
        57 => Frame::StageDone {
            mtime_applied: r.bool()?,
            durable: r.bool()?,
        },
        58 => Frame::TargetLimits(r.string()?),
        59 => Frame::Limits(WireTargetLimits {
            windows_names: r.bool()?,
            name_limit: code(r, 2)?,
            name_max: r.u64()?,
            max_file_size: if r.bool()? { Some(r.u64()?) } else { None },
            precision: code(r, 7)?,
        }),
        60 => Frame::Watch {
            root: r.string()?,
            poll_ms: r.u64()?,
        },
        61 => Frame::Change(WireChange {
            kind: code(r, 3)?,
            generation: if r.bool()? { Some(r.u64()?) } else { None },
            paths: get_strings(r)?,
            text: r.string()?,
        }),
        _ => return Ok(None),
    }))
}

/// A one-byte code of at most `max`.
fn code(r: &mut Reader, max: u8) -> io::Result<u8> {
    let value = r.u8()?;
    if value > max {
        return Err(bad("extension frame code out of range"));
    }
    Ok(value)
}

/// An element count no larger than the remaining bytes can hold.
fn counted(r: &mut Reader, min_record: usize) -> io::Result<usize> {
    let count = r.u32()? as usize;
    if count > r.remaining() / min_record.max(1) {
        return Err(bad("element count exceeds the remaining frame bytes"));
    }
    Ok(count)
}

fn sum(mut left: usize, right: usize) -> io::Result<usize> {
    add_len(&mut left, right)?;
    Ok(left)
}

fn put_omission(b: &mut Vec<u8>, omission: &WireOmission) {
    put_str(b, &omission.rel);
    b.push(omission.reason);
    put_str(b, &omission.detail);
}

fn get_omission(r: &mut Reader) -> io::Result<WireOmission> {
    Ok(WireOmission {
        rel: r.string()?,
        reason: code(r, 4)?,
        detail: r.string()?,
    })
}

fn omission_len(omission: &WireOmission) -> io::Result<usize> {
    sum(
        string_len(&omission.rel)?,
        string_len(&omission.detail)? + 1,
    )
}

fn dir_part_len(entries: &[WireMeta], omitted: &[WireOmission]) -> io::Result<usize> {
    u32::try_from(entries.len()).map_err(|_| bad("frame too large"))?;
    u32::try_from(omitted.len()).map_err(|_| bad("frame too large"))?;
    let mut length = 8;
    for entry in entries {
        add_len(&mut length, metadata_len(entry)?)?;
    }
    for omission in omitted {
        add_len(&mut length, omission_len(omission)?)?;
    }
    Ok(length)
}

fn put_progress(b: &mut Vec<u8>, progress: &WireReclaimProgress) {
    for value in [
        progress.files,
        progress.dirs,
        progress.bytes,
        progress.fingerprinted,
        progress.hashed,
        progress.candidates,
    ] {
        put_u64(b, value);
    }
    b.push(progress.phase);
    for value in [
        progress.files_total,
        progress.bytes_total,
        progress.bytes_done,
    ] {
        put_u64(b, value);
    }
    put_str(b, &progress.current);
}

fn get_progress(r: &mut Reader) -> io::Result<WireReclaimProgress> {
    Ok(WireReclaimProgress {
        files: r.u64()?,
        dirs: r.u64()?,
        bytes: r.u64()?,
        fingerprinted: r.u64()?,
        hashed: r.u64()?,
        candidates: r.u64()?,
        phase: code(r, 3)?,
        files_total: r.u64()?,
        bytes_total: r.u64()?,
        bytes_done: r.u64()?,
        current: r.string()?,
    })
}

fn put_group(b: &mut Vec<u8>, group: &WireDuplicateGroup) {
    put_u64(b, group.size);
    b.push(group.algorithm);
    put_str(b, &group.hex);
    b.push(group.evidence);
    put_u64(b, group.reclaimable);
    put_u32(b, group.items.len() as u32);
    for item in &group.items {
        put_str(b, &item.path);
        put_str(b, &item.name);
        put_u64(b, item.size);
        put_i64(b, item.mtime_ms);
        put_opt_str(b, &item.backend_id);
    }
}

fn get_group(r: &mut Reader) -> io::Result<WireDuplicateGroup> {
    let size = r.u64()?;
    let algorithm = code(r, 2)?;
    let hex = r.string()?;
    let evidence = code(r, 2)?;
    let reclaimable = r.u64()?;
    let count = counted(r, MIN_ITEM_BYTES)?;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        items.push(WireDuplicateItem {
            path: r.string()?,
            name: r.string()?,
            size: r.u64()?,
            mtime_ms: r.i64()?,
            backend_id: r.opt_str()?,
        });
    }
    Ok(WireDuplicateGroup {
        size,
        algorithm,
        hex,
        evidence,
        reclaimable,
        items,
    })
}

fn group_len(group: &WireDuplicateGroup) -> io::Result<usize> {
    u32::try_from(group.items.len()).map_err(|_| bad("frame too large"))?;
    let mut length = sum(string_len(&group.hex)?, 22)?;
    for item in &group.items {
        add_len(&mut length, string_len(&item.path)?)?;
        add_len(&mut length, string_len(&item.name)?)?;
        add_len(&mut length, 16)?;
        add_len(&mut length, optional_string_len(&item.backend_id)?)?;
    }
    Ok(length)
}

fn put_strings(b: &mut Vec<u8>, values: &[String]) {
    put_u32(b, values.len() as u32);
    for value in values {
        put_str(b, value);
    }
}

fn get_strings(r: &mut Reader) -> io::Result<Vec<String>> {
    let count = counted(r, MIN_STRING_BYTES)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(r.string()?);
    }
    Ok(values)
}

fn strings_len(values: &[String]) -> io::Result<usize> {
    u32::try_from(values.len()).map_err(|_| bad("frame too large"))?;
    let mut length = 4;
    for value in values {
        add_len(&mut length, string_len(value)?)?;
    }
    Ok(length)
}

fn put_summary(b: &mut Vec<u8>, summary: &WireDuplicateSummary) {
    for value in [
        summary.files,
        summary.bytes,
        summary.candidates,
        summary.compared,
        summary.groups,
    ] {
        put_u64(b, value);
    }
    put_u32(b, summary.protected.len() as u32);
    for (area, entries) in &summary.protected {
        put_str(b, area);
        put_u64(b, *entries);
    }
    put_strings(b, &summary.errors);
    put_u64(b, summary.suppressed_errors);
    put_strings(b, &summary.limits);
    put_opt_str(b, &summary.root_error);
}

fn get_summary(r: &mut Reader) -> io::Result<WireDuplicateSummary> {
    let mut summary = WireDuplicateSummary {
        files: r.u64()?,
        bytes: r.u64()?,
        candidates: r.u64()?,
        compared: r.u64()?,
        groups: r.u64()?,
        ..WireDuplicateSummary::default()
    };
    let count = counted(r, MIN_AREA_BYTES)?;
    summary.protected.reserve(count);
    for _ in 0..count {
        summary.protected.push((r.string()?, r.u64()?));
    }
    summary.errors = get_strings(r)?;
    summary.suppressed_errors = r.u64()?;
    summary.limits = get_strings(r)?;
    summary.root_error = r.opt_str()?;
    Ok(summary)
}

fn summary_len(summary: &WireDuplicateSummary) -> io::Result<usize> {
    u32::try_from(summary.protected.len()).map_err(|_| bad("frame too large"))?;
    let mut length = 44;
    for (area, _) in &summary.protected {
        add_len(&mut length, string_len(area)?)?;
        add_len(&mut length, 8)?;
    }
    add_len(&mut length, strings_len(&summary.errors)?)?;
    add_len(&mut length, 8)?;
    add_len(&mut length, strings_len(&summary.limits)?)?;
    add_len(&mut length, optional_string_len(&summary.root_error)?)?;
    Ok(length)
}

fn change_len(change: &WireChange) -> io::Result<usize> {
    let generation = if change.generation.is_some() { 9 } else { 1 };
    let mut length = 1 + generation;
    add_len(&mut length, strings_len(&change.paths)?)?;
    add_len(&mut length, string_len(&change.text)?)?;
    Ok(length)
}
