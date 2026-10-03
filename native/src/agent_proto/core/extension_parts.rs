//! Bounded extension replies. A duplicate group can occupy consecutive
//! `DupGroup` parts with the same digest, size and evidence; receivers merge
//! those parts before presenting the report. No extra wire fields are needed.
use std::io;

use super::{Frame, WireDuplicateGroup, WireMeta, WireOmission};

const PART_BYTES: usize = 1024 * 1024;

pub(crate) fn emit_listing_parts(
    entries: impl IntoIterator<Item = WireMeta>,
    omissions: impl IntoIterator<Item = WireOmission>,
    mut emit: impl FnMut(Frame) -> io::Result<()>,
) -> io::Result<()> {
    let mut part_entries = Vec::new();
    let mut part_omissions = Vec::new();
    let mut bytes = 17usize;
    for entry in entries {
        let size = 23usize.saturating_add(entry.name.len()).saturating_add(
            entry.content_md5.as_ref().map_or(0, |hash| 4usize.saturating_add(hash.len())));
        check_item_size(size, 17)?;
        if bytes.saturating_add(size) > PART_BYTES && !part_entries.is_empty() {
            emit(Frame::DirPart { entries: std::mem::take(&mut part_entries), omitted: Vec::new() })?;
            bytes = 17;
        }
        bytes = bytes.saturating_add(size);
        part_entries.push(entry);
    }
    for omitted in omissions {
        let size = 9usize.saturating_add(omitted.rel.len()).saturating_add(omitted.detail.len());
        check_item_size(size, 17)?;
        if bytes.saturating_add(size) > PART_BYTES && (!part_entries.is_empty() || !part_omissions.is_empty()) {
            emit(Frame::DirPart { entries: std::mem::take(&mut part_entries),
                omitted: std::mem::take(&mut part_omissions) })?;
            bytes = 17;
        }
        bytes = bytes.saturating_add(size);
        part_omissions.push(omitted);
    }
    if !part_entries.is_empty() || !part_omissions.is_empty() {
        emit(Frame::DirPart { entries: part_entries, omitted: part_omissions })?;
    }
    Ok(())
}

pub(crate) fn emit_duplicate_parts(
    mut group: WireDuplicateGroup,
    mut emit: impl FnMut(Frame) -> io::Result<()>,
) -> io::Result<()> {
    let items = std::mem::take(&mut group.items);
    let header_bytes = 35usize.saturating_add(group.hex.len());
    let mut bytes = header_bytes;
    for item in items {
        let size = 25usize.saturating_add(item.path.len()).saturating_add(item.name.len())
            .saturating_add(item.backend_id.as_ref().map_or(0, |id| 4usize.saturating_add(id.len())));
        check_item_size(size, header_bytes)?;
        if bytes.saturating_add(size) > PART_BYTES && !group.items.is_empty() {
            let items = std::mem::take(&mut group.items);
            let mut part = group.clone();
            part.items = items;
            part.reclaimable = part.size.saturating_mul(part.items.len().saturating_sub(1) as u64);
            emit(Frame::DupGroup(part))?;
            bytes = header_bytes;
        }
        group.items.push(item);
        bytes = bytes.saturating_add(size);
    }
    if !group.items.is_empty() {
        group.reclaimable = group.size.saturating_mul(group.items.len().saturating_sub(1) as u64);
        emit(Frame::DupGroup(group))?;
    }
    Ok(())
}

fn check_item_size(size: usize, header: usize) -> io::Result<()> {
    if size.saturating_add(header) > PART_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "einzelner Erweiterungseintrag ist zu groß"));
    }
    Ok(())
}

pub(crate) fn append_duplicate_part(groups: &mut Vec<WireDuplicateGroup>, part: WireDuplicateGroup) {
    if let Some(last) = groups.last_mut().filter(|last| last.size == part.size
        && last.algorithm == part.algorithm && last.hex == part.hex && last.evidence == part.evidence) {
        last.items.extend(part.items);
        last.reclaimable = last.size.saturating_mul(last.items.len().saturating_sub(1) as u64);
    } else {
        groups.push(part);
    }
}

#[cfg(test)]
#[path = "extension_parts_task_tests.rs"]
mod tests;
