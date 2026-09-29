//! Bounds of one batch request and the header arithmetic both ends share:
//! clients split by it, servers enforce it.
use std::io;
use std::ops::Range;

use super::types::{BatchEntry, BatchItem, CHUNK};

/// Files per batch: the engine's upper bound for one batch (research §3.1:
/// a batch aims at about 250 ms of transfer and stays below 256 files).
pub const BATCH_MAX_FILES: usize = 256;
/// Bytes per batch, the same research bound (16 MiB at the fastest rate).
pub const BATCH_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Largest encoded batch header. One data chunk: a header then delays the
/// control lane no longer than one data frame does, so batches are split
/// by header size, not only by file count.
pub const BATCH_HEADER_MAX: usize = CHUNK;
/// Error marker of a batch whose outcome is unknown (the service's peer
/// failed mid-batch): the client reports the whole batch as ambiguous and
/// never counts its files as failed.
pub const BATCH_UNKNOWN_MARKER: &str = "SE_BATCH_UNKNOWN_V1";
/// Request id, tag and element count of a batch header.
const HEADER_BASE: usize = 13;
/// Longest error text in a batch item frame (`ItemEnd`, `ItemFailed`): a
/// reason with one path fits. Item frames carry no credit, so their size
/// bound keeps every receiver queue bounded in bytes; senders clip longer
/// texts with `clip_text`.
pub const ITEM_TEXT_MAX: usize = 4 * 1024;
/// Longest path in `ItemPublished`: the longest NT path (32 767 UTF-16
/// units) takes at most 98 301 UTF-8 bytes, so every real path fits.
pub const ITEM_PATH_MAX: usize = 128 * 1024;
/// Room a numbered name adds to a requested path (" (1000)" takes 7 bytes).
const NUMBERED_SUFFIX_MAX: usize = 16;

/// `text` shortened to `ITEM_TEXT_MAX` bytes on a character boundary.
pub fn clip_text(mut text: String) -> String {
    const ELLIPSIS: char = '…';
    if text.len() > ITEM_TEXT_MAX {
        let mut end = ITEM_TEXT_MAX - ELLIPSIS.len_utf8();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push(ELLIPSIS);
    }
    text
}

/// Encoded header bytes of one upload entry (path, size, nonce).
pub fn put_entry_len(entry: &BatchEntry) -> usize {
    entry.path.len().saturating_add(20)
}

/// Encoded header bytes of one download item (path, optional id, size).
pub fn get_item_len(item: &BatchItem) -> usize {
    let id = item.id.as_ref().map_or(0, |id| id.len().saturating_add(4));
    item.path.len().saturating_add(13).saturating_add(id)
}

/// Consecutive ranges whose header, file count and byte sum stay within the
/// bounds. An element that alone exceeds a bound travels alone (the server
/// then refuses it with a clear error instead of the client dropping it).
pub fn split_batch(header: &[usize], sizes: &[u64]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut header_bytes = HEADER_BASE;
    let mut data_bytes = 0u64;
    for (index, (&entry_header, &size)) in header.iter().zip(sizes).enumerate() {
        let next_header = header_bytes.saturating_add(entry_header);
        let next_data = data_bytes.saturating_add(size);
        let full = index - start >= BATCH_MAX_FILES
            || next_header > BATCH_HEADER_MAX
            || next_data > BATCH_MAX_BYTES;
        if full && index > start {
            ranges.push(start..index);
            start = index;
            header_bytes = HEADER_BASE.saturating_add(entry_header);
            data_bytes = size;
        } else {
            header_bytes = next_header;
            data_bytes = next_data;
        }
    }
    if start < header.len().min(sizes.len()) {
        ranges.push(start..header.len().min(sizes.len()));
    }
    ranges
}

fn check(count: usize, header: usize, bytes: Option<u64>) -> io::Result<()> {
    let bytes_fit = matches!(bytes, Some(bytes) if bytes <= BATCH_MAX_BYTES);
    if count > BATCH_MAX_FILES || header > BATCH_HEADER_MAX || !bytes_fit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Paket überschreitet die Grenzen ({count} Dateien, Kopf {header} Bytes; \
                 erlaubt {BATCH_MAX_FILES} Dateien, {BATCH_MAX_BYTES} Bytes, \
                 Kopf {BATCH_HEADER_MAX} Bytes)"
            ),
        ));
    }
    Ok(())
}

/// Server side: refuse a batch upload beyond the bounds. Every published
/// path (a requested one, maybe numbered) must fit its outcome frame, so a
/// longer path is refused before anything is written.
pub fn check_put_batch(entries: &[BatchEntry]) -> io::Result<()> {
    let header = entries.iter().fold(HEADER_BASE, |total, entry| {
        total.saturating_add(put_entry_len(entry))
    });
    let bytes = entries
        .iter()
        .try_fold(0u64, |total, entry| total.checked_add(entry.size));
    check(entries.len(), header, bytes)?;
    if entries
        .iter()
        .any(|entry| entry.path.len() > ITEM_PATH_MAX - NUMBERED_SUFFIX_MAX)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Paket-Zielpfad ist zu lang",
        ));
    }
    Ok(())
}

/// Server side: refuse a batch download beyond the bounds.
pub fn check_get_batch(items: &[BatchItem]) -> io::Result<()> {
    let header = items.iter().fold(HEADER_BASE, |total, item| {
        total.saturating_add(get_item_len(item))
    });
    let bytes = items
        .iter()
        .try_fold(0u64, |total, item| total.checked_add(item.size));
    check(items.len(), header, bytes)
}

/// The numbered variant of `name` (`name (2).ext`), identical to
/// `vfs::remote_util::numbered_remote_name`; kept here because the agent
/// binary builds this module without the application.
pub fn numbered_name(name: &str, index: usize) -> String {
    if index <= 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({index}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({index})"),
    }
}
