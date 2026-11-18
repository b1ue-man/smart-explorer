//! Bounds-checked decoding of variable-length Windows directory records.
//!
//! A malformed record is reported together with the offset of the record
//! after it (when its header still allows that), so the caller can skip one
//! entry instead of abandoning the whole batch. Names that Win32 could never
//! open — carrying NUL or separator units — are kept with those units
//! replaced, flagged `unrepresentable`, so the entry is still counted.
use std::{ffi::OsString, io, mem::offset_of, os::windows::ffi::OsStringExt};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FULL_DIR_INFO, FILE_ID_EXTD_DIR_INFO,
};

#[derive(Clone, Copy)]
pub(super) enum Layout {
    Extended,
    Full,
}

pub(super) struct Record {
    pub name: OsString,
    pub size: u64,
    /// `AllocationSize`: bytes the file occupies on its volume.
    pub allocated: u64,
    pub mtime_ms: i64,
    pub btime_ms: i64,
    pub attributes: u32,
    /// The reparse tag of a reparse point: the extended layout's own field,
    /// the full layout's `EaSize` (which holds the tag for reparse points).
    pub tag: Option<u32>,
    /// `FileId` of the extended layout (128 bits; NTFS uses the low 64).
    pub file_id: Option<u128>,
    pub next: Option<usize>,
    /// The stored name contained NUL or separator units and was sanitized;
    /// it cannot be joined into an openable path.
    pub unrepresentable: bool,
}

pub(super) struct DecodeError {
    /// Offset of the following record when the header was readable.
    pub next: Option<usize>,
    pub error: io::Error,
}

const REPLACEMENT: u16 = 0xfffd;

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Ungültiger Windows-Verzeichniseintrag",
    )
}

fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let value = bytes.get(offset..offset + 4).ok_or_else(invalid)?;
    Ok(u32::from_le_bytes(value.try_into().map_err(|_| invalid())?))
}

/// The absolute offset of the record after the one at `offset`, when the
/// header's `NextEntryOffset` is plausible.
fn next_offset(bytes: &[u8], offset: usize) -> Option<usize> {
    let record = bytes.get(offset..)?;
    let next = u32_at(record, 0).ok()? as usize;
    if next == 0 || !next.is_multiple_of(8) || next >= record.len() {
        return None;
    }
    offset.checked_add(next)
}

pub(super) fn decode(bytes: &[u8], offset: usize, layout: Layout) -> Result<Record, DecodeError> {
    decode_inner(bytes, offset, layout).map_err(|error| DecodeError {
        next: next_offset(bytes, offset),
        error,
    })
}

fn decode_inner(bytes: &[u8], offset: usize, layout: Layout) -> io::Result<Record> {
    let bytes = bytes.get(offset..).ok_or_else(invalid)?;
    let name_offset = match layout {
        Layout::Extended => offset_of!(FILE_ID_EXTD_DIR_INFO, FileName),
        Layout::Full => offset_of!(FILE_FULL_DIR_INFO, FileName),
    };
    let next = u32_at(bytes, 0)? as usize;
    let name_len = u32_at(bytes, offset_of!(FILE_FULL_DIR_INFO, FileNameLength))? as usize;
    if name_len == 0 || !name_len.is_multiple_of(2) {
        return Err(invalid());
    }
    let end = name_offset.checked_add(name_len).ok_or_else(invalid)?;
    if next != 0 && (!next.is_multiple_of(8) || next < end || next >= bytes.len()) {
        return Err(invalid());
    }
    let mut unrepresentable = false;
    let units: Vec<u16> = bytes
        .get(name_offset..end)
        .ok_or_else(invalid)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|unit| u16::from_le_bytes(*unit))
        .map(|unit| {
            if matches!(unit, 0 | 47 | 92) {
                unrepresentable = true;
                REPLACEMENT
            } else {
                unit
            }
        })
        .collect();
    let size = i64_at(bytes, offset_of!(FILE_FULL_DIR_INFO, EndOfFile))?;
    // A negative EndOfFile is a provider bug, not a reason to lose the entry.
    let size = size.max(0) as u64;
    // Both layouts share the FILE_FULL_DIR_INFO prefix up to FileNameLength.
    let allocated = i64_at(bytes, offset_of!(FILE_FULL_DIR_INFO, AllocationSize))?.max(0) as u64;
    let attributes = u32_at(bytes, offset_of!(FILE_FULL_DIR_INFO, FileAttributes))?;
    let reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    Ok(Record {
        name: OsString::from_wide(&units),
        size,
        allocated,
        mtime_ms: filetime_ms(bytes, offset_of!(FILE_FULL_DIR_INFO, LastWriteTime))?,
        btime_ms: filetime_ms(bytes, offset_of!(FILE_FULL_DIR_INFO, CreationTime))?,
        attributes,
        tag: match layout {
            Layout::Extended => Some(u32_at(
                bytes,
                offset_of!(FILE_ID_EXTD_DIR_INFO, ReparsePointTag),
            )?),
            Layout::Full if reparse => Some(u32_at(bytes, offset_of!(FILE_FULL_DIR_INFO, EaSize))?),
            Layout::Full => None,
        },
        file_id: match layout {
            Layout::Extended => {
                let offset = offset_of!(FILE_ID_EXTD_DIR_INFO, FileId);
                Some(u128::from_le_bytes(
                    bytes
                        .get(offset..offset + 16)
                        .ok_or_else(invalid)?
                        .try_into()
                        .map_err(|_| invalid())?,
                ))
            }
            Layout::Full => None,
        },
        next: if next == 0 {
            None
        } else {
            Some(offset.checked_add(next).ok_or_else(invalid)?)
        },
        unrepresentable,
    })
}

fn i64_at(bytes: &[u8], offset: usize) -> io::Result<i64> {
    Ok(i64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ))
}

fn filetime_ms(bytes: &[u8], offset: usize) -> io::Result<i64> {
    let value = i64_at(bytes, offset)?;
    Ok(if value == 0 {
        0
    } else {
        value / 10_000 - 11_644_473_600_000
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    fn record(layout: Layout, name: &[u16]) -> Vec<u8> {
        let name_offset = match layout {
            Layout::Extended => offset_of!(FILE_ID_EXTD_DIR_INFO, FileName),
            Layout::Full => offset_of!(FILE_FULL_DIR_INFO, FileName),
        };
        let mut bytes = vec![0; (name_offset + name.len() * 2 + 7) & !7];
        bytes[40..48].copy_from_slice(&123i64.to_le_bytes());
        bytes[60..64].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
        for (index, unit) in name.iter().enumerate() {
            bytes[name_offset + index * 2..name_offset + index * 2 + 2]
                .copy_from_slice(&unit.to_le_bytes());
        }
        bytes
    }
    #[test]
    fn analytics_access_task_sdk_layout_and_native_names() {
        assert_eq!(offset_of!(FILE_ID_EXTD_DIR_INFO, FileName), 88);
        assert_eq!(offset_of!(FILE_FULL_DIR_INFO, FileName), 68);
        for layout in [Layout::Extended, Layout::Full] {
            let name = [b'a' as u16, 0xd800, b'z' as u16];
            let bytes = record(layout, &name);
            let decoded = decode(&bytes, 0, layout).ok().unwrap();
            assert_eq!(decoded.name.encode_wide().collect::<Vec<_>>(), name);
            assert_eq!(decoded.size, 123);
            assert!(decoded.next.is_none());
            assert!(!decoded.unrepresentable);
            assert_eq!(decoded.tag.is_some(), matches!(layout, Layout::Extended));
            let mut multiple = bytes.clone();
            multiple[0..4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            multiple.extend_from_slice(&bytes);
            let first = decode(&multiple, 0, layout).ok().unwrap();
            assert_eq!(
                decode(&multiple, first.next.unwrap(), layout)
                    .ok()
                    .unwrap()
                    .size,
                123
            );
        }
    }
    #[test]
    fn analytics_access_task_directory_decoder_rejects_malformed_records() {
        for layout in [Layout::Extended, Layout::Full] {
            let bytes = record(layout, &[b'a' as u16]);
            for length in [0, 1, u32::MAX] {
                let mut invalid = bytes.clone();
                invalid[60..64].copy_from_slice(&length.to_le_bytes());
                assert!(decode(&invalid, 0, layout).is_err());
            }
            for next in [1, 8, u32::MAX] {
                let mut invalid = bytes.clone();
                invalid[..4].copy_from_slice(&next.to_le_bytes());
                assert!(decode(&invalid, 0, layout).is_err());
            }
            assert!(decode(&bytes[..30], 0, layout).is_err());
            assert!(decode(&bytes, usize::MAX, layout).is_err());
            // Forbidden name units no longer discard the entry: the record
            // stays countable, its name is sanitized and flagged.
            for unit in [0, 47, 92] {
                let decoded = decode(&record(layout, &[b'x' as u16, unit]), 0, layout)
                    .ok()
                    .expect("sanitized, not rejected");
                assert!(decoded.unrepresentable);
                assert_eq!(
                    decoded.name.encode_wide().collect::<Vec<_>>(),
                    [b'x' as u16, REPLACEMENT]
                );
                assert_eq!(decoded.size, 123);
            }
            // A negative size is clamped, never a reason to drop the entry.
            let mut negative = bytes.clone();
            negative[40..48].copy_from_slice(&(-1i64).to_le_bytes());
            assert_eq!(decode(&negative, 0, layout).ok().unwrap().size, 0);
            let mut negative = bytes.clone();
            negative[48..56].copy_from_slice(&(-1i64).to_le_bytes());
            assert_eq!(decode(&negative, 0, layout).ok().unwrap().allocated, 0);
            // A malformed record inside a batch still tells the caller where
            // the next record starts, so only that entry is skipped.
            let mut batch = bytes.clone();
            batch[0..4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            batch.extend_from_slice(&bytes);
            batch[60..64].copy_from_slice(&1u32.to_le_bytes());
            let error = decode(&batch, 0, layout).err().unwrap();
            assert_eq!(error.next, Some(bytes.len()));
        }
    }
    /// A cloud placeholder or compressed file: AllocationSize is read from
    /// the shared record prefix of both layouts, apart from EndOfFile.
    #[test]
    fn belegt_task_windows_records_carry_allocation_size() {
        assert_eq!(offset_of!(FILE_FULL_DIR_INFO, AllocationSize), 48);
        assert_eq!(offset_of!(FILE_ID_EXTD_DIR_INFO, AllocationSize), 48);
        for layout in [Layout::Extended, Layout::Full] {
            let mut bytes = record(layout, &[b'p' as u16]);
            bytes[40..48].copy_from_slice(&(5_000_000_000i64).to_le_bytes());
            bytes[48..56].copy_from_slice(&(4096i64).to_le_bytes());
            let decoded = decode(&bytes, 0, layout).ok().expect("valid record");
            assert_eq!(decoded.size, 5_000_000_000);
            assert_eq!(decoded.allocated, 4096);
        }
    }

    /// Hard links share the extended layout's FileId; the full layout reads
    /// a reparse point's tag from `EaSize` instead of opening the entry.
    #[test]
    fn belegt_task_windows_records_carry_file_id_and_full_layout_tag() {
        assert_eq!(offset_of!(FILE_ID_EXTD_DIR_INFO, FileId), 72);
        assert_eq!(offset_of!(FILE_FULL_DIR_INFO, EaSize), 64);
        let mut extended = record(Layout::Extended, &[b'x' as u16]);
        extended[72..88].copy_from_slice(&0x0001_0000_0000_002au128.to_le_bytes());
        let decoded = decode(&extended, 0, Layout::Extended).ok().expect("record");
        assert_eq!(decoded.file_id, Some(0x0001_0000_0000_002a));
        let mut full = record(Layout::Full, &[b'w' as u16]);
        full[56..60].copy_from_slice(&FILE_ATTRIBUTE_REPARSE_POINT.to_le_bytes());
        full[64..68].copy_from_slice(&0x8000_0017u32.to_le_bytes());
        let decoded = decode(&full, 0, Layout::Full).ok().expect("record");
        assert_eq!(decoded.tag, Some(0x8000_0017));
        assert_eq!(decoded.file_id, None);
        let plain = decode(&record(Layout::Full, &[b'p' as u16]), 0, Layout::Full)
            .ok()
            .expect("record");
        assert_eq!(plain.tag, None);
    }
}
