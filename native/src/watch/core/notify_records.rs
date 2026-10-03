//! Parser for the change records `ReadDirectoryChangesW` writes
//! (`FILE_NOTIFY_INFORMATION`: next-entry offset, action and name length as
//! little-endian `u32`, then the UTF-16 name relative to the watched
//! directory, without NUL). Platform-neutral so it is tested everywhere; the
//! records are read byte-wise because they are only WCHAR-aligned under some
//! file system drivers (notify 8.0 changelog).

use super::types::EventKind;

const HEADER: usize = 12;

/// `FILE_ACTION_*` codes.
const ADDED: u32 = 1;
const REMOVED: u32 = 2;
const MODIFIED: u32 = 3;
const RENAMED_OLD: u32 = 4;
const RENAMED_NEW: u32 = 5;

/// One record: what happened and the path below the watched directory with
/// `/` separators (lossy for unpaired surrogates).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) kind: EventKind,
    pub(crate) rel: String,
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// All complete records of `bytes`; a malformed tail ends the list.
pub(crate) fn parse(bytes: &[u8]) -> Vec<Record> {
    let mut records = Vec::new();
    let mut offset = 0usize;
    loop {
        let (Some(next), Some(action), Some(length)) = (
            read_u32(bytes, offset),
            read_u32(bytes, offset + 4),
            read_u32(bytes, offset + 8),
        ) else {
            break;
        };
        let start = offset + HEADER;
        let Some(name) = usize::try_from(length)
            .ok()
            .and_then(|length| bytes.get(start..start.checked_add(length)?))
        else {
            break;
        };
        let units: Vec<u16> = name
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let rel = String::from_utf16_lossy(&units).replace('\\', "/");
        let kind = match action {
            ADDED => Some(EventKind::Created),
            REMOVED => Some(EventKind::Removed),
            MODIFIED => Some(EventKind::Modified),
            RENAMED_OLD => Some(EventKind::RenamedFrom),
            RENAMED_NEW => Some(EventKind::RenamedTo),
            _ => None,
        };
        if let Some(kind) = kind {
            if !rel.is_empty() {
                records.push(Record { kind, rel });
            }
        }
        if next == 0 {
            break;
        }
        let Some(following) = usize::try_from(next)
            .ok()
            .and_then(|next| offset.checked_add(next))
        else {
            break;
        };
        offset = following;
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(next: u32, action: u32, name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&next.to_le_bytes());
        bytes.extend_from_slice(&action.to_le_bytes());
        bytes.extend_from_slice(&((units.len() * 2) as u32).to_le_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn review_task_watch_parses_change_records_in_order() {
        let mut first = record(0, RENAMED_OLD, "Projekt\\alt.txt");
        // Pad the first record to a DWORD boundary as the kernel does.
        while first.len() % 4 != 0 {
            first.push(0);
        }
        let next = first.len() as u32;
        first[0..4].copy_from_slice(&next.to_le_bytes());
        let mut bytes = first;
        bytes.extend(record(0, RENAMED_NEW, "Projekt\\neu.txt"));

        assert_eq!(
            parse(&bytes),
            vec![
                Record {
                    kind: EventKind::RenamedFrom,
                    rel: "Projekt/alt.txt".into()
                },
                Record {
                    kind: EventKind::RenamedTo,
                    rel: "Projekt/neu.txt".into()
                },
            ]
        );
    }

    #[test]
    fn review_task_watch_ignores_truncated_and_unknown_records() {
        let mut bytes = record(0, 9, "x");
        assert!(parse(&bytes).is_empty());
        bytes = record(0, ADDED, "neu.txt");
        bytes.truncate(bytes.len() - 1);
        assert!(parse(&bytes).is_empty());
        assert!(parse(&[]).is_empty());
    }
}
