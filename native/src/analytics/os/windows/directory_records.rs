//! Bounds-checked decoding of variable-length Windows directory records.
use std::{ffi::OsString, io, mem::offset_of, os::windows::ffi::OsStringExt};
use windows_sys::Win32::Storage::FileSystem::{FILE_FULL_DIR_INFO, FILE_ID_EXTD_DIR_INFO};

#[derive(Clone, Copy)]
pub(super) enum Layout {
    Extended,
    Full,
}

pub(super) struct Record {
    pub name: OsString,
    pub size: u64,
    pub attributes: u32,
    pub tag: Option<u32>,
    pub next: Option<usize>,
}

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

pub(super) fn decode(bytes: &[u8], offset: usize, layout: Layout) -> io::Result<Record> {
    let bytes = bytes.get(offset..).ok_or_else(invalid)?;
    let name_offset = match layout {
        Layout::Extended => offset_of!(FILE_ID_EXTD_DIR_INFO, FileName),
        Layout::Full => offset_of!(FILE_FULL_DIR_INFO, FileName),
    };
    let next = u32_at(bytes, 0)? as usize;
    let name_len = u32_at(bytes, offset_of!(FILE_FULL_DIR_INFO, FileNameLength))? as usize;
    if name_len == 0 || name_len % 2 != 0 {
        return Err(invalid());
    }
    let end = name_offset.checked_add(name_len).ok_or_else(invalid)?;
    if next != 0 && (next % 8 != 0 || next < end || next >= bytes.len()) {
        return Err(invalid());
    }
    let units: Vec<u16> = bytes
        .get(name_offset..end)
        .ok_or_else(invalid)?
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect();
    if units.iter().any(|v| matches!(*v, 0 | 47 | 92)) {
        return Err(invalid());
    }
    let size_offset = offset_of!(FILE_FULL_DIR_INFO, EndOfFile);
    let size = i64::from_le_bytes(
        bytes
            .get(size_offset..size_offset + 8)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    );
    if size < 0 {
        return Err(invalid());
    }
    Ok(Record {
        name: OsString::from_wide(&units),
        size: size as u64,
        attributes: u32_at(bytes, offset_of!(FILE_FULL_DIR_INFO, FileAttributes))?,
        tag: match layout {
            Layout::Extended => Some(u32_at(
                bytes,
                offset_of!(FILE_ID_EXTD_DIR_INFO, ReparsePointTag),
            )?),
            Layout::Full => None,
        },
        next: if next == 0 {
            None
        } else {
            Some(offset.checked_add(next).ok_or_else(invalid)?)
        },
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
            let decoded = decode(&bytes, 0, layout).unwrap();
            assert_eq!(decoded.name.encode_wide().collect::<Vec<_>>(), name);
            assert_eq!(decoded.size, 123);
            assert!(decoded.next.is_none());
            assert_eq!(decoded.tag.is_some(), matches!(layout, Layout::Extended));
            let mut multiple = bytes.clone();
            multiple[0..4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            multiple.extend_from_slice(&bytes);
            let first = decode(&multiple, 0, layout).unwrap();
            assert_eq!(
                decode(&multiple, first.next.unwrap(), layout).unwrap().size,
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
            for unit in [0, 47, 92] {
                assert!(decode(&record(layout, &[unit]), 0, layout).is_err());
            }
            let mut negative = bytes;
            negative[40..48].copy_from_slice(&(-1i64).to_le_bytes());
            assert!(decode(&negative, 0, layout).is_err());
        }
    }
}
