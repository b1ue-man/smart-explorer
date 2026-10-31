//! The file of one sync side's folder keys (`folder_key_cache.rs`): magic,
//! record count, then per record its key, its items (tagged as in
//! `Part`) and its walked subfolders with their keys, all big-endian with
//! length-prefixed texts. Every record is checked against its key when it
//! is read.
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::folder_key_cache::{invalid, FolderCache, Part, Record};
use crate::agent_proto::{EntryKind, FolderKey};
use crate::vfs::OmissionReason;

const MAGIC: &[u8] = b"SEFK\x01";
/// Records a side file may hold (one per distinct folder key).
const MAX_RECORDS: u32 = 4_000_000;
/// Items of one folder (as the sync's walk budget for one folder).
const MAX_PARTS: u32 = 10_000_000;
const MAX_TEXT: u32 = 64 * 1024;

/// The file of one side: its state identity, root and digest algorithm.
pub(super) fn side_path(identity: &str, root: &str, algorithm: Option<&str>) -> PathBuf {
    let side = super::persistence::pair_id_parts([
        (identity, root),
        ("folder-keys-v1", algorithm.unwrap_or("none")),
    ]);
    crate::support_dirs::sync_data_dir()
        .join("folder-keys")
        .join(format!("{side}.sefk"))
}

/// The stored folders of `path`; an absent or unreadable file is empty.
pub(super) fn load(path: &Path) -> FolderCache {
    let Ok(bytes) = std::fs::read(path) else {
        return FolderCache::default();
    };
    parse(&bytes).unwrap_or_default()
}

fn parse(bytes: &[u8]) -> io::Result<FolderCache> {
    let mut input = bytes;
    if take(&mut input, MAGIC.len())? != MAGIC {
        return Err(invalid("no folder key file"));
    }
    let count = read_u32(&mut input)?.min(MAX_RECORDS);
    let mut cache = FolderCache::default();
    for _ in 0..count {
        let key: FolderKey = take(&mut input, 32)?
            .try_into()
            .map_err(|_| invalid("folder key"))?;
        let record = read_record(&mut input)?;
        // A damaged record never stands for its key.
        if record.key() == key {
            cache.insert(key, record);
        }
    }
    Ok(cache)
}

fn read_record(input: &mut &[u8]) -> io::Result<Record> {
    let parts = read_u32(input)?;
    if parts > MAX_PARTS {
        return Err(invalid("folder record too large"));
    }
    let mut record = Record::default();
    for _ in 0..parts {
        let tag = take(input, 1)?[0];
        record.parts.push(match tag {
            1..=4 => {
                let kind = match tag {
                    1 => EntryKind::File,
                    2 => EntryKind::Dir,
                    3 => EntryKind::Link,
                    _ => EntryKind::Special,
                };
                let name = read_str(input)?;
                let size = u64::from_be_bytes(array(input)?);
                let mtime_ms = i64::from_be_bytes(array(input)?);
                let digest = match take(input, 1)?[0] {
                    0 => None,
                    _ => Some(read_str(input)?),
                };
                let hidden = take(input, 1)?[0] != 0;
                Part::Entry {
                    name,
                    kind,
                    size,
                    mtime_ms,
                    digest,
                    hidden,
                }
            }
            5 => Part::Omitted {
                name: read_str(input)?,
                reason: reason(take(input, 1)?[0]),
                detail: read_str(input)?,
            },
            6 => Part::Failed {
                reason: reason(take(input, 1)?[0]),
                detail: read_str(input)?,
            },
            _ => return Err(invalid("unknown folder record item")),
        });
    }
    let children = read_u32(input)?;
    if children > MAX_PARTS {
        return Err(invalid("folder record too large"));
    }
    for _ in 0..children {
        let name = read_str(input)?;
        record.children.push((name, array(input)?));
    }
    Ok(record)
}

fn reason(code: u8) -> OmissionReason {
    match code {
        0 => OmissionReason::Link,
        1 => OmissionReason::Special,
        3 => OmissionReason::Vanished,
        4 => OmissionReason::Unrepresentable,
        _ => OmissionReason::Unreadable,
    }
}

fn take<'a>(input: &mut &'a [u8], count: usize) -> io::Result<&'a [u8]> {
    if input.len() < count {
        return Err(invalid("folder key file truncated"));
    }
    let (head, rest) = input.split_at(count);
    *input = rest;
    Ok(head)
}

fn array<const N: usize>(input: &mut &[u8]) -> io::Result<[u8; N]> {
    take(input, N)?
        .try_into()
        .map_err(|_| invalid("folder key file field"))
}

fn read_u32(input: &mut &[u8]) -> io::Result<u32> {
    Ok(u32::from_be_bytes(array(input)?))
}

fn read_str(input: &mut &[u8]) -> io::Result<String> {
    let length = read_u32(input)?;
    if length > MAX_TEXT {
        return Err(invalid("folder key file text too long"));
    }
    String::from_utf8(take(input, length as usize)?.to_vec())
        .map_err(|_| invalid("folder key file text"))
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    let text = &text.as_bytes()[..text.len().min(MAX_TEXT as usize)];
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text);
}

fn encode(cache: &FolderCache) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(cache.len() as u32).to_be_bytes());
    for (key, record) in cache.records() {
        out.extend_from_slice(key);
        out.extend_from_slice(&(record.parts.len() as u32).to_be_bytes());
        for part in &record.parts {
            match part {
                Part::Entry {
                    name,
                    kind,
                    size,
                    mtime_ms,
                    digest,
                    hidden,
                } => {
                    out.push(match kind {
                        EntryKind::File => 1,
                        EntryKind::Dir => 2,
                        EntryKind::Link => 3,
                        EntryKind::Special => 4,
                    });
                    put_str(&mut out, name);
                    out.extend_from_slice(&size.to_be_bytes());
                    out.extend_from_slice(&mtime_ms.to_be_bytes());
                    match digest {
                        Some(digest) => {
                            out.push(1);
                            put_str(&mut out, digest);
                        }
                        None => out.push(0),
                    }
                    out.push(u8::from(*hidden));
                }
                Part::Omitted {
                    name,
                    reason,
                    detail,
                } => {
                    out.push(5);
                    put_str(&mut out, name);
                    out.push(reason.code());
                    put_str(&mut out, detail);
                }
                Part::Failed { reason, detail } => {
                    out.push(6);
                    out.push(reason.code());
                    put_str(&mut out, detail);
                }
            }
        }
        out.extend_from_slice(&(record.children.len() as u32).to_be_bytes());
        for (name, child) in &record.children {
            put_str(&mut out, name);
            out.extend_from_slice(child);
        }
    }
    out
}

/// Replaces the file of a side with `cache` (written aside, then renamed).
pub(super) fn save(path: &Path, cache: &FolderCache) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("folder key file without folder"))?;
    std::fs::create_dir_all(parent)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staged = path.with_extension(format!("sefk-{}-{unique}.tmp", std::process::id()));
    let written = std::fs::File::create(&staged).and_then(|mut file| {
        file.write_all(&encode(cache))?;
        file.sync_all()
    });
    if let Err(error) = written.and_then(|()| std::fs::rename(&staged, path)) {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    Ok(())
}

/// Reads a whole side file through `reader` (tests of damaged files).
#[cfg(test)]
pub(super) fn parse_from(mut reader: impl std::io::Read) -> io::Result<FolderCache> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    parse(&bytes)
}

#[cfg(test)]
pub(super) fn encode_for_test(cache: &FolderCache) -> Vec<u8> {
    encode(cache)
}
